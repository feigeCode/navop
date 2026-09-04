use crate::card::{CardMessage, ChatCard};
use crate::cards::ChartJsonCard;
use crate::code_block::CodeBlockActionRegistry;
use crate::html_code_block::HtmlCodeBlockView;
use crate::parse_chart_json_block;
use crate::theme::{
    AgentChatTheme, active_agent_chat_theme, themed_markdown, with_agent_chat_theme,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use gpui::{
    AnyElement, App, Entity, IntoElement, ParentElement, SharedString, Styled, Window, div,
};
use gpui_base::TextView;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{
    Sizable,
    clipboard::Clipboard,
    h_flex,
    text::{MarkdownNode, MarkdownParseContext, markdown_ast},
    v_flex,
};
use html_preview::HtmlPreviewDocument;
use rust_i18n::t;

const COPY_CODE_ACTION_ID: &str = "copy-code";
const HTML_DOWNLOAD_ACTION_ID: &str = "html-download";
const HTML_OPEN_BROWSER_ACTION_ID: &str = "html-open-browser";
const HTML_PREVIEW_ACTION_ID: &str = "html-preview";
const RICH_CODE_BLOCK_NODE: &str = "agent-rich-code-block";

#[derive(Clone, Copy)]
enum RichCodeBlockRender {
    Html,
    Chart,
}

#[derive(Clone)]
struct RichCodeBlockData {
    code: String,
    lang: Option<String>,
    markdown: String,
    source_offset: usize,
    render: RichCodeBlockRender,
}

pub(crate) fn apply_code_block_features(
    text_view: TextView,
    registry: Option<&CodeBlockActionRegistry>,
    theme: Option<&AgentChatTheme>,
    is_streaming: bool,
) -> TextView {
    let toolbar_registry = registry.cloned();
    let toolbar_theme = theme.cloned();
    let renderer_registry = registry.cloned();
    let renderer_theme = theme.cloned();
    text_view
        .code_block_actions(move |block, window, cx| {
            let theme = toolbar_theme
                .clone()
                .unwrap_or_else(|| active_agent_chat_theme(cx));
            let code = block.code();
            let lang = block.lang();
            render_code_block_toolbar(
                code_block_key(0, code.as_ref(), lang.as_deref()),
                code,
                lang,
                toolbar_registry.as_ref(),
                &theme,
                is_streaming,
                window,
                cx,
            )
        })
        .markdown_block_parser(move |node, cx| parse_rich_code_block(node, cx, is_streaming))
        .markdown_block_renderer(RICH_CODE_BLOCK_NODE, move |node, window, cx| {
            render_rich_code_block(
                node,
                renderer_registry.as_ref(),
                renderer_theme.as_ref(),
                window,
                cx,
            )
        })
}

fn render_code_block_toolbar(
    block_key: u64,
    code: SharedString,
    lang: Option<SharedString>,
    registry: Option<&CodeBlockActionRegistry>,
    theme: &AgentChatTheme,
    is_streaming: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    if is_html_code_block(lang.as_deref()) {
        return render_html_code_block_toolbar(
            block_key,
            code,
            lang.as_deref(),
            theme,
            is_streaming,
            window,
            cx,
        );
    }

    let copy_id = SharedString::from(format!(
        "{COPY_CODE_ACTION_ID}-{}-{}",
        lang.as_deref().unwrap_or("text"),
        code.len()
    ));
    let mut row = h_flex().gap_1().text_color(theme.code_foreground).child(
        Clipboard::new(copy_id)
            .value(code.clone())
            .tooltip(t!("AgentUi.copy_code").to_string()),
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

fn parse_rich_code_block(
    node: &markdown_ast::Node,
    cx: &MarkdownParseContext<'_>,
    is_streaming: bool,
) -> Option<MarkdownNode> {
    let markdown_ast::Node::Code(block) = node else {
        return None;
    };
    let lang = block.lang.as_deref();
    let render = if should_render_html_preview(&block.value, lang, is_streaming) {
        RichCodeBlockRender::Html
    } else if is_renderable_chart_code_block(&block.value, lang) {
        RichCodeBlockRender::Chart
    } else {
        return None;
    };
    let source_offset = cx.offset() + block.position.as_ref()?.start.offset;
    let markdown = cx.node_source(node).unwrap_or_default().to_string();
    let data = RichCodeBlockData {
        code: block.value.clone(),
        lang: block.lang.clone(),
        markdown: markdown.clone(),
        source_offset,
        render,
    };
    Some(
        MarkdownNode::new(RICH_CODE_BLOCK_NODE, data)
            .text(block.value.clone())
            .markdown(markdown),
    )
}

fn render_rich_code_block(
    node: &MarkdownNode,
    registry: Option<&CodeBlockActionRegistry>,
    configured_theme: Option<&AgentChatTheme>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(data) = node.data::<RichCodeBlockData>() else {
        return div().into_any_element();
    };
    let theme = configured_theme
        .cloned()
        .unwrap_or_else(|| active_agent_chat_theme(cx));
    match data.render {
        RichCodeBlockRender::Html => render_html_code_block(data, registry, &theme, window, cx),
        RichCodeBlockRender::Chart => {
            with_agent_chat_theme(&theme, || render_chart_code_block(&data.code, window, cx))
        }
    }
}

fn code_block_key(source_offset: usize, code: &str, lang: Option<&str>) -> u64 {
    let mut hasher = DefaultHasher::new();
    source_offset.hash(&mut hasher);
    code.hash(&mut hasher);
    lang.unwrap_or("text")
        .to_ascii_lowercase()
        .hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
fn code_block_toolbar_action_ids(
    lang: Option<&str>,
    registry: Option<&CodeBlockActionRegistry>,
    is_streaming: bool,
) -> Vec<String> {
    if is_html_code_block(lang) {
        let mut ids = vec![COPY_CODE_ACTION_ID.to_string()];
        if !is_streaming {
            ids.extend([
                HTML_DOWNLOAD_ACTION_ID.to_string(),
                HTML_OPEN_BROWSER_ACTION_ID.to_string(),
                HTML_PREVIEW_ACTION_ID.to_string(),
            ]);
        }
        return ids;
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

fn should_render_html_preview(code: &str, lang: Option<&str>, is_streaming: bool) -> bool {
    !is_streaming && is_renderable_html_code_block(code, lang)
}

fn html_preview_view_state_id(block_key: u64, code: &str, lang: Option<&str>) -> SharedString {
    let mut hasher = DefaultHasher::new();
    block_key.hash(&mut hasher);
    code.hash(&mut hasher);
    lang.unwrap_or("html")
        .to_ascii_lowercase()
        .hash(&mut hasher);
    SharedString::from(format!("html-code-block-view-{:016x}", hasher.finish()))
}

fn html_preview_state(
    block_key: u64,
    code: &str,
    lang: Option<&str>,
    window: &mut Window,
    cx: &mut App,
) -> Option<(SharedString, Entity<HtmlCodeBlockView>)> {
    let document = html_preview_document_for_block(code, lang)?;
    let state_id = html_preview_view_state_id(block_key, code, lang);
    let preview = window.use_keyed_state(state_id.clone(), cx, |window, cx| {
        HtmlCodeBlockView::new(state_id.clone(), document.clone(), window, cx)
    });
    Some((state_id, preview))
}

fn render_html_code_block_toolbar(
    block_key: u64,
    code: SharedString,
    lang: Option<&str>,
    theme: &AgentChatTheme,
    is_streaming: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let copy_id = SharedString::from(format!("{COPY_CODE_ACTION_ID}-html-{}", code.len()));
    let row = h_flex().gap_1().text_color(theme.code_foreground).child(
        Clipboard::new(copy_id)
            .value(code.clone())
            .tooltip(t!("HtmlPreview.copy_html").to_string()),
    );
    if is_streaming {
        return row.into_any_element();
    }
    let Some((state_id, preview)) = html_preview_state(block_key, code.as_ref(), lang, window, cx)
    else {
        return row.into_any_element();
    };

    row.child(html_download_button(&state_id, &preview))
        .child(html_open_browser_button(&state_id, &preview))
        .child(html_preview_button(&state_id, &preview))
        .into_any_element()
}

fn html_download_button(state_id: &SharedString, preview: &Entity<HtmlCodeBlockView>) -> Button {
    let preview = preview.clone();
    Button::new(SharedString::from(format!(
        "{state_id}-{HTML_DOWNLOAD_ACTION_ID}"
    )))
    .icon(gpui_component::IconName::ArrowDown)
    .ghost()
    .xsmall()
    .tooltip(t!("HtmlPreview.download_html").to_string())
    .on_click(move |_, _, cx| {
        preview.update(cx, |preview, cx| preview.download_html(cx));
    })
}

fn html_open_browser_button(
    state_id: &SharedString,
    preview: &Entity<HtmlCodeBlockView>,
) -> Button {
    let preview = preview.clone();
    Button::new(SharedString::from(format!(
        "{state_id}-{HTML_OPEN_BROWSER_ACTION_ID}"
    )))
    .icon(gpui_component::IconName::ExternalLink)
    .ghost()
    .xsmall()
    .tooltip(t!("HtmlPreview.open_browser").to_string())
    .on_click(move |_, _, cx| {
        preview.update(cx, |preview, cx| preview.open_in_browser(cx));
    })
}

fn html_preview_button(state_id: &SharedString, preview: &Entity<HtmlCodeBlockView>) -> Button {
    let preview = preview.clone();
    Button::new(SharedString::from(format!(
        "{state_id}-{HTML_PREVIEW_ACTION_ID}"
    )))
    .icon(gpui_component::IconName::Eye)
    .ghost()
    .xsmall()
    .tooltip(t!("HtmlPreview.open_dialog").to_string())
    .on_click(move |_, window, cx| {
        preview.update(cx, |preview, cx| preview.open_preview_dialog(window, cx));
    })
}

fn is_renderable_chart_code_block(code: &str, lang: Option<&str>) -> bool {
    parse_chart_json_block(code, lang).is_some()
}

fn render_html_code_block(
    data: &RichCodeBlockData,
    registry: Option<&CodeBlockActionRegistry>,
    theme: &AgentChatTheme,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let block_key = code_block_key(data.source_offset, &data.code, data.lang.as_deref());
    let Some((_, preview)) =
        html_preview_state(block_key, &data.code, data.lang.as_deref(), window, cx)
    else {
        return div().into_any_element();
    };
    let markdown = if data.markdown.is_empty() {
        format!(
            "```{}\n{}\n```",
            data.lang.as_deref().unwrap_or(""),
            data.code
        )
    } else {
        data.markdown.clone()
    };
    v_flex()
        .gap_2()
        .child(
            themed_markdown(
                SharedString::from(format!("agent-html-code-{block_key}")),
                markdown,
                theme,
            )
            .selectable(true),
        )
        .child(render_code_block_toolbar(
            block_key,
            data.code.clone().into(),
            data.lang.clone().map(Into::into),
            registry,
            theme,
            false,
            window,
            cx,
        ))
        .child(preview)
        .into_any_element()
}

fn render_chart_code_block(code: &str, window: &mut Window, cx: &mut App) -> AnyElement {
    let msg = CardMessage {
        id: "chart-code-block",
        kind: "chart-json",
        content: code,
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
            code_block_toolbar_action_ids(Some("sql"), Some(&registry), false)
        );
        assert_eq!(
            vec!["copy-code"],
            code_block_toolbar_action_ids(Some("rust"), Some(&registry), false)
        );
    }

    #[test]
    fn html_code_block_uses_inner_toolbar_only() {
        assert_eq!(
            vec![
                "copy-code",
                "html-download",
                "html-open-browser",
                "html-preview"
            ],
            code_block_toolbar_action_ids(Some("html"), None, false)
        );
        assert_eq!(
            vec!["copy-code"],
            code_block_toolbar_action_ids(Some("HTML"), None, true)
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
    fn html_preview_waits_until_message_streaming_finishes() {
        assert!(should_render_html_preview(
            "<main>Done</main>",
            Some("html"),
            false
        ));
        assert!(!should_render_html_preview(
            "<main>Partial",
            Some("html"),
            true
        ));
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
        let first = html_preview_view_state_id(1, "<main>A</main>", Some("HTML"));
        let same = html_preview_view_state_id(1, "<main>A</main>", Some("html"));
        let changed_content = html_preview_view_state_id(1, "<main>B</main>", Some("html"));
        let changed_message = html_preview_view_state_id(2, "<main>A</main>", Some("html"));

        assert_eq!(first, same);
        assert_ne!(first, changed_content);
        assert_ne!(first, changed_message);
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
