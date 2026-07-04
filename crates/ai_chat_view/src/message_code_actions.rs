use crate::card::{CardMessage, ChatCard};
use crate::cards::ChartJsonCard;
use crate::code_block::CodeBlockActionRegistry;
use crate::parse_chart_json_block;
use crate::theme::{AgentChatTheme, active_agent_chat_theme, with_agent_chat_theme};
use gpui::{AnyElement, App, IntoElement, ParentElement, SharedString, Styled, Window};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{
    Sizable,
    clipboard::Clipboard,
    h_flex,
    text::{CodeBlock, TextView},
};

const COPY_CODE_ACTION_ID: &str = "copy-code";

pub(crate) fn apply_code_block_features(
    text_view: TextView,
    registry: Option<&CodeBlockActionRegistry>,
    theme: Option<&AgentChatTheme>,
) -> TextView {
    let toolbar_registry = registry.cloned();
    let toolbar_theme = theme.cloned();
    let renderer_theme = theme.cloned();
    text_view
        .code_block_actions(move |block, _window, cx| {
            let theme = toolbar_theme
                .clone()
                .unwrap_or_else(|| active_agent_chat_theme(cx));
            render_code_block_toolbar(block, toolbar_registry.as_ref(), &theme, cx)
        })
        .code_block_renderer(move |block, _options, default, window, cx| {
            if is_renderable_chart_code_block(block.code().as_ref(), block.lang().as_deref()) {
                let theme = renderer_theme
                    .clone()
                    .unwrap_or_else(|| active_agent_chat_theme(cx));
                with_agent_chat_theme(&theme, || render_chart_code_block(block, window, cx))
            } else {
                default
            }
        })
}

fn render_code_block_toolbar(
    block: &CodeBlock,
    registry: Option<&CodeBlockActionRegistry>,
    theme: &AgentChatTheme,
    _cx: &App,
) -> AnyElement {
    let code = block.code();
    let lang = block.lang();
    let copy_id = SharedString::from(format!(
        "{COPY_CODE_ACTION_ID}-{}-{}",
        lang.as_deref().unwrap_or("text"),
        code.len()
    ));
    let mut row = h_flex().gap_1().text_color(theme.code_foreground).child(
        Clipboard::new(copy_id)
            .value(code.clone())
            .tooltip("复制代码"),
    );

    if let Some(registry) = registry {
        for (idx, action) in registry
            .get_actions_for_lang(lang.as_deref())
            .into_iter()
            .enumerate()
        {
            let callback = action.callback.clone();
            let action_code = code.to_string();
            let action_lang = lang.as_ref().map(ToString::to_string);
            let mut button = Button::new(SharedString::from(format!("{}-{idx}", action.id)))
                .icon(action.icon.clone())
                .ghost()
                .xsmall()
                .on_click(move |_, window, cx| {
                    callback(action_code.clone(), action_lang.clone(), window, cx);
                });
            if let Some(label) = &action.label {
                button = button.tooltip(label.clone());
            }
            row = row.child(button);
        }
    }
    row.into_any_element()
}

#[cfg(test)]
fn code_block_toolbar_action_ids(
    lang: Option<&str>,
    registry: Option<&CodeBlockActionRegistry>,
) -> Vec<String> {
    std::iter::once(COPY_CODE_ACTION_ID.to_string())
        .chain(
            registry
                .into_iter()
                .flat_map(|r| r.get_actions_for_lang(lang))
                .map(|action| action.id.to_string()),
        )
        .collect()
}

fn is_renderable_chart_code_block(code: &str, lang: Option<&str>) -> bool {
    parse_chart_json_block(code, lang).is_some()
}

fn render_chart_code_block(block: &CodeBlock, window: &mut Window, cx: &mut App) -> AnyElement {
    let code = block.code();
    let msg = CardMessage {
        id: "chart-code-block",
        kind: "chart-json",
        content: code.as_ref(),
        is_streaming: false,
    };
    ChartJsonCard.render(&msg, window, cx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_block::{CodeBlockAction, LanguageMatcher};

    #[test]
    fn code_block_toolbar_ids_always_include_copy_before_custom_actions() {
        let action = CodeBlockAction::new("run-sql")
            .matcher(LanguageMatcher::sql())
            .on_click(|_, _, _, _| {})
            .build()
            .expect("action should build");
        let mut registry = CodeBlockActionRegistry::new();
        registry.register(action);

        assert_eq!(
            vec!["copy-code", "run-sql"],
            code_block_toolbar_action_ids(Some("sql"), Some(&registry))
        );
        assert_eq!(
            vec!["copy-code"],
            code_block_toolbar_action_ids(Some("rust"), Some(&registry))
        );
    }

    #[test]
    fn chart_code_block_detection_requires_supported_language_and_valid_data() {
        let chart = r#"{"chart_type":"bar","data":[{"x":"Jan","y":3}]}"#;

        assert!(is_renderable_chart_code_block(chart, Some("chart-json")));
        assert!(is_renderable_chart_code_block(chart, Some("json")));
        assert!(!is_renderable_chart_code_block(
            r#"{"hello":"world"}"#,
            Some("json")
        ));
        assert!(!is_renderable_chart_code_block(chart, Some("rust")));
    }
}
