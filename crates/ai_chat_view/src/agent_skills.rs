use agent_runtime::{
    SkillCatalog, SkillContext, SkillImportError, SkillMetadata, SkillRef, SkillSummary,
    import_skill_dir,
};
use gpui::SharedString;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::input::{ComposerSkillItem, ComposerSkillSummary};

#[derive(Clone, Debug)]
pub(crate) struct AgentSkillState {
    roots: Vec<PathBuf>,
    catalog: SkillCatalog,
    selected_paths: HashSet<PathBuf>,
}

impl AgentSkillState {
    /// 以工作区根为项目级 skill 来源加载。
    ///
    /// 项目内 `.codex/skills` / `.agents/skills` 优先，用户主目录的全局 skills 保留；
    /// 之前的实现只扫进程 cwd——工作台切到 worktree 后项目 skills 全部失效。
    pub(crate) fn load_for_workspace(workspace_root: &Path) -> Self {
        let mut roots = Vec::new();
        roots.push(workspace_root.join(".codex").join("skills"));
        roots.push(workspace_root.join(".agents").join("skills"));
        if let Some(home) = home_dir() {
            roots.push(home.join(".codex").join("skills"));
            roots.push(home.join(".agents").join("skills"));
        }
        Self::load(roots)
    }

    /// 工作区切换后重载项目级 skills；保留仍然可用的选择。
    pub(crate) fn reload_for_workspace(&mut self, workspace_root: &Path) {
        let mut roots = Vec::new();
        roots.push(workspace_root.join(".codex").join("skills"));
        roots.push(workspace_root.join(".agents").join("skills"));
        if let Some(home) = home_dir() {
            roots.push(home.join(".codex").join("skills"));
            roots.push(home.join(".agents").join("skills"));
        }
        self.roots = roots;
        self.reload();
    }

    fn load(roots: Vec<PathBuf>) -> Self {
        let catalog = SkillCatalog::load_from_roots(roots.clone());
        Self {
            roots,
            catalog,
            selected_paths: HashSet::new(),
        }
    }

    pub(crate) fn import_skill(&mut self, source: &Path) -> Result<(), SkillImportError> {
        let dest_root = default_import_root();
        let imported = import_skill_dir(source, &dest_root)?;
        if !self.roots.contains(&dest_root) {
            self.roots.push(dest_root);
        }
        self.reload();
        self.selected_paths.insert(imported.path);
        Ok(())
    }

    pub(crate) fn toggle(&mut self, id: &str) -> bool {
        let Some(skill) = self.skill_by_id(id) else {
            return false;
        };
        let path = skill.path.clone();
        if !self.selected_paths.remove(&path) {
            self.selected_paths.insert(path);
        }
        true
    }

    pub(crate) fn summary(&self) -> ComposerSkillSummary {
        ComposerSkillSummary::new(self.catalog.skills.len(), self.selected_paths.len())
    }

    pub(crate) fn items(&self) -> Vec<ComposerSkillItem> {
        self.catalog
            .skills
            .iter()
            .map(|skill| {
                ComposerSkillItem::new(
                    skill_id(skill),
                    skill.name.clone(),
                    skill.description.clone(),
                    skill.path.display().to_string(),
                    true,
                    self.selected_paths.contains(&skill.path),
                )
            })
            .collect()
    }

    pub(crate) fn selected_context(&self) -> SkillContext {
        let mut context = SkillContext::new();
        for skill in &self.catalog.skills {
            context = context.with_available_skill(SkillSummary::new(
                skill.name.clone(),
                skill.description.clone(),
                skill.path.clone(),
            ));
        }
        for skill in &self.catalog.skills {
            if !self.selected_paths.contains(&skill.path) {
                continue;
            }
            context = context.with_skill(SkillRef::new(
                skill.name.clone(),
                skill.description.clone(),
                skill.path.clone(),
            ));
        }
        context
    }

    fn reload(&mut self) {
        self.catalog = SkillCatalog::load_from_roots(self.roots.clone());
        let available = self
            .catalog
            .skills
            .iter()
            .map(|skill| skill.path.clone())
            .collect::<HashSet<_>>();
        self.selected_paths.retain(|path| available.contains(path));
    }

    fn skill_by_id(&self, id: &str) -> Option<&SkillMetadata> {
        self.catalog
            .skills
            .iter()
            .find(|skill| skill_id(skill).as_ref() == id)
    }
}

fn skill_id(skill: &SkillMetadata) -> SharedString {
    SharedString::from(skill.path.display().to_string())
}

fn default_import_root() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".codex")
        .join("skills")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod workspace_skills_tests {
    use super::AgentSkillState;
    use std::fs;

    fn make_skill(root: &std::path::Path, name: &str) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name} skill\n---\nbody\n"),
        )
        .unwrap();
    }

    #[test]
    fn workspace_skills_follow_the_selected_root() {
        let base = std::env::temp_dir().join(format!(
            "navop-skills-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project_a = base.join("project-a");
        let project_b = base.join("project-b");
        make_skill(&project_a.join(".codex").join("skills"), "alpha");
        make_skill(&project_b.join(".codex").join("skills"), "beta");

        let mut state = AgentSkillState::load_for_workspace(&project_a);
        let names: Vec<&str> = state
            .catalog
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect();
        assert!(names.contains(&"alpha"), "{names:?}");
        assert!(!names.contains(&"beta"), "{names:?}");

        state.reload_for_workspace(&project_b);
        let names: Vec<&str> = state
            .catalog
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect();
        assert!(names.contains(&"beta"), "{names:?}");
        assert!(!names.contains(&"alpha"), "{names:?}");

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn reloading_keeps_home_skills_and_drops_stale_selections() {
        // 项目 skill 选中后切走，选择集合应丢弃失效路径。
        let base = std::env::temp_dir().join(format!(
            "navop-skills-sel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = base.join("project");
        let skills_root = project.join(".codex").join("skills");
        make_skill(&skills_root, "temp-skill");

        let mut state = AgentSkillState::load_for_workspace(&project);
        let skill_path = state
            .catalog
            .skills
            .iter()
            .find(|skill| skill.name == "temp-skill")
            .expect("temp-skill should load from workspace root")
            .path
            .clone();
        state.selected_paths.insert(skill_path.clone());

        // 目录消失后 reload：该选择被清掉，home 全局 skills 不受影响。
        fs::remove_dir_all(&skills_root).unwrap();
        state.reload_for_workspace(&project);

        assert!(
            !state.selected_paths.contains(&skill_path),
            "失效路径的选择应被丢弃: {skill_path:?}"
        );
        assert!(state.catalog.skills.iter().all(|skill| skill.name != "temp-skill"));

        fs::remove_dir_all(&base).ok();
    }
}
