use gpui::{ParentElement, Styled, div};
use gpui_component::{
    ActiveTheme,
    setting::{SettingField, SettingItem},
    v_flex,
};
use public_mcp::client_config::{ClientConfigInstall, NAVOP_MCP_CLIENT_VERSION};
use rust_i18n::t;

#[cfg(test)]
const MCP_RUNTIME_REQUIREMENTS_ITEM_ID: &str = "mcp-runtime-requirements";

pub(crate) fn mcp_runtime_requirements_item() -> SettingItem {
    SettingItem::new(
        t!("Settings.General.Mcp.runtime_requirements"),
        SettingField::render(|_, _, cx| {
            let status = runtime_requirements_status();
            v_flex().gap_1().child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(status),
            )
        }),
    )
    .description(t!("Settings.General.Mcp.runtime_requirements_desc").to_string())
}

#[cfg(test)]
pub(crate) fn mcp_runtime_requirements_item_id() -> &'static str {
    MCP_RUNTIME_REQUIREMENTS_ITEM_ID
}

fn runtime_requirements_status() -> String {
    match ClientConfigInstall::from_current_app() {
        Ok(install) => t!(
            "Settings.General.Mcp.runtime_requirements_ready",
            path = install.launcher_path.display().to_string(),
            package = format!("@navop/mcp@{NAVOP_MCP_CLIENT_VERSION}")
        )
        .to_string(),
        Err(error) => t!(
            "Settings.General.Mcp.runtime_requirements_missing",
            error = error.to_string()
        )
        .to_string(),
    }
}
