//! 思考级别（reasoning effort）的展示与解析。
//!
//! provider 配置里存的是「默认思考级别」，聊天面板里允许临时切换；这里的
//! id 既用于下拉项，也用于事件回传（`auto` / `none` / `low` / `medium` / `high`）。

use gpui::SharedString;
use llm_connector::types::ReasoningEffort;
use rust_i18n::t;

use crate::input::ComposerMenuOption;

/// 「自动」：请求里不写 `reasoning_effort`，跟随上游默认。
pub const AUTO_EFFORT_ID: &str = "auto";

/// 选项 id；`None` 映射为 `auto`。
pub fn effort_id(effort: Option<ReasoningEffort>) -> &'static str {
    match effort {
        None => AUTO_EFFORT_ID,
        Some(effort) => effort.as_str(),
    }
}

/// 把下拉项的 id 还原成思考级别；`auto` 返回 `Some(None)`，无法识别时返回 `None`。
pub fn parse_effort_id(id: &str) -> Option<Option<ReasoningEffort>> {
    let id = id.trim();
    if id.eq_ignore_ascii_case(AUTO_EFFORT_ID) {
        return Some(None);
    }
    ReasoningEffort::parse(id).map(Some)
}

/// 当前档位文案。
pub fn effort_label(effort: Option<ReasoningEffort>) -> SharedString {
    match effort {
        None => t!("AgentUi.reasoning_effort_auto").to_string().into(),
        Some(ReasoningEffort::None) => t!("AgentUi.reasoning_effort_none").to_string().into(),
        Some(ReasoningEffort::Low) => t!("AgentUi.reasoning_effort_low").to_string().into(),
        Some(ReasoningEffort::Medium) => t!("AgentUi.reasoning_effort_medium").to_string().into(),
        Some(ReasoningEffort::High) => t!("AgentUi.reasoning_effort_high").to_string().into(),
    }
}

/// 下拉项：自动 / 关闭 / 低 / 中 / 高。
pub fn effort_menu_options() -> Vec<ComposerMenuOption> {
    [
        None,
        Some(ReasoningEffort::None),
        Some(ReasoningEffort::Low),
        Some(ReasoningEffort::Medium),
        Some(ReasoningEffort::High),
    ]
    .into_iter()
    .map(|effort| ComposerMenuOption::new(effort_id(effort), effort_label(effort)))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_ids_round_trip() {
        for effort in [
            None,
            Some(ReasoningEffort::None),
            Some(ReasoningEffort::Low),
            Some(ReasoningEffort::Medium),
            Some(ReasoningEffort::High),
        ] {
            let id = effort_id(effort);
            assert_eq!(parse_effort_id(id), Some(effort), "id {id}");
        }
    }

    #[test]
    fn auto_maps_to_no_parameter() {
        assert_eq!(parse_effort_id("auto"), Some(None));
        assert_eq!(parse_effort_id(" AUTO "), Some(None));
        assert_eq!(parse_effort_id("bogus"), None);
    }

    #[test]
    fn menu_has_five_levels_in_order() {
        let ids: Vec<String> = effort_menu_options()
            .into_iter()
            .map(|option| option.id.to_string())
            .collect();
        assert_eq!(ids, ["auto", "none", "low", "medium", "high"]);
    }
}
