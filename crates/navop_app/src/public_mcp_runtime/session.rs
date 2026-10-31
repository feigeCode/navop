use super::{GlobalPublicMcpRuntime, config, reconcile_runtime};
use gpui::App;
use one_core::settings::{AppSettings, McpServerMode};

pub fn mcp_server_enabled(cx: &App) -> bool {
    let settings = AppSettings::current(cx);
    config::effective_server_enabled(&settings, runtime_session_enabled(cx))
}

pub fn set_mcp_server_enabled(cx: &mut App, enabled: bool) {
    let mut settings = AppSettings::current(cx);
    let mut session_enabled = runtime_session_enabled(cx);
    config::apply_server_enabled_for_mode(&mut settings, &mut session_enabled, enabled);
    apply_mcp_settings(cx, settings, session_enabled);
}

pub fn set_mcp_server_mode(cx: &mut App, mode: McpServerMode) {
    let mut settings = AppSettings::current(cx);
    let mut session_enabled = runtime_session_enabled(cx);
    config::apply_server_mode_preserving_enabled(&mut settings, &mut session_enabled, mode);
    apply_mcp_settings(cx, settings, session_enabled);
}

pub(super) fn runtime_session_enabled(cx: &App) -> bool {
    cx.try_global::<GlobalPublicMcpRuntime>()
        .map(|state| state.session_enabled)
        .unwrap_or_default()
}

fn apply_mcp_settings(cx: &mut App, settings: AppSettings, session_enabled: bool) {
    if cx.has_global::<GlobalPublicMcpRuntime>() {
        cx.global_mut::<GlobalPublicMcpRuntime>().session_enabled = session_enabled;
    }
    settings.save();
    cx.set_global(settings);
    if cx.has_global::<GlobalPublicMcpRuntime>() {
        reconcile_runtime(cx);
    }
}
