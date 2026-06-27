use gpui::{App, SharedString};
use gpui_component::setting::{SettingField, SettingItem};
use one_core::settings::{AcpAgentSettings, AiChatSettings, AppSettings};
use rust_i18n::t;

pub fn acp_agents_json_item(default_settings: &AiChatSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.AiChat.acp_agents_json"),
        SettingField::input(
            |cx: &App| format_acp_agents_json(&AppSettings::global(cx).ai_chat).into(),
            |val: SharedString, cx: &mut App| {
                let Some(agents) = parse_acp_agents_json(val.as_ref()) else {
                    tracing::warn!("Invalid AI Chat ACP agents JSON setting");
                    return;
                };
                AppSettings::update_and_save(cx, |settings| {
                    settings.ai_chat.acp_agents = agents;
                });
            },
        )
        .default_value(format_acp_agents_json(default_settings)),
    )
    .description(t!("Settings.General.AiChat.acp_agents_json_desc").to_string())
}

fn format_acp_agents_json(settings: &AiChatSettings) -> String {
    serde_json::to_string(&settings.acp_agents).unwrap_or_else(|_| "[]".to_string())
}

fn parse_acp_agents_json(value: &str) -> Option<Vec<AcpAgentSettings>> {
    serde_json::from_str::<Vec<AcpAgentSettings>>(value).ok()
}

#[cfg(test)]
pub(super) fn format_acp_agents_json_for_test(settings: &AiChatSettings) -> String {
    format_acp_agents_json(settings)
}

#[cfg(test)]
pub(super) fn parse_acp_agents_json_for_test(value: &str) -> Option<Vec<AcpAgentSettings>> {
    parse_acp_agents_json(value)
}
