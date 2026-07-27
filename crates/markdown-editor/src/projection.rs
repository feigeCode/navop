use markdown_source::{SourceMarkdownDocument, SourceNodeId};
use std::ops::Range;

mod block_syntax;
use block_syntax::block_hidden_ranges;
mod syntax;
use syntax::{block_inline_nodes, hidden_syntax_ranges};
mod styles;
use styles::{
    active_marker_style_spans, projection_style_spans, reserved_inline_math_marker_style_spans,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionSegment {
    Visible,
    HiddenSyntax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionStyle {
    Marker,
    Emphasis,
    Strong,
    InlineCode,
    InlineMath,
    Link,
    Image,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionStyleSpan {
    pub range: Range<usize>,
    pub style: ProjectionStyle,
    pub node_id: SourceNodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionEdit {
    pub source_range: Range<usize>,
    pub replacement: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownProjection {
    pub text: String,
    pub active_inline: Option<SourceNodeId>,
    pub styles: Vec<ProjectionStyleSpan>,
    pub source_range: Range<usize>,
    display_to_source: Vec<usize>,
    display_end_to_source: Vec<usize>,
    source_to_display: Vec<usize>,
}

impl MarkdownProjection {
    pub fn build(document: &SourceMarkdownDocument, active_inline: Option<SourceNodeId>) -> Self {
        Self::build_range(document, active_inline, 0..document.source.len())
    }

    pub fn build_range(
        document: &SourceMarkdownDocument,
        active_inline: Option<SourceNodeId>,
        source_range: Range<usize>,
    ) -> Self {
        Self::build_range_with_reveal(document, active_inline, source_range, true)
    }

    /// Builds the text that owns a mounted editor surface's physical layout.
    ///
    /// Inline-math delimiters stay in this projection even while the formula
    /// is rendered as SVG. The overlay covers those reserved glyphs in preview
    /// mode, and activation only changes their paint style/visibility instead
    /// of inserting two new characters into the Input and reflowing the
    /// paragraph.
    pub(crate) fn build_surface_range(
        document: &SourceMarkdownDocument,
        active_inline: Option<SourceNodeId>,
        source_range: Range<usize>,
    ) -> Self {
        Self::build_range_with_options(document, active_inline, source_range, true, true)
    }

    fn build_range_with_reveal(
        document: &SourceMarkdownDocument,
        active_inline: Option<SourceNodeId>,
        source_range: Range<usize>,
        reveal_active: bool,
    ) -> Self {
        Self::build_range_with_options(document, active_inline, source_range, reveal_active, false)
    }

    fn build_range_with_options(
        document: &SourceMarkdownDocument,
        active_inline: Option<SourceNodeId>,
        source_range: Range<usize>,
        reveal_active: bool,
        reserve_inline_math_markers: bool,
    ) -> Self {
        let mut hidden = hidden_syntax_ranges(
            document,
            active_inline,
            &source_range,
            reveal_active,
            reserve_inline_math_markers,
        );
        if source_range != (0..document.source.len())
            && let Some(separator) = trailing_line_ending(&document.source, &source_range)
        {
            hidden.push(separator);
            hidden.sort_by_key(|range| range.start);
        }
        let mut builder =
            ProjectionBuilder::new(document.source.len(), active_inline, source_range.clone());
        let terminal_code_content_boundary =
            terminal_code_content_boundary(document, &source_range);
        builder.append_source(&document.source, source_range, &hidden);
        let mut projection = builder.finish();
        if let Some(source_offset) = terminal_code_content_boundary {
            if let Some(last) = projection.display_to_source.last_mut() {
                *last = source_offset;
            }
            if let Some(last) = projection.display_end_to_source.last_mut() {
                *last = source_offset;
            }
        }
        projection.styles = projection_style_spans(document, &projection);
        projection.styles.extend(active_marker_style_spans(
            document,
            active_inline,
            &projection,
        ));
        if reserve_inline_math_markers {
            projection
                .styles
                .extend(reserved_inline_math_marker_style_spans(
                    document,
                    active_inline,
                    &projection,
                ));
        }
        projection.styles.sort_by_key(|span| span.range.start);
        projection
    }

    pub fn display_to_source(&self, display_offset: usize) -> usize {
        let display_offset = floor_char_boundary(&self.text, display_offset);
        self.display_to_source
            .get(display_offset)
            .copied()
            .unwrap_or_else(|| *self.display_to_source.last().unwrap_or(&0))
    }

    pub fn source_to_display(&self, source_offset: usize) -> usize {
        let display_offset = self
            .source_to_display
            .get(source_offset)
            .copied()
            .unwrap_or(self.text.len());
        // `source_to_display` is indexed per byte, so a caller that hands us an
        // offset inside a multi-byte character (for example a stale cursor after
        // a deferred newline flush) would otherwise receive a display offset
        // that splits a UTF-8 character and panics on the next string slice.
        floor_char_boundary(&self.text, display_offset)
    }

    pub fn display_end_to_source(&self, display_offset: usize) -> usize {
        let display_offset = floor_char_boundary(&self.text, display_offset);
        self.display_end_to_source
            .get(display_offset)
            .copied()
            .unwrap_or_else(|| *self.display_end_to_source.last().unwrap_or(&0))
    }

    pub fn edit_for_value(&self, value: &str) -> Option<ProjectionEdit> {
        if value == self.text {
            return None;
        }
        let prefix = common_prefix(&self.text, value);
        let suffix = common_suffix(&self.text[prefix..], &value[prefix..]);
        let old_end = self.text.len().saturating_sub(suffix);
        let new_end = value.len().saturating_sub(suffix);
        let source_start = self.display_to_source(prefix);
        let source_end = if old_end == prefix {
            source_start
        } else {
            self.display_end_to_source(old_end)
        };
        let source_range = source_start..source_end;
        if source_range.len() != old_end.saturating_sub(prefix) {
            return None;
        }
        Some(ProjectionEdit {
            source_range,
            replacement: value[prefix..new_end].to_owned(),
        })
    }
}

fn trailing_line_ending(source: &str, range: &Range<usize>) -> Option<Range<usize>> {
    let value = source.get(range.clone())?;
    let length = if value.ends_with("\r\n") {
        2
    } else if value.ends_with('\n') {
        1
    } else {
        0
    };
    (length > 0).then(|| range.end - length..range.end)
}

fn terminal_code_content_boundary(
    document: &SourceMarkdownDocument,
    source_range: &Range<usize>,
) -> Option<usize> {
    document.blocks.iter().rev().find_map(|block| {
        if block.source_range.end != source_range.end
            || block.source_range.start < source_range.start
        {
            return None;
        }
        match &block.kind {
            markdown_source::SourceBlockKind::CodeFence {
                closing_fence: Some(_),
                ..
            } => block.content_range.as_ref().map(|content| content.end),
            _ => None,
        }
    })
}

struct ProjectionBuilder {
    text: String,
    active_inline: Option<SourceNodeId>,
    display_to_source: Vec<usize>,
    display_end_to_source: Vec<usize>,
    source_to_display: Vec<usize>,
    source_range: Range<usize>,
}

impl ProjectionBuilder {
    fn new(
        source_len: usize,
        active_inline: Option<SourceNodeId>,
        source_range: Range<usize>,
    ) -> Self {
        Self {
            text: String::with_capacity(source_range.len()),
            active_inline,
            display_to_source: vec![source_range.start],
            display_end_to_source: vec![source_range.start],
            source_to_display: vec![0; source_len.saturating_add(1)],
            source_range,
        }
    }

    fn append_source(&mut self, source: &str, source_range: Range<usize>, hidden: &[Range<usize>]) {
        let mut cursor = source_range.start;
        for range in hidden {
            self.append_visible(source, cursor..range.start);
            self.hide(range.clone());
            cursor = range.end;
        }
        self.append_visible(source, cursor..source_range.end);
    }

    fn append_visible(&mut self, source: &str, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        let source_start = range.start;
        let display_start = self.text.len();
        self.text.push_str(&source[range.clone()]);
        for source_offset in range {
            self.source_to_display[source_offset] =
                display_start + source_offset.saturating_sub(source_start);
            self.display_to_source.push(source_offset + 1);
            self.display_end_to_source.push(source_offset + 1);
            self.source_to_display[source_offset + 1] =
                display_start + source_offset + 1 - source_start;
        }
    }

    fn hide(&mut self, range: Range<usize>) {
        let display_offset = self.text.len();
        self.source_to_display[range.clone()].fill(display_offset);
        self.source_to_display[range.end] = display_offset;
        if let Some(last) = self.display_to_source.last_mut() {
            *last = range.end;
        }
    }

    fn finish(self) -> MarkdownProjection {
        MarkdownProjection {
            text: self.text,
            active_inline: self.active_inline,
            styles: Vec::new(),
            source_range: self.source_range,
            display_to_source: self.display_to_source,
            display_end_to_source: self.display_end_to_source,
            source_to_display: self.source_to_display,
        }
    }
}

fn common_prefix(left: &str, right: &str) -> usize {
    left.char_indices()
        .zip(right.chars())
        .take_while(|((_, left), right)| left == right)
        .last()
        .map_or(0, |((offset, ch), _)| offset + ch.len_utf8())
}

fn common_suffix(left: &str, right: &str) -> usize {
    left.char_indices()
        .rev()
        .zip(right.chars().rev())
        .take_while(|((_, left), right)| left == right)
        .map(|((_, ch), _)| ch.len_utf8())
        .sum()
}

/// Snaps `offset` down to the nearest UTF-8 character boundary of `text`.
pub(crate) fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(source: &str) -> SourceMarkdownDocument {
        SourceMarkdownDocument::parse(source).unwrap()
    }

    #[test]
    fn display_mappings_stay_on_char_boundaries_for_every_byte_offset() {
        let source = "新段落包含 **加粗新** 与 `code新`。\n\n第二行新内容";
        let document = document(source);
        let projection = MarkdownProjection::build(&document, None);
        assert!(projection.text.is_char_boundary(projection.text.len()));

        for source_offset in 0..=source.len() {
            let display = projection.source_to_display(source_offset);
            assert!(
                projection.text.is_char_boundary(display),
                "source offset {source_offset} mapped to non-boundary display offset {display}"
            );
        }
        for display_offset in 0..=projection.text.len() {
            let start = projection.display_to_source(display_offset);
            let end = projection.display_end_to_source(display_offset);
            assert!(
                source.is_char_boundary(start),
                "display offset {display_offset} mapped to non-boundary source offset {start}"
            );
            assert!(
                source.is_char_boundary(end),
                "display offset {display_offset} mapped to non-boundary source end {end}"
            );
        }
    }

    #[test]
    fn source_offset_inside_a_multibyte_char_snaps_to_the_character_start() {
        let source = "新段落";
        let document = document(source);
        let projection = MarkdownProjection::build(&document, None);
        // '新' occupies bytes 0..3; offsets inside it must snap to its start.
        assert_eq!(projection.source_to_display(1), 0);
        assert_eq!(projection.source_to_display(2), 0);
        assert_eq!(projection.source_to_display(3), 3);
    }

    #[test]
    fn floor_char_boundary_snaps_into_multibyte_characters() {
        let text = "新x";
        assert_eq!(floor_char_boundary(text, 0), 0);
        assert_eq!(floor_char_boundary(text, 1), 0);
        assert_eq!(floor_char_boundary(text, 2), 0);
        assert_eq!(floor_char_boundary(text, 3), 3);
        assert_eq!(floor_char_boundary(text, 4), 4);
        assert_eq!(floor_char_boundary(text, usize::MAX), 4);
    }
}
