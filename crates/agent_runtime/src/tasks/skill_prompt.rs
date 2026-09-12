use rust_i18n::t;

use crate::skill::SkillContext;

pub(crate) fn skill_section(skills: &SkillContext) -> String {
    if skills.is_empty() {
        return String::new();
    }
    t!(
        "AgentRuntime.system_prompt_skill_section",
        skills = skills.prompt_section()
    )
    .to_string()
}
