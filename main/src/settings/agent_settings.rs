use agent_runtime::{
    DEFAULT_AGENT_MAX_ITERATIONS, MAX_AGENT_MAX_ITERATIONS, MIN_AGENT_MAX_ITERATIONS,
};
use gpui::{App, AppContext, Context, Entity, IntoElement, ParentElement, Styled, Window};
use gpui_component::input::{InputEvent, Textarea, TextareaState};
use gpui_component::setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem};
use gpui_component::{ActiveTheme, v_flex};
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
        .item(custom_system_prompt_item())
}

fn custom_system_prompt_item() -> SettingItem {
    SettingItem::render(|_options, window, cx| render_custom_system_prompt(window, cx)).keywords([
        t!("Settings.General.Agent.custom_system_prompt").to_string(),
        t!("Settings.General.Agent.custom_system_prompt_desc").to_string(),
    ])
}

struct CustomSystemPromptEditor {
    input: Entity<TextareaState>,
    _subscription: gpui::Subscription,
}

fn render_custom_system_prompt(window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let editor = window.use_keyed_state("agent-custom-system-prompt", cx, |window, cx| {
        CustomSystemPromptEditor::new(window, cx)
    });
    editor.into_any_element()
}

impl CustomSystemPromptEditor {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(4, 12)
                .placeholder(t!("Settings.General.Agent.custom_system_prompt_placeholder"))
                .default_value(AppSettings::global(cx).ai_chat.custom_system_prompt.clone())
        });
        let subscription = cx.subscribe(&input, |editor: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let value = editor.input.read(cx).value().to_string();
                AppSettings::update_and_save(cx, |settings| {
                    settings.ai_chat.custom_system_prompt = value;
                });
                cx.notify();
            }
        });
        Self {
            input,
            _subscription: subscription,
        }
    }
}

impl gpui::Render for CustomSystemPromptEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        v_flex()
            .w_full()
            .max_w(gpui::px(640.))
            .gap_2()
            .child(
                gpui::div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Settings.General.Agent.custom_system_prompt_desc").to_string()),
            )
            .child(Textarea::new(&self.input))
    }
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
