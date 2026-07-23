use super::MarkdownEditor;
use super::list_marker_source::list_markers;
use gpui::{
    App, Bounds, Corners, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels,
    SharedString, Styled, TextAlign, TextRun, Window, canvas, point, px,
};
use markdown_source::SourceBlock;

const LINE_HEIGHT: f32 = 24.;
const FONT_SIZE: f32 = 16.;
const MARKER_GAP: f32 = 6.;

impl MarkdownEditor {
    pub(super) fn active_list_marker_overlay(&self, block: &SourceBlock) -> gpui::AnyElement {
        let markers = list_markers(block, |offset| self.projection.source_to_display(offset));
        let input = self.input.clone();
        let foreground = self.theme.foreground;
        let primary = self.theme.primary;
        let check = self.theme.background;
        gpui::div()
            .id(("markdown-active-list-markers", block.id.0))
            .debug_selector(|| format!("markdown-active-list-markers-{}", block.id.0))
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
                paint_task_marker(checked, layout.caret, primary, check, window, cx)
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
    primary: Hsla,
    check: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let bounds = Bounds::new(
        point(
            caret.origin.x - px(MARKER_GAP + 14.),
            caret.origin.y + px(5.),
        ),
        gpui::size(px(14.), px(14.)),
    );
    window.paint_quad(gpui::PaintQuad {
        bounds,
        corner_radii: Corners::all(px(3.)),
        background: checked
            .then_some(primary)
            .unwrap_or_else(gpui::transparent_black)
            .into(),
        border_widths: gpui::Edges::all(px(1.)),
        border_color: primary,
        border_style: gpui::BorderStyle::Solid,
    });
    if checked {
        paint_check(bounds, check, window, cx);
    }
}

fn paint_check(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window, cx: &mut App) {
    let caret = Bounds::new(
        point(bounds.right() + px(MARKER_GAP), bounds.origin.y - px(5.)),
        bounds.size,
    );
    paint_text_marker("✓", caret, color, window, cx);
}
