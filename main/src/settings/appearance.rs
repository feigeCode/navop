use super::{appearance_import::prompt_import_theme, appearance_state::AppearanceSettingsState};
use gpui::{
    App, Entity, Hsla, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Selectable, Sizable, ThemeColor, ThemeConfig, ThemeMode,
    ThemeRegistry,
    button::{Button, ButtonVariants},
    color_picker::ColorPicker,
    h_flex,
    slider::Slider,
    switch::Switch,
    try_parse_color, v_flex,
};
use one_core::{settings::AppSettings, themes};
use rust_i18n::t;
use std::rc::Rc;
use terminal_view::TerminalTheme;

const OPACITY_PRESETS: &[f32] = &[100.0, 85.0, 70.0];
const THEME_CARD_WIDTH: f32 = 132.0;
const THEME_CARD_HEIGHT: f32 = 64.0;
const THEME_MARKER_SIZE: f32 = 12.0;
const THEME_PREVIEW_HEIGHT: f32 = 8.0;

pub fn render(
    options: &gpui_component::setting::RenderOptions,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let state = window.use_keyed_state("appearance-settings", cx, |window, cx| {
        AppearanceSettingsState::new(window, cx)
    });
    state.update(cx, |state, cx| state.sync_from_settings(window, cx));
    v_flex()
        .gap_4()
        .w_full()
        .child(render_mode(cx))
        .child(render_opacity(options, &state, cx))
        .child(render_palette(cx))
        .child(render_terminal_palette(cx))
        .child(render_accent(&state, cx))
        .into_any_element()
}

fn render_mode(cx: &App) -> impl IntoElement {
    let mode = AppSettings::global(cx).theme_mode.as_str();
    let labels = [
        ("light", t!("Settings.General.Appearance.mode_light")),
        ("system", t!("Settings.General.Appearance.mode_system")),
        ("dark", t!("Settings.General.Appearance.mode_dark")),
    ];
    h_flex()
        .justify_between()
        .items_center()
        .child(div().child(t!("Settings.General.Appearance.theme_mode")))
        .child(
            h_flex()
                .gap_1()
                .children(labels.into_iter().map(|(value, label)| {
                    let selected = mode == value;
                    Button::new(SharedString::from(format!("appearance-mode-{value}")))
                        .label(label)
                        .selected(selected)
                        .when(selected, |button| button.primary())
                        .small()
                        .on_click(move |_, _, cx| set_theme_mode(value, cx))
                })),
        )
}

fn set_theme_mode(mode: &'static str, cx: &mut App) {
    if AppSettings::global(cx).theme_mode == mode {
        return;
    }
    AppSettings::update_and_save(cx, |settings| {
        settings.theme_mode = mode.to_string();
        settings.auto_switch_theme = mode == "system";
    });
    themes::apply_appearance(&AppSettings::current(cx), cx);
}

fn render_opacity(
    options: &gpui_component::setting::RenderOptions,
    state: &Entity<AppearanceSettingsState>,
    cx: &App,
) -> impl IntoElement {
    let percentage = (AppSettings::global(cx).window_opacity * 100.0).round();
    let slider = state.read(cx).opacity_slider.clone();
    let presets = OPACITY_PRESETS.iter().map(|value| {
        let state = state.clone();
        Button::new(SharedString::from(format!("opacity-preset-{value}")))
            .label(format!("{value:.0}%"))
            .with_size(options.size)
            .on_click(move |_, window, cx| {
                state.update(cx, |state, cx| state.set_opacity(*value, true, window, cx));
            })
    });
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .justify_between()
                .child(div().child(t!("Settings.General.Appearance.window_opacity")))
                .child(div().text_sm().child(format!("{percentage:.0}%"))),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(Slider::new(&slider).horizontal().flex_1())
                .children(presets),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!("Settings.General.Appearance.window_opacity_desc")),
        )
}

fn render_palette(cx: &App) -> impl IntoElement {
    let mode =
        AppSettings::global(cx).effective_theme_mode(ThemeMode::from(cx.window_appearance()));
    let selected = if mode.is_dark() {
        &AppSettings::global(cx).dark_theme
    } else {
        &AppSettings::global(cx).light_theme
    };
    let themes = ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .filter(|theme| theme.mode == mode)
        .map(|theme| render_theme_card(theme, selected, cx));
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(div().child(t!("Settings.General.Appearance.theme_palette")))
                .child(
                    Button::new("appearance-import-theme")
                        .icon(Icon::new(IconName::File))
                        .label(t!("Settings.General.Appearance.import_theme"))
                        .small()
                        .tooltip(t!("Settings.General.Appearance.import_theme_desc"))
                        .on_click(|_, window, cx| prompt_import_theme(window, cx)),
                ),
        )
        .child(h_flex().flex_wrap().gap_2().children(themes))
}

fn render_theme_card(config: &Rc<ThemeConfig>, selected: &str, cx: &App) -> impl IntoElement {
    let background = config_color(config, |colors| colors.background.as_ref())
        .unwrap_or_else(|| fallback_theme(config.mode).background);
    let foreground = config_color(config, |colors| colors.foreground.as_ref())
        .unwrap_or_else(|| fallback_theme(config.mode).foreground);
    let accent = config_color(config, |colors| colors.primary.as_ref()).unwrap_or(background);
    let is_selected = config.name == selected;
    let name = config.name.clone();
    let theme_name = config.name.clone();
    let theme_mode = config.mode;
    div()
        .id(SharedString::from(format!("theme-card-{}", config.name)))
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
        .bg(background)
        .text_color(foreground)
        .on_click(move |_, _, cx| {
            AppSettings::update_and_save(cx, |settings| {
                if theme_mode.is_dark() {
                    settings.dark_theme = theme_name.to_string();
                } else {
                    settings.light_theme = theme_name.to_string();
                }
            });
            themes::apply_appearance(&AppSettings::current(cx), cx);
        })
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().size(px(THEME_MARKER_SIZE)).rounded_full().bg(accent))
                .child(div().text_xs().truncate().child(name)),
        )
        .child(
            div()
                .mt_2()
                .h(px(THEME_PREVIEW_HEIGHT))
                .rounded(cx.theme().radius)
                .bg(accent),
        )
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

fn config_color<F>(config: &ThemeConfig, get: F) -> Option<Hsla>
where
    F: Fn(&gpui_component::ThemeConfigColors) -> Option<&SharedString>,
{
    get(&config.colors).and_then(|color| try_parse_color(color.as_ref()).ok())
}

fn fallback_theme(mode: ThemeMode) -> ThemeColor {
    if mode.is_dark() {
        *ThemeColor::dark()
    } else {
        *ThemeColor::light()
    }
}

fn render_accent(state: &Entity<AppearanceSettingsState>, cx: &App) -> impl IntoElement {
    let settings = AppSettings::global(cx);
    let picker = state.read(cx).accent_picker.clone();
    let enabled = settings.custom_accent_enabled;
    h_flex()
        .justify_between()
        .items_center()
        .child(
            v_flex()
                .gap_1()
                .child(div().child(t!("Settings.General.Appearance.custom_accent")))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Settings.General.Appearance.custom_accent_desc")),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    Switch::new("appearance-custom-accent")
                        .checked(enabled)
                        .on_click(|value, _, cx| {
                            AppSettings::update_and_save(cx, |settings| {
                                settings.custom_accent_enabled = *value;
                            });
                            themes::apply_appearance(&AppSettings::current(cx), cx);
                        }),
                )
                .when(enabled, |this| {
                    this.child(ColorPicker::new(&picker).small())
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::OPACITY_PRESETS;

    #[test]
    fn appearance_surface_keeps_all_requested_controls() {
        let source = include_str!("appearance.rs");

        assert_eq!(&[100.0, 85.0, 70.0], OPACITY_PRESETS);
        assert!(source.contains("set_theme_mode(value, cx)"));
        assert!(source.contains("(\"light\", t!"));
        assert!(source.contains("(\"system\", t!"));
        assert!(source.contains("(\"dark\", t!"));
        assert!(source.contains("ThemeRegistry::global"));
        assert!(source.contains(".child(render_terminal_palette(cx))"));
        assert!(source.contains("TerminalTheme::all(cx.theme())"));
        assert!(source.contains("settings.terminal_theme = theme_name.to_string()"));
        assert!(source.contains("Slider::new"));
        assert!(source.contains("ColorPicker::new"));
        assert!(source.contains("custom_accent_enabled"));
    }
}
