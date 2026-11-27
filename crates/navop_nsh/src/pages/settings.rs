use nsh_base::{input, switch, theme};
use nsh_core::prelude::*;

use crate::{services::NavopNshServices, ui};

pub(crate) fn settings() -> impl IntoElement {
    let services = NavopNshServices::current();
    let base = services
        .as_ref()
        .map(|services| services.settings.clone())
        .unwrap_or_default();
    let version = services
        .as_ref()
        .map(|services| services.version.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    let data_dir = services
        .as_ref()
        .and_then(|services| services.data_dir.as_ref().map(ToString::to_string))
        .unwrap_or_else(|| "unavailable".to_owned());

    let locale = signal(base.locale.clone());
    let theme_mode = signal(base.theme_mode.clone());
    let auto_switch_theme = signal(base.auto_switch_theme);
    let font_size = signal(base.font_size.to_string());
    let ui_scale_percent = signal(base.ui_scale_percent.to_string());
    let terminal_font_size = signal(base.terminal_font_size.to_string());
    let terminal_scrollback = signal(base.terminal_scrollback_lines.to_string());
    let terminal_auto_copy = signal(base.terminal_auto_copy);
    let terminal_auto_session_logging = signal(base.terminal_auto_session_logging);
    let auto_update = signal(base.auto_update);
    let sync_enabled = signal(base.sync_enabled);
    let status = signal(String::from(
        "Settings loaded from the shared Navop configuration",
    ));
    let saving = signal(false);

    let save = {
        let base = base.clone();
        move || {
            saving.set(true);
            let mut next = base.clone();
            let parsed = parse_settings_fields(
                &font_size.get_untracked(),
                &ui_scale_percent.get_untracked(),
                &terminal_font_size.get_untracked(),
                &terminal_scrollback.get_untracked(),
            );
            match parsed {
                Ok((font, scale, terminal_font, scrollback)) => {
                    next.locale = locale.get_untracked();
                    next.theme_mode = theme_mode.get_untracked();
                    next.auto_switch_theme = auto_switch_theme.get_untracked();
                    next.font_size = font;
                    next.ui_scale_percent = scale;
                    next.terminal_font_size = terminal_font;
                    next.terminal_scrollback_lines = scrollback;
                    next.terminal_auto_copy = terminal_auto_copy.get_untracked();
                    next.terminal_auto_session_logging =
                        terminal_auto_session_logging.get_untracked();
                    next.auto_update = auto_update.get_untracked();
                    next.sync_enabled = sync_enabled.get_untracked();
                    next.normalize_font_settings();
                    next.normalize_appearance_settings();
                    next.normalize_terminal_settings();
                    match next.try_save() {
                        Ok(()) => status.set(
                            "Settings saved. Restart to apply every desktop preference.".to_owned(),
                        ),
                        Err(error) => status.set(format!("Failed to save settings: {error}")),
                    }
                }
                Err(error) => status.set(error),
            }
            saving.set(false);
        }
    };

    view()
        .size_full()
        .child(ui::page_header(
            "Settings",
            "Edit the shared AppSettings file through nsh-native controls",
        ))
        .child(
            scroll().flex_1().child(
                view()
                    .p_6()
                    .gap_4()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .child(
                                text(move || status.get())
                                    .flex_1()
                                    .text_sm()
                                    .text_color(move || theme().muted_foreground),
                            )
                            .child(
                                ui::primary_button("Save settings")
                                    .disabled(move || saving.get())
                                    .on_press(save),
                            ),
                    )
                    .child(
                        ui::card()
                            .child(section_title("Appearance"))
                            .child(setting_row(
                                "Locale",
                                "Locale identifier used by both Navop and nsh.",
                                input()
                                    .value(move || locale.get())
                                    .on_change(move |value: &str| locale.set(value.to_owned()))
                                    .placeholder("en")
                                    .w(px(220.0))
                                    .h(px(34.0))
                                    .px_3()
                                    .rounded(move || theme().radius)
                                    .border_1()
                                    .border_color(move || theme().input)
                                    .bg(move || theme().background)
                                    .text_sm()
                                    .text_color(move || theme().foreground),
                            ))
                            .child(setting_row(
                                "Theme",
                                "Choose the color mode used at startup.",
                                view()
                                    .flex_row()
                                    .gap_1()
                                    .child(theme_choice("Light", "light", theme_mode))
                                    .child(theme_choice("Dark", "dark", theme_mode))
                                    .child(theme_choice("System", "system", theme_mode)),
                            ))
                            .child(setting_row(
                                "Follow system theme",
                                "Prefer the operating system's light or dark appearance.",
                                toggle(
                                    auto_switch_theme,
                                    "Follow the system's light or dark appearance",
                                ),
                            ))
                            .child(setting_row(
                                "Font size",
                                "Base interface font size.",
                                number_input(font_size, "14"),
                            ))
                            .child(setting_row(
                                "UI scale",
                                "Whole-interface scale percentage (100 = default).",
                                number_input(ui_scale_percent, "100"),
                            )),
                    )
                    .child(
                        ui::card()
                            .child(section_title("Terminal"))
                            .child(setting_row(
                                "Font size",
                                "Terminal glyph size.",
                                number_input(terminal_font_size, "14"),
                            ))
                            .child(setting_row(
                                "Scrollback",
                                "Maximum retained terminal lines.",
                                number_input(terminal_scrollback, "10000"),
                            ))
                            .child(setting_row(
                                "Auto copy",
                                "Copy selected terminal text automatically.",
                                toggle(terminal_auto_copy, "Automatically copy selected text"),
                            ))
                            .child(setting_row(
                                "Session logs",
                                "Record new terminal sessions into the shared log catalog.",
                                toggle(
                                    terminal_auto_session_logging,
                                    "Automatically record terminal sessions",
                                ),
                            )),
                    )
                    .child(
                        ui::card()
                            .child(section_title("Updates and sync"))
                            .child(setting_row(
                                "Automatic updates",
                                "Check and install updates using the existing updater configuration.",
                                toggle(auto_update, "Enable automatic updates"),
                            ))
                            .child(setting_row(
                                "Cloud sync",
                                "Allow configured personal or team synchronization providers to run.",
                                toggle(sync_enabled, "Enable cloud synchronization"),
                            )),
                    )
                    .child(
                        ui::card()
                            .child(section_title("Runtime"))
                            .child(info_row("UI", "nsh-core + nsh-base (Rust)"))
                            .child(info_row("Backend", "GPUI (nsh-platform-gpui)"))
                            .child(info_row("App version", version))
                            .child(info_row("Data directory", data_dir)),
                    ),
            ),
        )
}

fn parse_settings_fields(
    font_size: &str,
    ui_scale_percent: &str,
    terminal_font_size: &str,
    terminal_scrollback: &str,
) -> Result<(f64, u32, f64, usize), String> {
    let font = parse_positive_f64("Font size", font_size)?;
    let scale = ui_scale_percent
        .trim()
        .parse::<u32>()
        .map_err(|_| "UI scale must be a whole percentage".to_owned())?;
    if !(50..=300).contains(&scale) {
        return Err("UI scale must be between 50 and 300 percent".to_owned());
    }
    let terminal_font = parse_positive_f64("Terminal font size", terminal_font_size)?;
    let scrollback = terminal_scrollback
        .trim()
        .parse::<usize>()
        .map_err(|_| "Scrollback must be a positive whole number".to_owned())?;
    if scrollback == 0 {
        return Err("Scrollback must be greater than zero".to_owned());
    }
    Ok((font, scale, terminal_font, scrollback))
}

fn parse_positive_f64(label: &str, value: &str) -> Result<f64, String> {
    let value = value
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("{label} must be a number"))?;
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(format!("{label} must be greater than zero"))
    }
}

fn theme_choice(
    label: &'static str,
    value: &'static str,
    theme_mode: Signal<String>,
) -> impl IntoElement {
    let selected = move || theme_mode.get() == value;
    view()
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected() {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected() {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: selected(),
            ..AccessibilityState::default()
        })
        .on_press(move || theme_mode.set(value.to_owned()))
        .child(text(label).text_sm().text_color(move || theme().foreground))
}

fn number_input(value: Signal<String>, placeholder: &'static str) -> impl IntoElement {
    input()
        .value(move || value.get())
        .on_change(move |next: &str| value.set(next.to_owned()))
        .placeholder(placeholder)
        .w(px(120.0))
        .h(px(34.0))
        .px_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().input)
        .bg(move || theme().background)
        .text_sm()
        .text_color(move || theme().foreground)
}

fn toggle(checked: Signal<bool>, label: &'static str) -> impl IntoElement {
    switch()
        .checked(move || checked.get())
        .track_class("w-9 h-5 rounded-full bg-muted checked:bg-primary")
        .thumb_class("size-4 rounded-full bg-background checked:translate-x-4")
        .accessibility_label(label)
        .on_change(move |next: &bool| checked.set(*next))
        .child(
            text(move || if checked.get() { "On" } else { "Off" })
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}

fn setting_row(
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: impl IntoElement,
) -> impl IntoElement {
    let label = label.into();
    let description = description.into();
    view()
        .flex_row()
        .items_center()
        .gap_4()
        .py_2()
        .child(
            view()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    text(label)
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(description)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(control)
}

fn section_title(title: &'static str) -> impl IntoElement {
    text(title)
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().foreground)
}

fn info_row(label: impl Into<SharedString>, value: impl Into<SharedString>) -> impl IntoElement {
    let label = label.into();
    let value = value.into();
    view()
        .flex_row()
        .items_start()
        .gap_4()
        .child(
            text(label)
                .w(px(150.0))
                .flex_none()
                .text_sm()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(value)
                .flex_1()
                .text_sm()
                .text_color(move || theme().foreground),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_fields_validate_numeric_values() {
        assert!(parse_settings_fields("14", "100", "13", "10000").is_ok());
        assert!(parse_settings_fields("0", "100", "13", "10000").is_err());
        assert!(parse_settings_fields("14", "10", "13", "10000").is_err());
        assert!(parse_settings_fields("14", "100", "13", "0").is_err());
    }
}
