use super::MarkdownEditor;
use gpui::{
    App, Bounds, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels, SharedString,
    Styled, TextAlign, TextRun, Window, canvas, point, px,
};
use markdown_source::{SourceBlockKind, SourceInlineKind, SourceInlineNode};
use std::ops::Range;

const MARKER_FONT_SIZE: f32 = 12.;
const MARKER_LINE_HEIGHT: f32 = 18.;
const MARKER_GAP: f32 = 2.;

struct InlineMarkerPaint {
    content_start: Bounds<Pixels>,
    content_end: Bounds<Pixels>,
    opening: SharedString,
    closing: SharedString,
}

impl MarkdownEditor {
    pub(super) fn active_inline_marker_overlay(&self) -> Option<gpui::AnyElement> {
        let node_id = self.projection.active_inline?;
        let node = self.find_inline_node(node_id)?;
        let source = &self.history.document().source;
        let (content, opening, closing) = marker_ranges(node, source)?;
        let display_start = self.projection.source_to_display(content.start);
        let display_end = self.projection.source_to_display(content.end);
        let input = self.input.clone();
        let foreground = self.theme.muted_foreground;
        Some(
            gpui::div()
                .id(("markdown-active-inline-markers", node_id.0))
                .debug_selector(|| format!("markdown-active-inline-markers-{}", node_id.0))
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .left_0()
                .child(
                    canvas(
                        move |_, _, cx| {
                            marker_layout(&input, display_start, display_end, opening, closing, cx)
                        },
                        move |_, layout, window, cx| {
                            paint_inline_markers(layout, foreground, window, cx);
                        },
                    )
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .left_0(),
                )
                .into_any_element(),
        )
    }

    fn find_inline_node(
        &self,
        node_id: markdown_source::SourceNodeId,
    ) -> Option<&SourceInlineNode> {
        for block in &self.history.document().blocks {
            if let Some(node) = block.inline_nodes.iter().find(|node| node.id == node_id) {
                return Some(node);
            }
            if let SourceBlockKind::Table(table) = &block.kind {
                for row in &table.rows {
                    if let Some(node) = row
                        .cells
                        .iter()
                        .flat_map(|cell| cell.inline_nodes.iter())
                        .find(|node| node.id == node_id)
                    {
                        return Some(node);
                    }
                }
            }
        }
        None
    }
}

fn marker_ranges(
    node: &SourceInlineNode,
    source: &str,
) -> Option<(Range<usize>, SharedString, SharedString)> {
    let (content, opening, closing) = match &node.kind {
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
        } => (
            node.content_range.clone()?,
            opening_marker.clone(),
            closing_marker.clone(),
        ),
        SourceInlineKind::Link(link) => (
            link.label_range.clone(),
            link.full_range.start..link.label_range.start,
            link.label_range.end..link.full_range.end,
        ),
        SourceInlineKind::Image(image) => (
            image.alt_range.clone(),
            image.full_range.start..image.alt_range.start,
            image.alt_range.end..image.full_range.end,
        ),
        _ => return None,
    };
    let opening = source.get(opening)?.to_owned().into();
    let closing = source.get(closing)?.to_owned().into();
    Some((content, opening, closing))
}

fn marker_layout(
    input: &gpui::Entity<gpui_component::input::InputState>,
    start: usize,
    end: usize,
    opening: SharedString,
    closing: SharedString,
    cx: &mut App,
) -> Option<InlineMarkerPaint> {
    let state = input.read(cx);
    Some(InlineMarkerPaint {
        content_start: state.range_to_bounds(&(start..start))?,
        content_end: state.range_to_bounds(&(end..end))?,
        opening,
        closing,
    })
}

fn paint_inline_markers(
    layout: Option<InlineMarkerPaint>,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(layout) = layout else {
        return;
    };
    paint_marker(
        &layout.opening,
        layout.content_start,
        true,
        color,
        window,
        cx,
    );
    paint_marker(
        &layout.closing,
        layout.content_end,
        false,
        color,
        window,
        cx,
    );
}

fn paint_marker(
    marker: &str,
    anchor: Bounds<Pixels>,
    before: bool,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    if marker.is_empty() {
        return;
    }
    let marker: SharedString = marker.to_owned().into();
    let line = window.text_system().shape_line(
        marker.clone(),
        px(MARKER_FONT_SIZE),
        &[TextRun {
            len: marker.len(),
            font: window.text_style().font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );
    let x = if before {
        anchor.origin.x - line.width - px(MARKER_GAP)
    } else {
        anchor.right() + px(MARKER_GAP)
    };
    let _ = line.paint(
        point(x, anchor.origin.y),
        px(MARKER_LINE_HEIGHT),
        TextAlign::Left,
        None,
        window,
        cx,
    );
}
