use crate::card::{CardMessage, ChatCard};
use crate::cards::ChartJsonCard;
use crate::code_block::CodeBlockActionRegistry;
use crate::html_code_block::HtmlCodeBlockView;
use crate::parse_chart_json_block;
use crate::theme::{AgentChatTheme, active_agent_chat_theme, with_agent_chat_theme};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use gpui::{
    AnyElement, App, ElementId, Entity, IntoElement, ParentElement, SharedString, Styled, Window,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{
    Sizable,
    clipboard::Clipboard,
    h_flex,
    text::{CodeBlock, CodeBlockRenderOptions, TextView},
    v_flex,
};
use html_preview::HtmlPreviewDocument;

const COPY_CODE_ACTION_ID: &str = "copy-code";
const HTML_DOWNLOAD_ACTION_ID: &str = "html-download";
const HTML_OPEN_BROWSER_ACTION_ID: &str = "html-open-browser";
const HTML_PREVIEW_ACTION_ID: &str = "html-preview";

pub(crate) fn apply_code_block_features(
    text_view: TextView,
    registry: Option<&CodeBlockActionRegistry>,
    theme: Option<&AgentChatTheme>,
    is_streaming: bool,
) -> TextView {
    let text_view_id = text_view.element_id();
    let toolbar_text_view_id = text_view_id.clone();
    let toolbar_registry = registry.cloned();
    let toolbar_theme = theme.cloned();
    let renderer_theme = theme.cloned();
    text_view
        .code_block_actions(move |block, options, window, cx| {
            let theme = toolbar_theme
                .clone()
                .unwrap_or_else(|| active_agent_chat_theme(cx));
            render_code_block_toolbar(
                &toolbar_text_view_id,
                block,
                options,
                toolbar_registry.as_ref(),
                &theme,
                is_streaming,
                window,
                cx,
            )
        })
        .code_block_renderer(move |block, options, default, window, cx| {
            if should_render_html_preview(
                block.code().as_ref(),
                block.lang().as_deref(),
                is_streaming,
            ) {
                render_html_code_block(&text_view_id, block, options, default, window, cx)
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
    text_view_id: &ElementId,
    block: &CodeBlock,
    options: CodeBlockRenderOptions,
    registry: Option<&CodeBlockActionRegistry>,
    theme: &AgentChatTheme,
    is_streaming: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let code = block.code();
    let lang = block.lang();
    if is_html_code_block(lang.as_deref()) {
        return render_html_code_block_toolbar(
            text_view_id,
            options,
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

fn html_preview_view_state_id(
    text_view_id: &ElementId,
    index: usize,
    code: &str,
    lang: Option<&str>,
) -> SharedString {
    let mut hasher = DefaultHasher::new();
    text_view_id.hash(&mut hasher);
    index.hash(&mut hasher);
    code.hash(&mut hasher);
    lang.unwrap_or("html")
        .to_ascii_lowercase()
        .hash(&mut hasher);
    SharedString::from(format!("html-code-block-view-{:016x}", hasher.finish()))
}

fn html_preview_state(
    text_view_id: &ElementId,
    options: CodeBlockRenderOptions,
    code: &str,
    lang: Option<&str>,
    window: &mut Window,
    cx: &mut App,
) -> Option<(SharedString, Entity<HtmlCodeBlockView>)> {
    let document = html_preview_document_for_block(code, lang)?;
    let state_id = html_preview_view_state_id(text_view_id, options.index, code, lang);
    let preview = window.use_keyed_state(state_id.clone(), cx, |window, cx| {
        HtmlCodeBlockView::new(state_id.clone(), document.clone(), window, cx)
    });
    Some((state_id, preview))
}

fn render_html_code_block_toolbar(
    text_view_id: &ElementId,
    options: CodeBlockRenderOptions,
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
            .tooltip("复制 HTML"),
    );
    if is_streaming {
        return row.into_any_element();
    }
    let Some((state_id, preview)) =
        html_preview_state(text_view_id, options, code.as_ref(), lang, window, cx)
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
    .tooltip("下载 HTML")
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
    .tooltip("在浏览器打开")
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
    .tooltip("切换预览")
    .on_click(move |_, _, cx| {
        preview.update(cx, |preview, cx| preview.toggle_preview(cx));
    })
}

fn is_renderable_chart_code_block(code: &str, lang: Option<&str>) -> bool {
    parse_chart_json_block(code, lang).is_some()
}

fn render_html_code_block(
    text_view_id: &ElementId,
    block: &CodeBlock,
    options: CodeBlockRenderOptions,
    default: AnyElement,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some((_, preview)) = html_preview_state(
        text_view_id,
        options,
        block.code().as_ref(),
        block.lang().as_deref(),
        window,
        cx,
    ) else {
        return default;
    };
    v_flex()
        .gap_2()
        .child(default)
        .child(preview)
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
        let first = html_preview_view_state_id(
            &ElementId::Name("msg-a".into()),
            0,
            "<main>A</main>",
            Some("HTML"),
        );
        let same = html_preview_view_state_id(
            &ElementId::Name("msg-a".into()),
            0,
            "<main>A</main>",
            Some("html"),
        );
        let changed_content = html_preview_view_state_id(
            &ElementId::Name("msg-a".into()),
            0,
            "<main>B</main>",
            Some("html"),
        );
        let changed_message = html_preview_view_state_id(
            &ElementId::Name("msg-b".into()),
            0,
            "<main>A</main>",
            Some("html"),
        );

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
