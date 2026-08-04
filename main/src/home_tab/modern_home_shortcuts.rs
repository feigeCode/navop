use gpui::{AnyElement, IntoElement, ParentElement, Styled, div};
use gpui_component::ActiveTheme;
use one_core::keybindings::{action_id, shortcuts_for};

use super::{OPEN_LOCAL_TERMINAL_SHORTCUT_MACOS, OPEN_LOCAL_TERMINAL_SHORTCUT_OTHER};

pub(super) fn quick_open_shortcut(cx: &gpui::App) -> AnyElement {
    shortcut_badge_for(action_id::HOME_QUICK_OPEN, quick_open_default(), cx)
}

pub(super) fn new_connection_shortcut(cx: &gpui::App) -> AnyElement {
    shortcut_badge_for(action_id::HOME_NEW_CONNECTION, new_default(), cx)
}

pub(super) fn terminal_shortcut(cx: &gpui::App) -> AnyElement {
    shortcut_badge_for(action_id::HOME_OPEN_LOCAL_TERMINAL, terminal_default(), cx)
}

fn shortcut_badge_for(action: &str, fallback: &str, cx: &gpui::App) -> AnyElement {
    let shortcut = shortcuts_for(cx, action, &[fallback])
        .into_iter()
        .next()
        .unwrap_or_else(|| fallback.to_string());

    div()
        .px_1p5()
        .py_0p5()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(shortcut)
        .into_any_element()
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
