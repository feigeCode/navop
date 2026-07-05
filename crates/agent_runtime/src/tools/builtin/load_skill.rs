//! 内置 Skill 加载工具。

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
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct LoadSkillTool;

#[derive(Deserialize)]
struct LoadSkillArgs {
    name: Option<String>,
    path: Option<PathBuf>,
}

#[async_trait]
impl Tool for LoadSkillTool {
    fn name(&self) -> ToolName {
        ToolName::new("load_skill")
    }

    fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
        ToolSpec::new(
            "load_skill",
            "读取当前 Skill 目录中某个 SKILL.md 的完整说明。只能按目录中已有的 name 或 path 精确加载,不能读取任意文件。",
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
                    }
                }
            }),
        )
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        let args: LoadSkillArgs = invocation.parse_arguments()?;
        let skill = resolve_skill(&invocation.skills.catalog(), &args)?;
        let contents = fs::read_to_string(&skill.path)
            .map_err(|err| ToolError::Execution(format!("读取 Skill 失败: {err}")))?;
        let body = format_skill(&skill, &contents);
        Ok(ToolObservation::success(
            invocation.call_id,
            invocation.tool_name,
            format!("已加载 Skill `{}`", skill.name),
            ObservationData::Text(body),
        ))
    }
}

fn resolve_skill(
    catalog: &[SkillSummary],
    args: &LoadSkillArgs,
) -> Result<SkillSummary, ToolError> {
    if let Some(path) = args.path.as_deref() {
        return find_by_path(catalog, path);
    }
    let Some(name) = args
        .name
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
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

fn format_skill(skill: &SkillSummary, contents: &str) -> String {
    format!(
        "Skill: {}\nDescription: {}\nPath: {}\nInstructions:\n{}",
        skill.name,
        skill.description,
        skill.path.display(),
        contents.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{SessionId, ToolCallId, TurnId};
    use crate::skill::SkillContext;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn loads_skill_from_current_catalog_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ops/SKILL.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# Ops\n\nFollow the checklist.").unwrap();
        let invocation = invocation_with_skills(
            json!({"name": "ops"}),
            SkillContext::new().with_available_skill(SkillSummary::new(
                "ops",
                "Run operational playbooks",
                path,
            )),
        );

        let observation = LoadSkillTool.execute(invocation).await.unwrap();

        assert!(observation.success);
        assert!(observation.data.to_text().contains("Follow the checklist."));
    }

    #[tokio::test]
    async fn refuses_paths_outside_current_catalog() {
        let invocation = invocation_with_skills(
            json!({"path": "/tmp/not-in-catalog/SKILL.md"}),
            SkillContext::new(),
        );

        let error = LoadSkillTool.execute(invocation).await.unwrap_err();

        assert!(matches!(error, ToolError::InvalidArguments(_)));
    }

    fn invocation_with_skills(
        arguments: serde_json::Value,
        skills: SkillContext,
    ) -> ToolInvocation {
        ToolInvocation {
            session_id: SessionId::new(),
            turn_id: TurnId::new(),
            call_id: ToolCallId::new(),
            tool_name: ToolName::new("load_skill"),
            arguments,
            resource_id: None,
            resources: ResourceContext::new(),
            skills,
            cancellation: CancellationToken::new(),
        }
    }
}
