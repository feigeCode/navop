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

/// [`SkillContext::wrap_user_prompt`] 注入的技能上下文固定以此开头。
///
/// 恢复历史时靠它认出「这条不是用户打的，是注入的」，见
/// [`SkillContext::unwrap_user_prompt`]。
const PROMPT_SECTION_PREFIX: &str = "Skill context for this turn:\n";

/// 注入的上下文与用户真正输入之间的分隔标记。
const USER_REQUEST_MARKER: &str = "\n\nUser request:\n";

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
        format!("{}{}{}", self.prompt_section(), USER_REQUEST_MARKER, text)
    }

    /// [`Self::wrap_user_prompt`] 的逆操作：剥掉注入的技能上下文，只留用户真正输入的那段。
    ///
    /// 恢复历史时必须用它。发给 agent 的是**包装过**的 prompt，agent 把这条原样记进自己的
    /// 会话，`session/load` 时再整段重放回来；不剥的话，界面会把整张技能目录当成一条用户
    /// 消息显示出来（「Skill context for this turn:」开头的一大段列表）。
    ///
    /// 只认 [`PROMPT_SECTION_PREFIX`] 这个精确开头，普通输入原样返回。
    pub fn unwrap_user_prompt(text: &str) -> &str {
        if !text.starts_with(PROMPT_SECTION_PREFIX) {
            return text;
        }
        // 取**第一个**标记：`prompt_section` 自己不会产生它，所以第一个一定是分隔符，
        // 哪怕用户输入里也写了同样的字样。
        match text.find(USER_REQUEST_MARKER) {
            Some(at) => &text[at + USER_REQUEST_MARKER.len()..],
            None => text,
        }
    }

    pub fn prompt_section(&self) -> String {
        let mut out = String::from(PROMPT_SECTION_PREFIX);
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

    #[test]
    fn unwrapping_a_wrapped_prompt_returns_the_user_text() {
        let context = SkillContext::new().with_available_skill(SkillSummary::new(
            "using-superpowers",
            "Use Superpowers workflows",
            "/tmp/skills/using-superpowers/SKILL.md",
        ));

        let wrapped = context.wrap_user_prompt("帮我看看这个 bug");

        assert_eq!(
            SkillContext::unwrap_user_prompt(&wrapped),
            "帮我看看这个 bug"
        );
    }

    #[test]
    fn unwrapping_leaves_plain_user_text_alone() {
        assert_eq!(
            SkillContext::unwrap_user_prompt("就是一句普通输入"),
            "就是一句普通输入"
        );
    }

    /// 用户自己打的内容里也可能出现分隔标记：`prompt_section` 不会产生它，所以第一个
    /// 一定是分隔符，剥完要原样留下用户那段（而不是在用户自己写的标记处再切一刀）。
    #[test]
    fn unwrapping_keeps_a_marker_the_user_typed_themselves() {
        let context = SkillContext::new()
            .with_available_skill(SkillSummary::new("demo", "Demo", "/tmp/demo/SKILL.md"));

        let typed = "原文里带一段：\n\nUser request:\n这行是用户自己写的";
        let wrapped = context.wrap_user_prompt(typed);

        assert_eq!(SkillContext::unwrap_user_prompt(&wrapped), typed);
    }
}
