//! 命中高亮的几何测试
//!
//! 这里不依赖真实字体：`x_for_index` 由一个忠实复刻 gpui 语义的假查询注入
//! （见 [`glyph_x`]），从而把「字符下标 vs 字节下标」这个问题确定性地钉住。
//!
//! 被复刻的真实语义：`LineLayout::x_for_index(byte_index)` 返回「起始字节下标
//! ≥ `byte_index`」的第一个字形的位置，全都越界时返回整行宽度——`gpui-pre`
//! `fork-0.3.111` 的 `src/text_system/line_layout.rs:108-117`。注意它比对的是
//! **字节**下标（同文件 `LineLayout::len` 的文档写明 "in utf-8 bytes"）。

use gpui::{Pixels, px};

use super::{HighlightSegment, highlight_bands};

/// 每个字符占 `char_width` 的假布局查询。
fn glyph_x(text: &'static str, char_width: f32) -> impl Fn(usize) -> Pixels {
    move |byte_index| {
        let mut position = 0.0f32;
        for (start_byte, _) in text.char_indices() {
            if start_byte >= byte_index {
                return px(position);
            }
            position += char_width;
        }
        px(position)
    }
}

/// 两个单元格 + 两个空格分隔符的整行文本。
const ROW: &str = "张三  北京市朝阳区";
const CELL_WIDTH: f32 = 10.0;

fn segment(char_range: std::ops::Range<usize>) -> HighlightSegment {
    HighlightSegment {
        char_range,
        is_current: true,
    }
}

#[test]
fn cjk_match_inside_a_cell_keeps_a_band_instead_of_collapsing() {
    // 命中第二个单元格里的「京」（整行第 5 个字符）。
    let bands = highlight_bands(glyph_x(ROW, CELL_WIDTH), ROW, &(4..10), &[segment(5..6)]);

    assert_eq!(
        1,
        bands.len(),
        "有命中就必须画出条带：把字符下标当字节下标用，区间会塌缩成 0 宽而被丢弃"
    );
    // 「京」是单元格内容的第 2 个字符：条带从 1 个字符宽处开始、宽 1 个字符。
    assert_eq!(px(CELL_WIDTH), bands[0].left);
    assert_eq!(px(CELL_WIDTH), bands[0].width);
}

#[test]
fn cjk_match_band_covers_every_matched_character() {
    // 「北京市」= 整行字符 4..7，三个 CJK 字符。
    let bands = highlight_bands(glyph_x(ROW, CELL_WIDTH), ROW, &(4..10), &[segment(4..7)]);

    assert_eq!(1, bands.len());
    assert_eq!(px(0.), bands[0].left);
    assert_eq!(px(3.0 * CELL_WIDTH), bands[0].width);
}

#[test]
fn ascii_match_band_is_unaffected_by_the_byte_conversion() {
    let row = "alice  30";
    // ASCII 下字符下标与字节下标相同，命中「30」必须覆盖两个字符。
    let bands = highlight_bands(glyph_x(row, CELL_WIDTH), row, &(7..9), &[segment(7..9)]);

    assert_eq!(1, bands.len());
    assert_eq!(px(0.), bands[0].left);
    assert_eq!(px(2.0 * CELL_WIDTH), bands[0].width);
}

#[test]
fn match_crossing_into_the_next_cell_is_clipped_to_this_cell() {
    let row = "张三  北京市朝阳区  电话";
    // 单元格 4..10 是「北京市朝阳区」；命中区间跨过分隔符伸到下一个单元格，
    // 只能画到本单元格右边界为止。
    let bands = highlight_bands(glyph_x(row, CELL_WIDTH), row, &(4..10), &[segment(9..13)]);

    assert_eq!(1, bands.len());
    // 「区」是单元格里的第 6 个字符，条带只覆盖它。
    assert_eq!(px(5.0 * CELL_WIDTH), bands[0].left);
    assert_eq!(px(CELL_WIDTH), bands[0].width);
}

#[test]
fn match_belonging_to_another_cell_draws_nothing_here() {
    let bands = highlight_bands(glyph_x(ROW, CELL_WIDTH), ROW, &(4..10), &[segment(0..2)]);
    assert!(bands.is_empty(), "别的单元格的命中不能画到本单元格里");
}

#[test]
fn bands_keep_the_current_match_flag() {
    let bands = highlight_bands(
        glyph_x(ROW, CELL_WIDTH),
        ROW,
        &(4..10),
        &[
            HighlightSegment {
                char_range: 4..5,
                is_current: false,
            },
            segment(5..6),
        ],
    );

    assert_eq!(2, bands.len());
    assert!(!bands[0].is_current);
    assert!(bands[1].is_current);
}
