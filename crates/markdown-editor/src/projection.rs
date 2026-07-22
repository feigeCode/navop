use markdown_source::{
    SourceBlockKind, SourceInlineKind, SourceInlineNode, SourceMarkdownDocument, SourceNodeId,
};
use std::ops::Range;

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
    Link,
    Image,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionStyleSpan {
    pub range: Range<usize>,
    pub style: ProjectionStyle,
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
        let revealed = active_inline.and_then(|id| inline_range(document, id));
        let hidden = hidden_syntax_ranges(document, revealed, &source_range);
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
        let source_range = self.display_to_source(prefix)..self.display_end_to_source(old_end);
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
    (!range.is_empty()).then_some(ProjectionStyleSpan { range, style })
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
        SourceInlineKind::Link(link) => Some((link.label_range.clone(), ProjectionStyle::Link)),
        SourceInlineKind::Image(image) => Some((image.alt_range.clone(), ProjectionStyle::Image)),
        SourceInlineKind::Delete { .. } => {
            Some((node.content_range.clone()?, ProjectionStyle::Delete))
        }
        _ => None,
    }
}

fn hidden_syntax_ranges(
    document: &SourceMarkdownDocument,
    revealed: Option<Range<usize>>,
    source_range: &Range<usize>,
) -> Vec<Range<usize>> {
    let mut ranges = document
        .blocks
        .iter()
        .flat_map(block_inline_nodes)
        .filter(|node| range_contains(source_range, &node.source_range))
        .flat_map(node_hidden_ranges)
        .filter(|range| {
            revealed
                .as_ref()
                .is_none_or(|active| !ranges_overlap(active, range))
        })
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start);
    merge_ranges(ranges)
}

fn block_inline_nodes(block: &markdown_source::SourceBlock) -> Vec<&SourceInlineNode> {
    let mut nodes = block.inline_nodes.iter().collect::<Vec<_>>();
    if let SourceBlockKind::Table(table) = &block.kind {
        nodes.extend(
            table
                .rows
                .iter()
                .flat_map(|row| row.cells.iter())
                .flat_map(|cell| cell.inline_nodes.iter()),
        );
    }
    nodes
}

fn node_hidden_ranges(node: &SourceInlineNode) -> Vec<Range<usize>> {
    match &node.kind {
        SourceInlineKind::Emphasis {
            opening_marker,
            closing_marker,
        }
        | SourceInlineKind::Strong {
            opening_marker,
            closing_marker,
        }
        | SourceInlineKind::InlineCode {
            opening_marker,
            closing_marker,
        }
        | SourceInlineKind::Delete {
            opening_marker,
            closing_marker,
        } => vec![opening_marker.clone(), closing_marker.clone()],
        SourceInlineKind::Link(link) => vec![
            link.full_range.start..link.label_range.start,
            link.label_range.end..link.full_range.end,
        ],
        SourceInlineKind::Image(image) => vec![
            image.full_range.start..image.alt_range.start,
            image.alt_range.end..image.full_range.end,
        ],
        _ => Vec::new(),
    }
}

fn inline_range(document: &SourceMarkdownDocument, id: SourceNodeId) -> Option<Range<usize>> {
    document
        .blocks
        .iter()
        .flat_map(block_inline_nodes)
        .find(|node| node.id == id)
        .map(|node| match &node.kind {
            SourceInlineKind::Image(image) => image.full_range.clone(),
            _ => node.source_range.clone(),
        })
}

fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn range_contains(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn merge_ranges(ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges.into_iter().filter(|range| !range.is_empty()) {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
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
