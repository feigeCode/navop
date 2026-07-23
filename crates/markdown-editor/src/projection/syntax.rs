use super::block_hidden_ranges;
use markdown_source::{
    SourceBlock, SourceBlockKind, SourceInlineKind, SourceInlineNode, SourceMarkdownDocument,
    SourceNodeId,
};
use std::ops::Range;

pub(super) fn hidden_syntax_ranges(
    document: &SourceMarkdownDocument,
    active_inline: Option<SourceNodeId>,
    source_range: &Range<usize>,
    reveal_active: bool,
) -> Vec<Range<usize>> {
    let active_inline = reveal_active.then_some(active_inline).flatten();
    let active_range = active_inline.and_then(|id| active_inline_range(document, id));
    let active_image = active_inline.and_then(|id| active_image_range(document, id));
    let mut ranges = document
        .blocks
        .iter()
        .flat_map(block_inline_nodes)
        .filter(|node| range_contains(source_range, &node.source_range))
        .filter(|node| {
            active_inline != Some(node.id)
                && active_range
                    .as_ref()
                    .is_none_or(|range| range != &node.source_range)
                && active_image
                    .as_ref()
                    .is_none_or(|range| !range_contains(range, &node.source_range))
        })
        .flat_map(node_hidden_ranges)
        .collect::<Vec<_>>();
    ranges.extend(
        document
            .blocks
            .iter()
            .filter(|block| range_contains(source_range, &block.source_range))
            .flat_map(|block| block_hidden_ranges(&document.source, block)),
    );
    ranges.sort_by_key(|range| range.start);
    merge_ranges(ranges)
}

pub(super) fn block_inline_nodes(block: &SourceBlock) -> Vec<&SourceInlineNode> {
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

fn active_inline_range(
    document: &SourceMarkdownDocument,
    id: SourceNodeId,
) -> Option<Range<usize>> {
    document
        .blocks
        .iter()
        .flat_map(block_inline_nodes)
        .find(|node| node.id == id)
        .map(|node| node.source_range.clone())
}

fn active_image_range(document: &SourceMarkdownDocument, id: SourceNodeId) -> Option<Range<usize>> {
    document
        .blocks
        .iter()
        .flat_map(block_inline_nodes)
        .find(|node| node.id == id)
        .and_then(|node| match &node.kind {
            SourceInlineKind::Image(image) => Some(image.full_range.clone()),
            _ => None,
        })
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
        | SourceInlineKind::InlineMath {
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
