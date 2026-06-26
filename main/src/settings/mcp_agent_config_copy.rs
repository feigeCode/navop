use gpui::{App, ClipboardItem, ParentElement, Window};
use gpui_component::{
    WindowExt,
    button::Button,
    h_flex,
    notification::Notification,
    setting::{SettingField, SettingItem},
};
use public_mcp::client_config::{ClientConfigInstall, agent_mcp_config_json};
use rust_i18n::t;

const MCP_AGENT_CONFIG_COPY_BUTTON_ID: &str = "mcp-copy-agent-config";

pub(crate) fn mcp_agent_config_copy_item() -> SettingItem {
    SettingItem::new(
        t!("Settings.General.Mcp.copy_agent_config"),
        SettingField::render(|_, _, _| {
            h_flex().child(
                Button::new(MCP_AGENT_CONFIG_COPY_BUTTON_ID)
                    .label(t!("Settings.General.Mcp.copy_agent_config_button").to_string())
                    .on_click(|_, window, cx| copy_agent_mcp_config(window, cx)),
            )
        }),
    )
    .description(t!("Settings.General.Mcp.copy_agent_config_desc").to_string())
}

#[cfg(test)]
pub(crate) fn mcp_agent_config_copy_item_id() -> &'static str {
    MCP_AGENT_CONFIG_COPY_BUTTON_ID
}

fn copy_agent_mcp_config(window: &mut Window, cx: &mut App) {
    let result =
        ClientConfigInstall::from_current_app().and_then(|install| agent_mcp_config_json(&install));
    match result {
        Ok(config) => {
            cx.write_to_clipboard(ClipboardItem::new_string(config));
            window.push_notification(
                Notification::success(t!("Settings.General.Mcp.copy_agent_config_success"))
                    .autohide(true),
                cx,
            );
        }
        Err(error) => window.push_notification(
            Notification::error(
                t!(
                    "Settings.General.Mcp.copy_agent_config_failed",
                    error = error.to_string()
                )
                .to_string(),
            )
            .autohide(true),
            cx,
        ),
    }
}
