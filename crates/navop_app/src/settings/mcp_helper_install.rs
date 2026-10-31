use gpui::{ParentElement, Styled, div};
use gpui_component::{
    ActiveTheme,
    setting::{SettingField, SettingItem},
    v_flex,
};
use public_mcp::client_config::{ClientConfigInstall, NAVOP_MCP_CLIENT_TAG};
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
    let install = ClientConfigInstall::from_current_app();
    let command = install.launch_spec.command;
    // GUI 进程的 PATH 往往看不到 Homebrew / nvm 安装的 npx，这里做真实探测，
    // 避免在 npx 缺失时仍然显示「就绪」，把失败推迟到点击安装的那一刻。
    match process_util::resolve_program(&command) {
        Some(path) => t!(
            "Settings.General.Mcp.runtime_requirements_ready",
            command = command,
            path = path.display().to_string(),
            package = format!("@navop/mcp@{NAVOP_MCP_CLIENT_TAG}")
        )
        .to_string(),
        None => t!(
            "Settings.General.Mcp.runtime_requirements_missing",
            command = command
        )
        .to_string(),
    }
}
