use markdown_source::{
    SourceBlockKind, SourceInlineKind, SourceInlineNode, SourceMarkdownDocument, SourceNodeId,
};
use std::ops::Range;

mod block_syntax;
use block_syntax::block_hidden_ranges;
mod syntax;
use syntax::{block_inline_nodes, hidden_syntax_ranges};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionSegment {
    Visible,
    HiddenSyntax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionStyle {
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

    /// Build the editor projection without changing text metrics when an inline node is active.
    /// The active node is still recorded for source mapping and marker overlays.
    pub fn build_range_preserving_layout(
        document: &SourceMarkdownDocument,
        active_inline: Option<SourceNodeId>,
        source_range: Range<usize>,
    ) -> Self {
        Self::build_range_with_reveal(document, active_inline, source_range, false)
    }

    fn build_range_with_reveal(
        document: &SourceMarkdownDocument,
        active_inline: Option<SourceNodeId>,
        source_range: Range<usize>,
        reveal_active: bool,
    ) -> Self {
        let hidden = hidden_syntax_ranges(document, active_inline, &source_range, reveal_active);
        let mut builder =
            ProjectionBuilder::new(document.source.len(), active_inline, source_range.clone());
        builder.append_source(&document.source, source_range, &hidden);
        let mut projection = builder.finish();
        projection.styles = projection_style_spans(document, &projection);
        projection
    }

    pub fn display_to_source(&self, display_offset: usize) -> usize {
        self.display_to_source
            .get(display_offset)
            .copied()
            .unwrap_or_else(|| *self.display_to_source.last().unwrap_or(&0))
    }

    pub fn source_to_display(&self, source_offset: usize) -> usize {
        self.source_to_display
            .get(source_offset)
            .copied()
            .unwrap_or(self.text.len())
    }

    pub fn display_end_to_source(&self, display_offset: usize) -> usize {
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

fn projection_style_spans(
    document: &SourceMarkdownDocument,
    projection: &MarkdownProjection,
) -> Vec<ProjectionStyleSpan> {
    document
        .blocks
        .iter()
        .flat_map(block_inline_nodes)
        .filter(|node| range_contains(&projection.source_range, &node.source_range))
        .filter_map(|node| node_style_span(node, projection))
        .collect()
}

fn node_style_span(
    node: &SourceInlineNode,
    projection: &MarkdownProjection,
) -> Option<ProjectionStyleSpan> {
    let (source_range, style) = node_style_source_range(node)?;
    let range = projection.source_to_display(source_range.start)
        ..projection.source_to_display(source_range.end);
    (!range.is_empty()).then_some(ProjectionStyleSpan {
        range,
        style,
        node_id: node.id,
    })
}

fn node_style_source_range(node: &SourceInlineNode) -> Option<(Range<usize>, ProjectionStyle)> {
    match &node.kind {
        SourceInlineKind::Emphasis { .. } => {
            Some((node.content_range.clone()?, ProjectionStyle::Emphasis))
        }
        SourceInlineKind::Strong { .. } => {
            Some((node.content_range.clone()?, ProjectionStyle::Strong))
        }
        SourceInlineKind::InlineCode {
            opening_marker,
            closing_marker,
        } => Some((
            opening_marker.end..closing_marker.start,
            ProjectionStyle::InlineCode,
        )),
        SourceInlineKind::InlineMath {
            opening_marker,
            closing_marker,
        } => Some((
            opening_marker.end..closing_marker.start,
            ProjectionStyle::InlineMath,
        )),
        SourceInlineKind::Link(link) => Some((link.label_range.clone(), ProjectionStyle::Link)),
        SourceInlineKind::Image(image) => Some((image.alt_range.clone(), ProjectionStyle::Image)),
        SourceInlineKind::Delete { .. } => {
            Some((node.content_range.clone()?, ProjectionStyle::Delete))
        }
        _ => None,
    }
}

fn range_contains(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
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
