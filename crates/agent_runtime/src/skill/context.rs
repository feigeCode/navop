use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

impl SkillSummary {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            path: path.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRef {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

impl SkillRef {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            path: path.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillContext {
    pub available_skills: Vec<SkillSummary>,
    pub skills: Vec<SkillRef>,
}

impl SkillContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_skill(mut self, skill: SkillRef) -> Self {
        if !self
            .skills
            .iter()
            .any(|existing| existing.path == skill.path)
        {
            self.skills.push(skill);
        }
        self
    }

    pub fn with_available_skill(mut self, skill: SkillSummary) -> Self {
        if !self
            .available_skills
            .iter()
            .any(|existing| existing.path == skill.path)
        {
            self.available_skills.push(skill);
        }
        self
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty() && self.available_skills.is_empty()
    }

    pub fn describe(&self) -> String {
        self.catalog()
            .iter()
            .map(|skill| {
                format!(
                    "- {} | {} | path={}",
                    skill.name,
                    skill.description,
                    skill.path.display()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn catalog(&self) -> Vec<SkillSummary> {
        let mut catalog = self.available_skills.clone();
        for skill in &self.skills {
            if catalog.iter().any(|available| available.path == skill.path) {
                continue;
            }
            catalog.push(SkillSummary::new(
                skill.name.clone(),
                skill.description.clone(),
                skill.path.clone(),
            ));
        }
        catalog
    }

    pub fn wrap_user_prompt(&self, text: &str) -> String {
        if self.is_empty() {
            return text.to_string();
        }
        format!("{}\n\nUser request:\n{}", self.prompt_section(), text)
    }

    pub fn prompt_section(&self) -> String {
        let mut out = String::from("Skill context for this turn:\n");
        let catalog = self.catalog();
        if !catalog.is_empty() {
            out.push_str("\nAvailable skill catalog (metadata only):\n");
            push_skill_lines(&mut out, &catalog);
        }
        if !self.skills.is_empty() {
            out.push_str("\nSelected skills for this turn (metadata only):\n");
            let selected = self
                .skills
                .iter()
                .map(|skill| {
                    SkillSummary::new(
                        skill.name.clone(),
                        skill.description.clone(),
                        skill.path.clone(),
                    )
                })
                .collect::<Vec<_>>();
            push_skill_lines(&mut out, &selected);
        }
        out
    }

    pub fn selected_names_csv(&self) -> String {
        self.skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn push_skill_lines(out: &mut String, skills: &[SkillSummary]) {
    for skill in skills {
        out.push_str(&format!(
            "- {} | {} | path={}\n",
            skill.name,
            skill.description,
            skill.path.display()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_skill_catalog_wraps_user_prompt_without_full_instructions() {
        let context = SkillContext::new().with_available_skill(SkillSummary::new(
            "using-superpowers",
            "Use Superpowers workflows",
            "/tmp/skills/using-superpowers/SKILL.md",
        ));

        let prompt = context.wrap_user_prompt("你有哪些 skill");

        assert!(prompt.contains("Available skill catalog"));
        assert!(prompt.contains("using-superpowers"));
        assert!(prompt.contains("User request:\n你有哪些 skill"));
        assert!(!prompt.contains("Instructions:"));
    }
}
