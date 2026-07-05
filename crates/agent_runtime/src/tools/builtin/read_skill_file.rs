//! 内置 Skill 关联文件读取工具。

use crate::error::ToolError;
use crate::resource::ResourceContext;
use crate::skill::SkillSummary;
use crate::tools::invocation::ToolInvocation;
use crate::tools::observation::{ObservationData, ToolObservation};
use crate::tools::registry::Tool;
use crate::tools::spec::{ToolName, ToolSpec};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Default)]
pub struct ReadSkillFileTool;

#[derive(Deserialize)]
struct ReadSkillFileArgs {
    name: Option<String>,
    path: Option<PathBuf>,
    file_path: PathBuf,
}

#[async_trait]
impl Tool for ReadSkillFileTool {
    fn name(&self) -> ToolName {
        ToolName::new("read_skill_file")
    }

    fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
        ToolSpec::new(
            "read_skill_file",
            "读取当前 Skill 目录中 SKILL.md 引用的相对文本文件。只能读取该 Skill 目录内部文件,不能读取任意路径。",
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Skill 名称,必须与 Skill catalog 中的 name 完全一致"
                    },
                    "path": {
                        "type": "string",
                        "description": "Skill catalog 中列出的 SKILL.md 完整路径"
                    },
                    "file_path": {
                        "type": "string",
                        "description": "相对 Skill 目录的文件路径,例如 references/guide.md"
                    }
                },
                "required": ["file_path"]
            }),
        )
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        let args: ReadSkillFileArgs = invocation.parse_arguments()?;
        let skill = resolve_skill(
            &invocation.skills.catalog(),
            args.name.as_deref(),
            args.path.as_deref(),
        )?;
        let target = resolve_skill_file_path(&skill, &args.file_path)?;
        let contents = fs::read_to_string(&target)
            .map_err(|err| ToolError::Execution(format!("读取 Skill 文件失败: {err}")))?;
        Ok(ToolObservation::success(
            invocation.call_id,
            invocation.tool_name,
            format!("已读取 Skill 文件 `{}`", args.file_path.display()),
            ObservationData::Text(contents),
        ))
    }
}

fn resolve_skill(
    catalog: &[SkillSummary],
    name: Option<&str>,
    path: Option<&Path>,
) -> Result<SkillSummary, ToolError> {
    if let Some(path) = path {
        return find_by_path(catalog, path);
    }
    let Some(name) = name.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(ToolError::InvalidArguments(
            "必须提供 name 或 path".to_string(),
        ));
    };
    find_by_name(catalog, name)
}

fn find_by_path(catalog: &[SkillSummary], path: &Path) -> Result<SkillSummary, ToolError> {
    catalog
        .iter()
        .find(|skill| skill.path == path)
        .cloned()
        .ok_or_else(|| {
            ToolError::InvalidArguments(format!("Skill path 不在当前目录中: {}", path.display()))
        })
}

fn find_by_name(catalog: &[SkillSummary], name: &str) -> Result<SkillSummary, ToolError> {
    let matches = catalog
        .iter()
        .filter(|skill| skill.name == name)
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [skill] => Ok(skill.clone()),
        [] => Err(ToolError::InvalidArguments(format!(
            "Skill name 不在当前目录中: {name}"
        ))),
        _ => Err(ToolError::InvalidArguments(format!(
            "Skill name `{name}` 不唯一,请改用 path"
        ))),
    }
}

fn resolve_skill_file_path(skill: &SkillSummary, file_path: &Path) -> Result<PathBuf, ToolError> {
    if file_path.is_absolute() {
        return Err(ToolError::InvalidArguments(
            "file_path 必须是相对路径".to_string(),
        ));
    }
    if file_path.components().any(is_forbidden_component) {
        return Err(ToolError::InvalidArguments(
            "file_path 不能包含 .. 或根路径前缀".to_string(),
        ));
    }
    let skill_dir = skill.path.parent().ok_or_else(|| {
        ToolError::InvalidArguments(format!("Skill path 无父目录: {}", skill.path.display()))
    })?;
    let base = skill_dir
        .canonicalize()
        .map_err(|err| ToolError::Execution(format!("解析 Skill 目录失败: {err}")))?;
    let target = base.join(file_path);
    let canonical = target
        .canonicalize()
        .map_err(|err| ToolError::Execution(format!("解析 Skill 文件失败: {err}")))?;
    if !canonical.starts_with(&base) {
        return Err(ToolError::InvalidArguments(
            "file_path 不能指向 Skill 目录外部".to_string(),
        ));
    }
    Ok(canonical)
}

fn is_forbidden_component(component: Component<'_>) -> bool {
    matches!(
        component,
        Component::ParentDir | Component::RootDir | Component::Prefix(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{SessionId, ToolCallId, TurnId};
    use crate::skill::SkillContext;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn reads_relative_file_from_skill_directory() {
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("ops/SKILL.md");
        let ref_path = dir.path().join("ops/references/guide.md");
        std::fs::create_dir_all(ref_path.parent().unwrap()).unwrap();
        std::fs::write(&skill_path, "# Ops").unwrap();
        std::fs::write(&ref_path, "Use this guide.").unwrap();
        let invocation = invocation_with_skills(
            json!({"name": "ops", "file_path": "references/guide.md"}),
            SkillContext::new().with_available_skill(SkillSummary::new(
                "ops",
                "Run operational playbooks",
                skill_path,
            )),
        );

        let observation = ReadSkillFileTool.execute(invocation).await.unwrap();

        assert!(observation.success);
        assert_eq!("Use this guide.", observation.data.to_text());
    }

    #[tokio::test]
    async fn refuses_absolute_or_escaping_paths() {
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("ops/SKILL.md");
        std::fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
        std::fs::write(&skill_path, "# Ops").unwrap();
        let skills = SkillContext::new().with_available_skill(SkillSummary::new(
            "ops",
            "Run operational playbooks",
            skill_path,
        ));

        let absolute = invocation_with_skills(
            json!({"name": "ops", "file_path": "/tmp/nope.md"}),
            skills.clone(),
        );
        let escaping =
            invocation_with_skills(json!({"name": "ops", "file_path": "../outside.md"}), skills);

        assert!(matches!(
            ReadSkillFileTool.execute(absolute).await.unwrap_err(),
            ToolError::InvalidArguments(_)
        ));
        assert!(matches!(
            ReadSkillFileTool.execute(escaping).await.unwrap_err(),
            ToolError::InvalidArguments(_)
        ));
    }

    fn invocation_with_skills(
        arguments: serde_json::Value,
        skills: SkillContext,
    ) -> ToolInvocation {
        ToolInvocation {
            session_id: SessionId::new(),
            turn_id: TurnId::new(),
            call_id: ToolCallId::new(),
            tool_name: ToolName::new("read_skill_file"),
            arguments,
            resource_id: None,
            resources: ResourceContext::new(),
            skills,
            cancellation: CancellationToken::new(),
        }
    }
}
