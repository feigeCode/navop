use super::MarkdownEditor;
use crate::editor::surface::MarkdownSurfaceKey;
use gpui::{
    App, Bounds, Corners, Image, ImageFormat, IntoElement, ObjectFit, Pixels, Styled, Window,
    canvas, point, px,
};
use markdown_source::SourceInlineKind;
use std::{ops::Range, sync::Arc};

impl MarkdownEditor {
    pub(super) fn inline_math_overlays(&self, key: MarkdownSurfaceKey) -> Vec<gpui::AnyElement> {
        let document = self.history.document();
        let nodes = match key {
            MarkdownSurfaceKey::Empty => return Vec::new(),
            MarkdownSurfaceKey::Block(block_id) => {
                let Some(block) = document.block_by_id(block_id) else {
                    return Vec::new();
                };
                &block.inline_nodes
            }
            MarkdownSurfaceKey::TableCell { .. } => {
                let Some(address) = key.table_address() else {
                    return Vec::new();
                };
                let Ok(cell) = document.table_cell(address) else {
                    return Vec::new();
                };
                &cell.inline_nodes
            }
        };
        let Some(surface) = self.surface(key) else {
            return Vec::new();
        };
        nodes
            .iter()
            .filter_map(|node| {
                let SourceInlineKind::InlineMath { .. } = node.kind else {
                    return None;
                };
                if surface.projection.active_inline == Some(node.id) {
                    return None;
                }
                let source = document.source[node.content_range.clone()?].to_owned();
                let artifact = self.inline_math_artifacts.get(&source)?;
                let range = surface
                    .projection
                    .source_to_display(node.content_range.as_ref()?.start)
                    ..surface
                        .projection
                        .source_to_display(node.content_range.as_ref()?.end);
                Some(inline_math_overlay(
                    surface.input.clone(),
                    range,
                    artifact.clone(),
                ))
            })
            .collect()
    }
}

fn inline_math_overlay(
    input: gpui::Entity<gpui_component::input::InputState>,
    range: Range<usize>,
    artifact: crate::MarkdownBlockRenderArtifact,
) -> gpui::AnyElement {
    canvas(
        move |_, _, cx| overlay_prepaint(&input, &range, &artifact, cx),
        move |_, mut state, window, cx| paint_overlay(&mut state, window, cx),
    )
    .absolute()
    .size_full()
    .into_any_element()
}

struct InlineMathPaint {
    bounds: Bounds<Pixels>,
    image: Arc<Image>,
}

fn overlay_prepaint(
    input: &gpui::Entity<gpui_component::input::InputState>,
    range: &Range<usize>,
    artifact: &crate::MarkdownBlockRenderArtifact,
    cx: &mut App,
) -> Option<InlineMathPaint> {
    let bounds = input.read(cx).range_to_bounds(range)?;
    let width = artifact
        .intrinsic_width
        .unwrap_or(bounds.size.width.as_f32())
        .clamp(12., bounds.size.width.as_f32().max(12.));
    let height = artifact.intrinsic_height.unwrap_or(24.).clamp(16., 24.);
    let center = bounds.center();
    Some(InlineMathPaint {
        bounds: Bounds::new(
            point(center.x - px(width / 2.), center.y - px(height / 2.)),
            gpui::size(px(width), px(height)),
        ),
        image: Arc::new(Image::from_bytes(ImageFormat::Svg, artifact.bytes.clone())),
    })
}

fn paint_overlay(state: &mut Option<InlineMathPaint>, window: &mut Window, cx: &mut App) {
    let Some(state) = state else {
        return;
    };
    let Some(image) = state.image.clone().use_render_image(window, cx) else {
        return;
    };
    let image_bounds = ObjectFit::Contain.get_bounds(state.bounds, image.size(0));
    let _ = window.paint_image(image_bounds, Corners::default(), image, 0, false);
}
