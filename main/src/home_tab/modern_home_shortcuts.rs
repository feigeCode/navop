use one_core::keybindings::{action_id, shortcuts_for};
use rust_i18n::t;

use super::{OPEN_LOCAL_TERMINAL_SHORTCUT_MACOS, OPEN_LOCAL_TERMINAL_SHORTCUT_OTHER};

/// 快捷键不再以独立 badge 占据首屏视觉，统一收进按钮 tooltip（"标签（快捷键）"）。
pub(super) fn quick_open_tooltip(cx: &gpui::App) -> String {
    let shortcut = shortcut_text_for(action_id::HOME_QUICK_OPEN, quick_open_default(), cx);
    tooltip_with_shortcut(t!("Home.StartCenter.quick_open"), &shortcut)
}

pub(super) fn new_connection_tooltip(cx: &gpui::App) -> String {
    let shortcut = shortcut_text_for(action_id::HOME_NEW_CONNECTION, new_default(), cx);
    tooltip_with_shortcut(t!("Home.new_connection"), &shortcut)
}

pub(super) fn terminal_tooltip(cx: &gpui::App) -> String {
    let shortcut = shortcut_text_for(action_id::HOME_OPEN_LOCAL_TERMINAL, terminal_default(), cx);
    tooltip_with_shortcut(t!("Home.local_terminal"), &shortcut)
}

fn shortcut_text_for(action: &str, fallback: &str, cx: &gpui::App) -> String {
    shortcuts_for(cx, action, &[fallback])
        .into_iter()
        .next()
        .unwrap_or_else(|| fallback.to_string())
}

fn tooltip_with_shortcut(label: std::borrow::Cow<'_, str>, shortcut: &str) -> String {
    format!("{label}（{shortcut}）")
}

fn quick_open_default() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-o"
    } else {
        "alt-o"
    }
}

fn new_default() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-n"
    } else {
        "alt-n"
    }
}

fn terminal_default() -> &'static str {
    if cfg!(target_os = "macos") {
        OPEN_LOCAL_TERMINAL_SHORTCUT_MACOS
    } else {
        OPEN_LOCAL_TERMINAL_SHORTCUT_OTHER
    }
}
