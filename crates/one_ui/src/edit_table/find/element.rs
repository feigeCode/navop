//! 命中高亮绘制元素
//!
//! 整行文本只 shape 一次，再按命中区间取出真实像素位置，
//! 这样 CJK 宽字符、比例字体和单元格内滚动都不会让高亮错位。
//!
//! ⚠️ 下标空间：gpui 的 `LineLayout::x_for_index` 与 `TextRun::len` 都按
//! **UTF-8 字节**下标定位（`LineLayout::len` 的文档写的是 "the length of the
//! line in utf-8 bytes"），而查找的匹配与计数按**字符**下标做。两者混用会让
//! 矩形落到别的字符上；当命中区间首尾落在同一个字形的字节范围内时，两端取到
//! 同一个 x、宽度变成 0 而被丢弃——表现就是「有命中却完全看不到高亮」。
//! 所以字符 → 字节的换算集中在 [`highlight_bands`] 里做。

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, Corners, Element, ElementId, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, LayoutId, Pixels, SharedString, Size, Style, TextRun, Window, fill, point, size,
};

use super::{MATCH_RADIUS, char_range_to_byte_range, find_highlight_color};

/// 一行里的高亮片段（整行文本中的**字符**区间 + 是否当前命中）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightSegment {
    pub char_range: Range<usize>,
    pub is_current: bool,
}

/// 一条命中高亮的横向条带。
///
/// `left` 是相对**单元格内容区左边界**的偏移，`width` 是条带宽度。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HighlightBand {
    pub left: Pixels,
    pub width: Pixels,
    pub is_current: bool,
}

/// 计算单元格内的所有命中条带。
///
/// `x_for_index` 是布局的「字节下标 → 像素 x」查询（生产路径传
/// `LineLayout::x_for_index`）。`cell_range` 与 `segments` 都是整行文本的
/// **字符**下标，这里先换算成字节下标再查询；落在单元格外的片段会被裁掉。
pub fn highlight_bands(
    x_for_index: impl Fn(usize) -> Pixels,
    text: &str,
    cell_range: &Range<usize>,
    segments: &[HighlightSegment],
) -> Vec<HighlightBand> {
    let cell = char_range_to_byte_range(text, cell_range.clone());
    if cell.start >= cell.end {
        return Vec::new();
    }

    // 单元格内容从自己左上角开始，所以以 cell.start 为原点。
    let origin_x = x_for_index(cell.start);
    let mut bands = Vec::new();

    for segment in segments {
        let range = char_range_to_byte_range(text, segment.char_range.clone());
        let start = range.start.max(cell.start);
        let end = range.end.min(cell.end);
        if start >= end {
            continue;
        }

        let left = x_for_index(start) - origin_x;
        let right = x_for_index(end) - origin_x;
        if left >= right {
            continue;
        }

        bands.push(HighlightBand {
            left,
            width: right - left,
            is_current: segment.is_current,
        });
    }

    bands
}

/// 把整行文本中的命中区间绘制成高亮底色。
///
/// `cell_range` 指定当前单元格在整行文本中的字符区间，条带的横向位置相对该
/// 单元格的内容区；纵向铺满元素自身高度——元素与 td 内容同处一个「内容区」
/// 盒子，所以 bounds 天然就是内容区。
pub struct FindHighlightElement {
    text: SharedString,
    font_size: Pixels,
    cell_range: Range<usize>,
    segments: Rc<Vec<HighlightSegment>>,
    selection_color: Hsla,
    text_runs: Rc<Vec<TextRun>>,
}

impl FindHighlightElement {
    pub fn new(
        text: SharedString,
        font_size: Pixels,
        cell_range: Range<usize>,
        segments: Rc<Vec<HighlightSegment>>,
        selection_color: Hsla,
        text_runs: Rc<Vec<TextRun>>,
    ) -> Self {
        Self {
            text,
            font_size,
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
        // 铺满宿主给的内容区盒子：paint 时既要拿到内容区左边界（横向原点），
        // 也要拿到内容区高度（条带高度），所以不能是 0×0 的叶子。
        (
            window.request_layout(
                Style {
                    size: Size::full(),
                    ..Style::default()
                },
                None,
                cx,
            ),
            (),
        )
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

        let layout = &line.unwrapped_layout;
        let bands = highlight_bands(
            |byte_index| layout.x_for_index(byte_index),
            &self.text,
            &self.cell_range,
            &self.segments,
        );

        for band in bands {
            let mut quad = fill(
                Bounds::new(
                    point(bounds.left() + band.left, bounds.top()),
                    size(band.width, bounds.size.height),
                ),
                find_highlight_color(self.selection_color, band.is_current),
            );
            quad.corner_radii = Corners::all(MATCH_RADIUS);
            window.paint_quad(quad);
        }
    }
}
