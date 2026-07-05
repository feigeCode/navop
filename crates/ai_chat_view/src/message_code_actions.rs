use crate::card::{CardMessage, ChatCard};
use crate::cards::ChartJsonCard;
use crate::code_block::CodeBlockActionRegistry;
use crate::html_code_block::HtmlCodeBlockView;
use crate::parse_chart_json_block;
use crate::theme::{AgentChatTheme, active_agent_chat_theme, with_agent_chat_theme};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use gpui::{AnyElement, App, IntoElement, ParentElement, SharedString, Styled, Window};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{
    Sizable,
    clipboard::Clipboard,
    h_flex,
    text::{CodeBlock, CodeBlockRenderOptions, TextView},
};
use html_preview::HtmlPreviewDocument;

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
        .code_block_renderer(move |block, options, default, window, cx| {
            if is_renderable_html_code_block(block.code().as_ref(), block.lang().as_deref()) {
                render_html_code_block(block, options, default, window, cx)
            } else if is_renderable_chart_code_block(block.code().as_ref(), block.lang().as_deref())
            {
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
    if is_html_code_block(lang.as_deref()) {
        return h_flex()
            .gap_1()
            .text_color(theme.code_foreground)
            .into_any_element();
    }

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
    if is_html_code_block(lang) {
        return Vec::new();
    }
    std::iter::once(COPY_CODE_ACTION_ID.to_string())
        .chain(
            registry
                .into_iter()
                .flat_map(|r| r.get_actions_for_lang(lang))
                .map(|action| action.id.to_string()),
        )
        .collect()
}

fn is_html_code_block(lang: Option<&str>) -> bool {
    lang.is_some_and(|lang| matches!(lang.to_ascii_lowercase().as_str(), "html" | "htm"))
}

fn is_renderable_html_code_block(code: &str, lang: Option<&str>) -> bool {
    is_html_code_block(lang) && !code.trim().is_empty()
}

fn html_preview_document_for_block(code: &str, lang: Option<&str>) -> Option<HtmlPreviewDocument> {
    is_renderable_html_code_block(code, lang)
        .then(|| HtmlPreviewDocument::new(lang.unwrap_or("html"), code))
}

fn html_preview_view_state_id(index: usize, code: &str, lang: Option<&str>) -> SharedString {
    let mut hasher = DefaultHasher::new();
    index.hash(&mut hasher);
    code.hash(&mut hasher);
    lang.unwrap_or("html")
        .to_ascii_lowercase()
        .hash(&mut hasher);
    SharedString::from(format!("html-code-block-view-{:016x}", hasher.finish()))
}

fn is_renderable_chart_code_block(code: &str, lang: Option<&str>) -> bool {
    parse_chart_json_block(code, lang).is_some()
}

fn render_html_code_block(
    block: &CodeBlock,
    options: CodeBlockRenderOptions,
    default: AnyElement,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(document) =
        html_preview_document_for_block(block.code().as_ref(), block.lang().as_deref())
    else {
        return default;
    };
    let state_id = html_preview_view_state_id(
        options.index,
        block.code().as_ref(),
        block.lang().as_deref(),
    );
    window
        .use_keyed_state(state_id.clone(), cx, |window, cx| {
            HtmlCodeBlockView::new(state_id.clone(), document.clone(), window, cx)
        })
        .into_any_element()
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
    fn html_code_block_uses_inner_toolbar_only() {
        assert_eq!(
            Vec::<String>::new(),
            code_block_toolbar_action_ids(Some("html"), None)
        );
        assert_eq!(
            Vec::<String>::new(),
            code_block_toolbar_action_ids(Some("HTML"), None)
        );
    }

    #[test]
    fn html_code_block_detection_requires_html_language() {
        assert!(is_renderable_html_code_block(
            "<h1>Hello</h1>",
            Some("html")
        ));
        assert!(is_renderable_html_code_block("<h1>Hello</h1>", Some("htm")));
        assert!(!is_renderable_html_code_block(
            "<h1>Hello</h1>",
            Some("rust")
        ));
        assert!(!is_renderable_html_code_block("", Some("html")));
    }

    #[test]
    fn html_code_block_document_normalizes_partial_markup() {
        let document = html_preview_document_for_block("<main>Partial", Some("html")).unwrap();

        assert!(
            document
                .render_html()
                .contains("<body><main>Partial</main></body>")
        );
    }

    #[test]
    fn html_preview_view_state_id_is_stable_and_tracks_content() {
        let first = html_preview_view_state_id(0, "<main>A</main>", Some("HTML"));
        let same = html_preview_view_state_id(0, "<main>A</main>", Some("html"));
        let changed = html_preview_view_state_id(0, "<main>B</main>", Some("html"));

        assert_eq!(first, same);
        assert_ne!(first, changed);
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
