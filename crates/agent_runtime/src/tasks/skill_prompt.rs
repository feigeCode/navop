use crate::skill::SkillContext;

pub(crate) fn append_skill_context(prompt: &mut String, skills: &SkillContext) {
    if skills.is_empty() {
        return;
    }
    prompt.push_str("\n\nSkill:\n");
    prompt.push_str(&skills.prompt_section());
    prompt.push_str(
        "规则: Skill 目录只提供元数据,完整说明不会内联在 prompt 中。\
当用户要求使用某个 Skill,或某个 Skill 的 description 明确匹配当前任务时,必须先调用 `load_skill` 读取该 Skill 的完整说明,再按返回的说明执行。\
如果 Skill 说明引用了相对路径文件、references、scripts、templates 或其它同目录资源,调用 `read_skill_file` 读取对应文件。\
不要声称需要不存在的 Skill 或 activate_skill 工具;如果 `load_skill` 在可用工具列表中,就使用它。\
当用户询问有哪些 Skill 时,基于目录元数据回答。",
    );
}
