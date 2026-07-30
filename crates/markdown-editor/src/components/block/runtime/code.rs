//! Code-block runtime cache management.

use super::*;
use crate::{
    CodeHighlightRequest, CodeHighlightResult as HostCodeHighlightResult, CodeHighlightService,
    CodeHighlightSpan as HostCodeHighlightSpan,
};

fn normalize_code_language_input(text: &str) -> String {
    text.replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
        .trim()
        .to_string()
}

impl Block {
    pub(crate) fn code_highlight_result(&self) -> Option<&CodeHighlightResult> {
        self.code_highlight.as_ref()
    }

    pub(super) fn sync_code_highlight(&mut self) {
        let BlockKind::CodeBlock { language } = &self.record.kind else {
            self.code_highlight = None;
            self.code_highlight_registry_revision = None;
            return;
        };
        let source = self.render_cache.visible_text();
        let service = self.host_services.code_highlighter();
        self.code_highlight_registry_revision = service.map(CodeHighlightService::revision);
        self.code_highlight =
            service.and_then(|service| highlight_with_host(service, language.as_deref(), source));
    }

    pub(crate) fn sync_code_highlight_registry_revision(&mut self) {
        let revision = self
            .host_services
            .code_highlighter()
            .map(CodeHighlightService::revision);
        if revision != self.code_highlight_registry_revision {
            self.sync_code_highlight();
        }
    }

    pub(crate) fn code_language_text(&self) -> &str {
        match &self.record.kind {
            BlockKind::CodeBlock {
                language: Some(language),
            } => language.as_ref(),
            _ => "",
        }
    }

    pub(crate) fn replace_code_language_text_in_range(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        _selected_range_relative: Option<Range<usize>>,
        _mark_inserted_text: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.kind().is_code_block() {
            return;
        }

        self.prepare_undo_capture(UndoCaptureKind::CoalescibleText, cx);

        let current = self.code_language_text().to_string();
        let range = range.start.min(current.len())..range.end.min(current.len());
        let inserted = new_text.replace("\r\n", " ").replace(['\r', '\n'], " ");
        let mut raw_next = String::new();
        raw_next.push_str(&current[..range.start]);
        raw_next.push_str(&inserted);
        raw_next.push_str(&current[range.end..]);

        let normalized = normalize_code_language_input(&raw_next);

        let old_language = match &self.record.kind {
            BlockKind::CodeBlock { language } => language.clone(),
            _ => None,
        };
        self.record.kind = BlockKind::CodeBlock {
            language: (!normalized.is_empty()).then(|| SharedString::from(normalized)),
        };
        self.sync_code_highlight();

        let next_language = match &self.record.kind {
            BlockKind::CodeBlock { language } => language.clone(),
            _ => None,
        };
        if old_language != next_language {
            cx.emit(BlockEvent::Changed);
        }
        cx.notify();
    }

    /// Replaces the complete fenced-code language from the header menu.
    ///
    /// The menu and the former inline language input intentionally share this
    /// mutation path so undo capture, host/WASM highlighting refresh, dirty
    /// propagation, and unknown language preservation stay identical.
    pub(crate) fn set_code_language(&mut self, language: &str, cx: &mut Context<Self>) {
        let range = 0..self.code_language_text().len();
        self.replace_code_language_text_in_range(range, language, None, false, cx);
    }
}

fn highlight_with_host(
    service: &CodeHighlightService,
    language: Option<&str>,
    source: &str,
) -> Option<CodeHighlightResult> {
    let request = CodeHighlightRequest {
        language: language.map(str::to_owned),
        source: source.to_owned(),
    };
    let result = service.highlight(request).ok()?;
    Some(host_highlight_result(language, source, result))
}

fn host_highlight_result(
    language: Option<&str>,
    source: &str,
    result: HostCodeHighlightResult,
) -> CodeHighlightResult {
    CodeHighlightResult {
        language: resolve_code_language_key(language).unwrap_or(CodeLanguageKey::PlainText),
        spans: normalize_host_highlight_spans(result.spans, source),
    }
}

fn normalize_host_highlight_spans(
    spans: Vec<HostCodeHighlightSpan>,
    source: &str,
) -> Vec<CodeHighlightSpan> {
    let mut spans = spans
        .into_iter()
        .filter_map(|span| normalize_host_highlight_span(span, source))
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| (span.range.start, span.range.end));
    make_host_highlight_spans_disjoint(spans)
}

fn normalize_host_highlight_span(
    span: HostCodeHighlightSpan,
    source: &str,
) -> Option<HostCodeHighlightSpan> {
    let range = span.range.start.min(source.len())..span.range.end.min(source.len());
    (range.start < range.end
        && source.is_char_boundary(range.start)
        && source.is_char_boundary(range.end))
    .then_some(HostCodeHighlightSpan {
        range,
        style: span.style,
    })
}

fn make_host_highlight_spans_disjoint(spans: Vec<HostCodeHighlightSpan>) -> Vec<CodeHighlightSpan> {
    let mut covered_until = 0usize;
    let mut normalized = Vec::with_capacity(spans.len());
    for span in spans {
        let start = span.range.start.max(covered_until);
        if start >= span.range.end {
            continue;
        }
        covered_until = span.range.end;
        normalized.push(CodeHighlightSpan {
            range: start..span.range.end,
            paint: CodeHighlightPaint::Host(span.style),
        });
    }
    normalized
}
