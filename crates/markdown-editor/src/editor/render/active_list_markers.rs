use super::MarkdownEditor;
use super::list_marker_source::list_markers;
use crate::editor::surface::MarkdownSurfaceKey;
use gpui::{
    App, Bounds, Corners, Hsla, InteractiveElement, IntoElement, ParentElement, PathBuilder,
    Pixels, SharedString, Styled, TextAlign, TextRun, Window, canvas, point, px,
};
use markdown_source::SourceBlock;

const LINE_HEIGHT: f32 = 24.;
const FONT_SIZE: f32 = 16.;
const MARKER_GAP: f32 = 6.;

impl MarkdownEditor {
    pub(super) fn list_marker_overlay(
        &self,
        key: MarkdownSurfaceKey,
        block: &SourceBlock,
        active: bool,
    ) -> gpui::AnyElement {
        let surface = self
            .surface(key)
            .expect("a list marker overlay must use its block surface");
        let markers = list_markers(block, |offset| surface.projection.source_to_display(offset));
        let input = surface.input.clone();
        let block_id = block.id;
        let foreground = self.theme.foreground;
        let primary = self.theme.primary;
        let check = self.theme.background;
        gpui::div()
            .id(("markdown-list-markers", block.id.0))
            .debug_selector(move || {
                if active {
                    format!("markdown-active-list-markers-{}", block_id.0)
                } else {
                    format!("markdown-list-markers-{}", block_id.0)
                }
            })
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .child(
                canvas(
                    move |_, _, cx| marker_layouts(&input, &markers, cx),
                    move |_, layouts, window, cx| {
                        paint_markers(layouts, foreground, primary, check, window, cx);
                    },
                )
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .left_0(),
            )
            .into_any_element()
    }
}

#[derive(Clone)]
pub(super) struct ListMarker {
    pub(super) display_offset: usize,
    pub(super) kind: MarkerKind,
}

#[derive(Clone)]
pub(super) enum MarkerKind {
    Text(String),
    Task(bool),
}

struct MarkerLayout {
    caret: Bounds<Pixels>,
    kind: MarkerKind,
}

fn marker_layouts(
    input: &gpui::Entity<gpui_component::input::InputState>,
    markers: &[ListMarker],
    cx: &mut App,
) -> Vec<MarkerLayout> {
    markers
        .iter()
        .filter_map(|marker| {
            let range = marker.display_offset..marker.display_offset;
            input
                .read(cx)
                .range_to_bounds(&range)
                .map(|caret| MarkerLayout {
                    caret,
                    kind: marker.kind.clone(),
                })
        })
        .collect()
}

fn paint_markers(
    layouts: Vec<MarkerLayout>,
    foreground: Hsla,
    primary: Hsla,
    check: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    for layout in layouts {
        match layout.kind {
            MarkerKind::Text(text) => {
                paint_text_marker(&text, layout.caret, foreground, window, cx)
            }
            MarkerKind::Task(checked) => {
                paint_task_marker(checked, layout.caret, foreground, primary, check, window)
            }
        }
    }
}

fn paint_text_marker(
    text: &str,
    caret: Bounds<Pixels>,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let text: SharedString = text.to_owned().into();
    let line = window.text_system().shape_line(
        text.clone(),
        px(FONT_SIZE),
        &[TextRun {
            len: text.len(),
            font: window.text_style().font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );
    let width = line.width;
    let marker_right = caret.origin.x - px(MARKER_GAP);
    let origin = point(marker_right - width, caret.origin.y);
    let _ = line.paint(origin, px(LINE_HEIGHT), TextAlign::Left, None, window, cx);
}

fn paint_task_marker(
    checked: bool,
    caret: Bounds<Pixels>,
    foreground: Hsla,
    primary: Hsla,
    check: Hsla,
    window: &mut Window,
) {
    let bounds = task_marker_bounds(caret);
    window.paint_quad(gpui::PaintQuad {
        bounds,
        corner_radii: Corners::all(px(3.)),
        background: checked
            .then_some(primary)
            .unwrap_or_else(gpui::transparent_black)
            .into(),
        border_widths: gpui::Edges::all(px(1.)),
        border_color: checked
            .then_some(primary)
            .unwrap_or(foreground.opacity(0.55)),
        border_style: gpui::BorderStyle::Solid,
    });
    if checked {
        paint_check(bounds, check, window);
    }
}

fn task_marker_bounds(caret: Bounds<Pixels>) -> Bounds<Pixels> {
    let size = px(14.);
    let line_height = caret.size.height.max(px(LINE_HEIGHT));
    Bounds::new(
        point(
            caret.origin.x - px(MARKER_GAP) - size,
            caret.origin.y + (line_height - size) / 2.,
        ),
        gpui::size(size, size),
    )
}

fn paint_check(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    // Paint a geometry check mark instead of relying on a font glyph. This keeps
    // the task marker crisp and stable across fonts, fallback chains and scale
    // factors, and matches the compact checkbox used by Typora.
    let mut path = PathBuilder::stroke(px(1.75));
    path.move_to(point(bounds.left() + px(3.5), bounds.top() + px(7.)));
    path.line_to(point(bounds.left() + px(6.), bounds.top() + px(9.5)));
    path.line_to(point(bounds.left() + px(10.75), bounds.top() + px(4.5)));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_marker_is_centered_in_the_input_line_and_stays_left_of_text() {
        let caret = Bounds::new(point(px(100.), px(40.)), gpui::size(px(1.), px(24.)));
        let marker = task_marker_bounds(caret);

        assert_eq!(
            marker,
            Bounds::new(point(px(80.), px(45.)), gpui::size(px(14.), px(14.)))
        );
        assert_eq!(caret.left() - marker.right(), px(MARKER_GAP));
        assert_eq!(marker.center().y, caret.top() + px(12.));
    }
}
