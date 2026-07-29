use std::sync::Arc;

use crate::{
    CodeHighlightProvider, CodeHighlightRequest, CodeHighlightResult, CodeHighlightService,
    CodeHighlightSpan, CodeHighlightStyle,
};
use gpui_component::highlighter::{HighlightTheme, LanguageRegistry, SyntaxHighlighter};
use ropey::Rope;

pub(crate) fn code_highlight_service(theme: Arc<HighlightTheme>) -> CodeHighlightService {
    let provider: CodeHighlightProvider =
        Arc::new(move |request| highlight_request(request, &theme));
    CodeHighlightService::new(provider, Arc::new(registry_revision))
}

fn highlight_request(
    request: CodeHighlightRequest,
    theme: &HighlightTheme,
) -> Result<CodeHighlightResult, String> {
    let language = resolve_language(request.language.as_deref());
    let rope = Rope::from_str(&request.source);
    let mut highlighter = SyntaxHighlighter::new(&language);
    highlighter.update(None, &rope, None);

    let mut spans = highlighter
        .styles(&(0..request.source.len()), theme)
        .into_iter()
        .filter_map(|(range, style)| {
            valid_range(range, &request.source).map(|range| CodeHighlightSpan {
                range,
                style: CodeHighlightStyle {
                    color: style.color,
                    font_weight: style.font_weight,
                    font_style: style.font_style,
                },
            })
        })
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| (span.range.start, span.range.end));
    Ok(CodeHighlightResult { spans })
}

fn resolve_language(language: Option<&str>) -> String {
    let identifier = language
        .and_then(|value| value.split_whitespace().next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("text");

    LanguageRegistry::singleton()
        .resolve_language_name(identifier)
        .unwrap_or_else(|| "text".to_owned())
}

fn valid_range(range: std::ops::Range<usize>, source: &str) -> Option<std::ops::Range<usize>> {
    let range = range.start.min(source.len())..range.end.min(source.len());
    (range.start < range.end
        && source.is_char_boundary(range.start)
        && source.is_char_boundary(range.end))
    .then_some(range)
}

#[cfg(not(target_family = "wasm"))]
fn registry_revision() -> u64 {
    LanguageRegistry::singleton().revision()
}

#[cfg(target_family = "wasm")]
fn registry_revision() -> u64 {
    0
}
