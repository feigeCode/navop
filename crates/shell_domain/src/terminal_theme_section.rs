//! 设置页「终端主题」区块：由 shell 领域渲染，公共壳只放一个贡献点。

use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use one_core::settings::AppSettings;
use rust_i18n::t;

const THEME_CARD_WIDTH: f32 = 132.0;
const THEME_CARD_HEIGHT: f32 = 64.0;
const THEME_MARKER_SIZE: f32 = 12.0;
const THEME_PREVIEW_HEIGHT: f32 = 8.0;

use terminal_view::TerminalTheme;

/// 渲染终端主题选择区块。
pub(crate) fn render(cx: &App) -> AnyElement {
    render_terminal_palette(cx).into_any_element()
}

fn render_terminal_palette(cx: &App) -> impl IntoElement {
    let selected = AppSettings::global(cx).terminal_theme.as_str();
    let themes = TerminalTheme::all(cx.theme())
        .into_iter()
        .map(|theme| render_terminal_theme_card(theme, selected, cx));

    v_flex()
        .gap_2()
        .child(
            v_flex()
                .gap_1()
                .child(div().child(t!("Settings.General.Appearance.terminal_theme")))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Settings.General.Appearance.terminal_theme_desc")),
                ),
        )
        .child(h_flex().flex_wrap().gap_2().children(themes))
}

fn render_terminal_theme_card(theme: TerminalTheme, selected: &str, cx: &App) -> impl IntoElement {
    let is_selected = theme.name == selected;
    let theme_name = theme.name;
    let display_name = theme.display_name();

    div()
        .id(SharedString::from(format!(
            "terminal-theme-card-{}",
            theme.name
        )))
        .w(px(THEME_CARD_WIDTH))
        .h(px(THEME_CARD_HEIGHT))
        .p_2()
        .rounded(cx.theme().radius)
        .border(px(if is_selected { 2.0 } else { 1.0 }))
        .border_color(if is_selected {
            cx.theme().primary
        } else {
            cx.theme().border
        })
        .bg(theme.background)
        .text_color(theme.foreground)
        .cursor_pointer()
        .on_click(move |_, _, cx| set_terminal_theme(theme_name, cx))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .size(px(THEME_MARKER_SIZE))
                        .rounded_full()
                        .bg(theme.cursor),
                )
                .child(div().flex_1().text_xs().truncate().child(display_name)),
        )
        .child(
            div()
                .mt_2()
                .h(px(THEME_PREVIEW_HEIGHT))
                .rounded(cx.theme().radius)
                .bg(theme.selection),
        )
}

fn set_terminal_theme(theme_name: &'static str, cx: &mut App) {
    if AppSettings::global(cx).terminal_theme == theme_name {
        return;
    }
    AppSettings::update_and_save(cx, |settings| {
        settings.terminal_theme = theme_name.to_string();
    });
}

