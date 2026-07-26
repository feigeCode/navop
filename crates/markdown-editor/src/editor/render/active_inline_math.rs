use super::MarkdownEditor;
use crate::editor::surface::MarkdownSurfaceKey;
use gpui::{
    App, Bounds, Corners, Hsla, Image, ImageFormat, IntoElement, ObjectFit, Pixels, Styled, Window,
    canvas, fill, point, px,
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
                    self.theme.background,
                ))
            })
            .collect()
    }
}

fn inline_math_overlay(
    input: gpui::Entity<gpui_component::input::InputState>,
    range: Range<usize>,
    artifact: crate::MarkdownBlockRenderArtifact,
    background: Hsla,
) -> gpui::AnyElement {
    canvas(
        move |_, _, cx| overlay_prepaint(&input, &range, &artifact, background, cx),
        move |_, mut state, window, cx| paint_overlay(&mut state, window, cx),
    )
    .absolute()
    .size_full()
    .into_any_element()
}

struct InlineMathPaint {
    geometry: InlineMathGeometry,
    image: Arc<Image>,
    background: Hsla,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct InlineMathGeometry {
    cover_bounds: Bounds<Pixels>,
    image_bounds: Bounds<Pixels>,
}

fn overlay_prepaint(
    input: &gpui::Entity<gpui_component::input::InputState>,
    range: &Range<usize>,
    artifact: &crate::MarkdownBlockRenderArtifact,
    background: Hsla,
    cx: &mut App,
) -> Option<InlineMathPaint> {
    let source_bounds = input.read(cx).range_to_bounds(range)?;
    let geometry = inline_math_geometry(source_bounds, artifact);
    Some(InlineMathPaint {
        geometry,
        image: Arc::new(Image::from_bytes(ImageFormat::Svg, artifact.bytes.clone())),
        background,
    })
}

fn inline_math_geometry(
    source_bounds: Bounds<Pixels>,
    artifact: &crate::MarkdownBlockRenderArtifact,
) -> InlineMathGeometry {
    let width = artifact
        .intrinsic_width
        .unwrap_or(source_bounds.size.width.as_f32())
        .clamp(12., source_bounds.size.width.as_f32().max(12.));
    let height = artifact.intrinsic_height.unwrap_or(24.).clamp(16., 24.);
    let center = source_bounds.center();
    InlineMathGeometry {
        cover_bounds: source_bounds,
        image_bounds: Bounds::new(
            point(center.x - px(width / 2.), center.y - px(height / 2.)),
            gpui::size(px(width), px(height)),
        ),
    }
}

fn paint_overlay(state: &mut Option<InlineMathPaint>, window: &mut Window, cx: &mut App) {
    let Some(state) = state else {
        return;
    };
    let Some(image) = state.image.clone().use_render_image(window, cx) else {
        return;
    };
    // The permanent Input remains the sole owner of text layout and caret
    // geometry. Once the SVG is paintable, cover the complete source range
    // before painting it; merely making a text run transparent is not reliable
    // across text backends and lets both the TeX source and SVG show at once.
    window.paint_quad(fill(state.geometry.cover_bounds, state.background));
    let image_bounds = ObjectFit::Contain.get_bounds(state.geometry.image_bounds, image.size(0));
    let _ = window.paint_image(image_bounds, Corners::default(), image, 0, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_math_covers_the_complete_source_range_before_painting_svg() {
        let source_bounds = Bounds::new(point(px(40.), px(12.)), gpui::size(px(120.), px(24.)));
        let artifact = crate::MarkdownBlockRenderArtifact {
            media_type: "image/svg+xml".to_owned(),
            bytes: Vec::new(),
            intrinsic_width: Some(72.),
            intrinsic_height: Some(18.),
        };

        let geometry = inline_math_geometry(source_bounds, &artifact);

        assert_eq!(source_bounds, geometry.cover_bounds);
        assert_eq!(geometry.image_bounds.size, gpui::size(px(72.), px(18.)));
        assert_eq!(source_bounds.center(), geometry.image_bounds.center());
    }
}
