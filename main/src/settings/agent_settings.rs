use agent_runtime::{
    DEFAULT_AGENT_MAX_ITERATIONS, MAX_AGENT_MAX_ITERATIONS, MIN_AGENT_MAX_ITERATIONS,
};
use gpui::App;
use gpui_component::setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem};
use one_core::settings::{AiChatSettings, AppSettings};
use rust_i18n::t;

pub fn agent_setting_group(default_settings: &AiChatSettings) -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.General.Agent.group_title"))
        .item(
            SettingItem::new(
                t!("Settings.General.Agent.max_iterations"),
                SettingField::number_input(
                    NumberFieldOptions {
                        min: MIN_AGENT_MAX_ITERATIONS as f64,
                        max: MAX_AGENT_MAX_ITERATIONS as f64,
                        step: 1.0,
                    },
                    |cx: &App| AppSettings::global(cx).ai_chat.max_iterations as f64,
                    |value: f64, cx: &mut App| {
                        AppSettings::update_and_save(cx, |settings| {
                            settings.ai_chat.max_iterations = normalize_max_iterations(value);
                        });
                    },
                )
                .default_value(default_settings.max_iterations as f64),
            )
            .description(t!("Settings.General.Agent.max_iterations_desc").to_string()),
        )
}

fn normalize_max_iterations(value: f64) -> usize {
    if !value.is_finite() {
        return DEFAULT_AGENT_MAX_ITERATIONS;
    }
    (value.round() as usize).clamp(MIN_AGENT_MAX_ITERATIONS, MAX_AGENT_MAX_ITERATIONS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_iterations_are_rounded_and_clamped_to_runtime_bounds() {
        assert_eq!(MIN_AGENT_MAX_ITERATIONS, normalize_max_iterations(0.0));
        assert_eq!(MIN_AGENT_MAX_ITERATIONS, normalize_max_iterations(1.0));
        assert_eq!(65, normalize_max_iterations(64.6));
        assert_eq!(
            MAX_AGENT_MAX_ITERATIONS,
            normalize_max_iterations(MAX_AGENT_MAX_ITERATIONS as f64)
        );
        assert_eq!(
            MAX_AGENT_MAX_ITERATIONS,
            normalize_max_iterations((MAX_AGENT_MAX_ITERATIONS + 1) as f64)
        );
        assert_eq!(
            DEFAULT_AGENT_MAX_ITERATIONS,
            normalize_max_iterations(f64::NAN)
        );
    }
}
