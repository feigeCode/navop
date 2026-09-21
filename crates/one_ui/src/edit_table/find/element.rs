//! 命中高亮绘制元素
//!
//! 整行文本只 shape 一次，再按字符下标取出命中区间的真实像素位置，
//! 这样 CJK 宽字符、比例字体和单元格内滚动都不会让高亮错位。

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, Corners, Element, ElementId, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, LayoutId, Pixels, SharedString, Style, TextRun, Window, fill, point,
};

use super::{MATCH_RADIUS, find_highlight_color};

/// 一行里的高亮片段（整行文本中的字符区间 + 是否当前命中）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightSegment {
    pub char_range: Range<usize>,
    pub is_current: bool,
}

/// 把整行文本中的命中区间绘制成高亮背景。
///
/// `cell_range` 指定当前单元格在整行文本中的字符区间：元素把自己的
/// 原点上移到该区间的起点，因此高亮天然落在单元格坐标里。
pub struct FindHighlightElement {
    text: SharedString,
    font_size: Pixels,
    line_height: Pixels,
    cell_range: Range<usize>,
    segments: Rc<Vec<HighlightSegment>>,
    selection_color: Hsla,
    text_runs: Rc<Vec<TextRun>>,
}

impl FindHighlightElement {
    pub fn new(
        text: SharedString,
        font_size: Pixels,
        line_height: Pixels,
        cell_range: Range<usize>,
        segments: Rc<Vec<HighlightSegment>>,
        selection_color: Hsla,
        text_runs: Rc<Vec<TextRun>>,
    ) -> Self {
        Self {
            text,
            font_size,
            line_height,
            cell_range,
            segments,
            selection_color,
            text_runs,
        }
    }
}

impl IntoElement for FindHighlightElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FindHighlightElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // 只负责绘制，尺寸由单元格容器给出，不参与布局计算。
        (window.request_layout(Style::default(), None, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        if self.segments.is_empty() || self.cell_range.is_empty() {
            return;
        }

        let Ok(lines) = window.text_system().shape_text(
            self.text.clone(),
            self.font_size,
            &self.text_runs,
            None,
            None,
        ) else {
            return;
        };
        let Some(line) = lines.first() else {
            return;
        };

        for quad in collect_quads(
            line,
            bounds,
            self.line_height,
            &self.cell_range,
            &self.segments,
            self.selection_color,
        ) {
            window.paint_quad(quad);
        }
    }
}

fn collect_quads(
    line: &gpui::WrappedLine,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    cell_range: &Range<usize>,
    segments: &[HighlightSegment],
    selection_color: Hsla,
) -> Vec<gpui::PaintQuad> {
    let layout = &line.unwrapped_layout;
    let cell_start = cell_range.start;
    let cell_end = cell_range.end;
    if cell_start >= cell_end {
        return Vec::new();
    }

    // 单元格内容从自己左上角开始，所以以 cell_start 为原点。
    let origin_x = layout.x_for_index(cell_start);
    let top = bounds.top();
    let mut quads = Vec::new();

    for segment in segments {
        let start = segment.char_range.start.max(cell_start);
        let end = segment.char_range.end.min(cell_end);
        if start >= end {
            continue;
        }

        let left = bounds.left() + (layout.x_for_index(start) - origin_x);
        let right = bounds.left() + (layout.x_for_index(end) - origin_x);
        if left >= right {
            continue;
        }

        let mut quad = fill(
            Bounds::from_corners(point(left, top), point(right, top + line_height)),
            find_highlight_color(selection_color, segment.is_current),
        );
        quad.corner_radii = Corners::all(MATCH_RADIUS);
        quads.push(quad);
    }

    quads
}
