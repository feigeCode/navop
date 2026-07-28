use super::{MarkdownProjection, ProjectionStyle, ProjectionStyleSpan, block_inline_nodes};
use markdown_source::{SourceInlineKind, SourceInlineNode, SourceMarkdownDocument, SourceNodeId};
use std::ops::Range;

pub(super) fn projection_style_spans(
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

pub(super) fn active_marker_style_spans(
    document: &SourceMarkdownDocument,
    active_inline: Option<SourceNodeId>,
    projection: &MarkdownProjection,
) -> Vec<ProjectionStyleSpan> {
    let Some(node) = active_inline.and_then(|id| find_inline_node(document, id)) else {
        return Vec::new();
    };
    marker_source_ranges(node)
        .into_iter()
        .filter_map(|range| style_span(range, ProjectionStyle::Marker, node.id, projection))
        .collect()
}

pub(super) fn reserved_inline_math_marker_style_spans(
    document: &SourceMarkdownDocument,
    active_inline: Option<SourceNodeId>,
    projection: &MarkdownProjection,
) -> Vec<ProjectionStyleSpan> {
    document
        .blocks
        .iter()
        .flat_map(block_inline_nodes)
        .filter(|node| range_contains(&projection.source_range, &node.source_range))
        .filter(|node| Some(node.id) != active_inline)
        .filter(|node| matches!(node.kind, SourceInlineKind::InlineMath { .. }))
        .flat_map(|node| {
            marker_source_ranges(node)
                .into_iter()
                .filter_map(|range| style_span(range, ProjectionStyle::Marker, node.id, projection))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn node_style_span(
    node: &SourceInlineNode,
    projection: &MarkdownProjection,
) -> Option<ProjectionStyleSpan> {
    let (range, style) = node_style_source_range(node)?;
    style_span(range, style, node.id, projection)
}

fn node_style_source_range(node: &SourceInlineNode) -> Option<(Range<usize>, ProjectionStyle)> {
    match &node.kind {
        SourceInlineKind::Emphasis { .. } => content_style(node, ProjectionStyle::Emphasis),
        SourceInlineKind::Strong { .. } => content_style(node, ProjectionStyle::Strong),
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
        SourceInlineKind::Delete { .. } => content_style(node, ProjectionStyle::Delete),
        _ => None,
    }
}

fn content_style(
    node: &SourceInlineNode,
    style: ProjectionStyle,
) -> Option<(Range<usize>, ProjectionStyle)> {
    Some((node.content_range.clone()?, style))
}

fn marker_source_ranges(node: &SourceInlineNode) -> Vec<Range<usize>> {
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

fn find_inline_node(
    document: &SourceMarkdownDocument,
    id: SourceNodeId,
) -> Option<&SourceInlineNode> {
    document
        .blocks
        .iter()
        .flat_map(block_inline_nodes)
        .find(|node| node.id == id)
}

fn style_span(
    source_range: Range<usize>,
    style: ProjectionStyle,
    node_id: SourceNodeId,
    projection: &MarkdownProjection,
) -> Option<ProjectionStyleSpan> {
    let range = projection.source_to_display(source_range.start)
        ..projection.source_to_display(source_range.end);
    (!range.is_empty()).then_some(ProjectionStyleSpan {
        range,
        style,
        node_id,
    })
}

fn range_contains(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}
