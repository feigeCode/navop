use gpui::{AnyElement, IntoElement, ParentElement, Styled, div};
use gpui_component::{ActiveTheme, h_flex};
use one_core::keybindings::{action_id, shortcuts_for};
use rust_i18n::t;

use super::{OPEN_LOCAL_TERMINAL_SHORTCUT_MACOS, OPEN_LOCAL_TERMINAL_SHORTCUT_OTHER};

pub(super) fn render_shortcuts(cx: &gpui::App) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_center()
        .flex_wrap()
        .gap_4()
        .pt_1()
        .children([
            shortcut_hint(
                "HOME_QUICK_OPEN",
                action_id::HOME_QUICK_OPEN,
                quick_open_default(),
                cx,
            ),
            shortcut_hint(
                "HOME_NEW_CONNECTION",
                action_id::HOME_NEW_CONNECTION,
                new_default(),
                cx,
            ),
            shortcut_hint(
                "HOME_OPEN_LOCAL_TERMINAL",
                action_id::HOME_OPEN_LOCAL_TERMINAL,
                terminal_default(),
                cx,
            ),
        ])
}

fn shortcut_hint(label_key: &str, action: &str, fallback: &str, cx: &gpui::App) -> AnyElement {
    let shortcut = shortcuts_for(cx, action, &[fallback])
        .into_iter()
        .next()
        .unwrap_or_else(|| fallback.to_string());
    let label_key = format!("Home.StartCenter.Shortcut.{label_key}");
    let label = t!(&label_key).to_string();
    h_flex()
        .gap_2()
        .items_center()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(shortcut_badge(shortcut, cx))
        .child(label)
        .into_any_element()
}

fn shortcut_badge(shortcut: String, cx: &gpui::App) -> impl IntoElement {
    div()
        .px_1p5()
        .py_0p5()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .text_color(cx.theme().foreground)
        .child(shortcut)
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
