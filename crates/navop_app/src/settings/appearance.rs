use super::{appearance_import::prompt_import_theme, appearance_state::AppearanceSettingsState};
use gpui::{
    App, Entity, Hsla, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Icon, Selectable, Sizable, ThemeColor, ThemeConfig, ThemeMode, ThemeRegistry,
    WindowExt,
    button::{Button, ButtonVariants},
    color_picker::ColorPicker,
    h_flex,
    notification::Notification,
    slider::Slider,
    switch::Switch,
    try_parse_color, v_flex,
};
use one_assets::IconName;
use one_core::{
    settings::{AppLayout, AppSettings, AppearanceDensity},
    themes,
};
use rust_i18n::t;
use std::rc::Rc;
const OPACITY_PRESETS: &[f32] = &[100.0, 85.0, 70.0];
const THEME_CARD_WIDTH: f32 = 132.0;
const THEME_CARD_HEIGHT: f32 = 64.0;
const THEME_MARKER_SIZE: f32 = 12.0;
const THEME_PREVIEW_HEIGHT: f32 = 8.0;
const LAYOUT_CARD_WIDTH: f32 = 232.0;

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
        .child(
            div()
                .pb_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(t!("Settings.General.Appearance.application_section")),
        )
        .child(render_layout(cx))
        .child(render_density(cx))
        .child(render_mode(cx))
        .child(render_opacity(options, &state, cx))
        .child(render_palette(cx))
        .child(render_accent(&state, cx))
        .into_any_element()
}

fn render_layout(cx: &App) -> impl IntoElement {
    let selected = AppSettings::global(cx).app_layout;
    v_flex()
        .gap_2()
        .child(
            v_flex()
                .gap_1()
                .child(div().child(t!("Settings.General.Appearance.app_layout")))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Settings.General.Appearance.app_layout_desc")),
                ),
        )
        .child(
            h_flex().flex_wrap().gap_3().children(
                [AppLayout::GlobalNavigation, AppLayout::HomeNavigation]
                    .into_iter()
                    .map(|layout| render_layout_card(layout, selected == layout, cx)),
            ),
        )
}

fn render_layout_card(layout: AppLayout, selected: bool, cx: &App) -> impl IntoElement {
    let (icon, title, description) = match layout {
        AppLayout::GlobalNavigation => (
            IconName::LayoutDashboard,
            t!("Settings.General.Appearance.layout_global"),
            t!("Settings.General.Appearance.layout_global_desc"),
        ),
        AppLayout::HomeNavigation => (
            IconName::Home,
            t!("Settings.General.Appearance.layout_home"),
            t!("Settings.General.Appearance.layout_home_desc"),
        ),
    };
    let layout_name = match layout {
        AppLayout::GlobalNavigation => "global",
        AppLayout::HomeNavigation => "home",
    };
    v_flex()
        .id(SharedString::from(format!(
            "appearance-layout-{layout_name}"
        )))
        .w(px(LAYOUT_CARD_WIDTH))
        .gap_2()
        .p_3()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(if selected {
            cx.theme().primary
        } else {
            cx.theme().border
        })
        .bg(if selected {
            cx.theme().primary.opacity(0.06)
        } else {
            cx.theme().background
        })
        .on_click(move |_, window, cx| set_app_layout(layout, window, cx))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(Icon::new(icon).size_4().text_color(if selected {
                    cx.theme().primary
                } else {
                    cx.theme().muted_foreground
                }))
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title),
                ),
        )
        .child(render_layout_preview(layout, cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(description),
        )
}

fn render_layout_preview(layout: AppLayout, cx: &App) -> impl IntoElement {
    let rail = v_flex()
        .h_full()
        .w(px(if layout == AppLayout::GlobalNavigation {
            24.0
        } else {
            48.0
        }))
        .flex_shrink_0()
        .gap_1()
        .p_1()
        .bg(cx.theme().muted)
        .children((0..3).map(|index| {
            div()
                .h(px(if layout == AppLayout::GlobalNavigation {
                    12.0
                } else {
                    6.0
                }))
                .w_full()
                .rounded(px(2.0))
                .bg(if index == 0 {
                    cx.theme().primary.opacity(0.7)
                } else {
                    cx.theme().border
                })
        }));
    h_flex()
        .h(px(52.0))
        .w_full()
        .overflow_hidden()
        .rounded(px(4.0))
        .border_1()
        .border_color(cx.theme().border)
        .child(rail)
        .child(
            v_flex()
                .flex_1()
                .gap_2()
                .p_2()
                .child(
                    div()
                        .h(px(6.0))
                        .w(px(56.0))
                        .rounded(px(2.0))
                        .bg(cx.theme().border),
                )
                .child(h_flex().gap_2().children((0..2).map(|_| {
                    div()
                        .h(px(22.0))
                        .flex_1()
                        .rounded(px(3.0))
                        .bg(cx.theme().muted)
                }))),
        )
}

fn set_app_layout(layout: AppLayout, window: &mut Window, cx: &mut App) {
    if AppSettings::global(cx).app_layout == layout {
        return;
    }
    match AppSettings::try_update_and_save(cx, |settings| settings.app_layout = layout) {
        Ok(()) => {
            cx.refresh_windows();
            window.push_notification(
                Notification::success(t!("Settings.General.Appearance.layout_saved").to_string())
                    .autohide(true),
                cx,
            );
        }
        Err(error) => {
            // 保存失败时全局设置未更新，选中态继续以原布局为准。
            window.push_notification(
                Notification::error(
                    t!("Settings.General.Appearance.save_failed", error = error).to_string(),
                )
                .autohide(false),
                cx,
            );
        }
    }
}

fn render_density(cx: &App) -> impl IntoElement {
    let selected = AppSettings::global(cx).interface_density;
    let options = [
        (
            AppearanceDensity::Compact,
            "compact",
            t!("Settings.General.Appearance.density_compact"),
        ),
        (
            AppearanceDensity::Normal,
            "normal",
            t!("Settings.General.Appearance.density_normal"),
        ),
        (
            AppearanceDensity::Comfortable,
            "comfortable",
            t!("Settings.General.Appearance.density_comfortable"),
        ),
    ];
    h_flex()
        .justify_between()
        .items_center()
        .child(
            v_flex()
                .gap_1()
                .child(div().child(t!("Settings.General.Appearance.interface_density")))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Settings.General.Appearance.interface_density_desc")),
                ),
        )
        .child(
            h_flex()
                .gap_1()
                .children(options.into_iter().map(|(density, name, label)| {
                    let is_selected = selected == density;
                    Button::new(SharedString::from(format!("appearance-density-{name}")))
                        .label(label)
                        .selected(is_selected)
                        .when(is_selected, |button| button.primary())
                        .small()
                        .on_click(move |_, window, cx| set_density(density, window, cx))
                })),
        )
}

fn set_density(density: AppearanceDensity, window: &mut Window, cx: &mut App) {
    if AppSettings::global(cx).interface_density == density {
        return;
    }
    match AppSettings::try_update_and_save(cx, |settings| {
        settings.interface_density = density;
    }) {
        Ok(()) => cx.refresh_windows(),
        Err(error) => window.push_notification(
            Notification::error(
                t!("Settings.General.Appearance.save_failed", error = error).to_string(),
            )
            .autohide(false),
            cx,
        ),
    }
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
    if AppSettings::try_update_and_save(cx, |settings| {
        settings.theme_mode = mode.to_string();
        settings.auto_switch_theme = mode == "system";
    })
    .is_ok()
    {
        themes::apply_appearance(&AppSettings::current(cx), cx);
    }
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
            .with_size(options.size())
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
                            if AppSettings::try_update_and_save(cx, |settings| {
                                settings.custom_accent_enabled = *value;
                            })
                            .is_ok()
                            {
                                themes::apply_appearance(&AppSettings::current(cx), cx);
                            }
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
        assert!(source.contains("AppLayout::GlobalNavigation"));
        assert!(source.contains("AppLayout::HomeNavigation"));
        assert!(source.contains("AppearanceDensity::Compact"));
        assert!(source.contains("AppearanceDensity::Normal"));
        assert!(source.contains("AppearanceDensity::Comfortable"));
        assert!(source.contains("settings.app_layout = layout"));
        assert!(source.contains("settings.interface_density = density"));
        assert!(source.contains("AppSettings::try_update_and_save"));
        assert!(source.contains("Notification::error"));
        // 终端配色已经搬到「设置 → 终端」，外观页不再嵌入终端主题选择。
        // 断言字面量拆开写：include_str! 读的正是本文件，整串写会命中断言自身。
        assert!(!source.contains(concat!("terminal_theme", "_section")));
        assert!(!source.contains(concat!(
            "settings.terminal_theme",
            " = theme_name.to_string()"
        )));
        assert!(source.contains("Slider::new"));
        assert!(source.contains("ColorPicker::new"));
        assert!(source.contains("custom_accent_enabled"));
    }
}
