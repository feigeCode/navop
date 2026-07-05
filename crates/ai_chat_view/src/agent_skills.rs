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
    pub(crate) fn load_default() -> Self {
        let roots = default_skill_roots();
        Self::load(roots)
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

fn default_skill_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join(".codex").join("skills"));
        roots.push(cwd.join(".agents").join("skills"));
    }
    if let Some(home) = home_dir() {
        roots.push(home.join(".codex").join("skills"));
        roots.push(home.join(".agents").join("skills"));
    }
    roots
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
