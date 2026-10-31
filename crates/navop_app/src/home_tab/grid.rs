//! 首页卡片网格几何：列数/列宽由内容区共同父级一次性计算，
//! 所有分组共享同一组列边界（design-docs/navop-home-redesign §4.1）。

use gpui::{Pixels, px};

/// 卡片最小可用列宽（rem），与原卡片 `min_w(rems(15.5))` 一致。
const CARD_MIN_WIDTH_REMS: f32 = 15.5;
/// 卡片最大列宽（rem），超过后留白而不继续拉宽卡片。
const CARD_MAX_WIDTH_REMS: f32 = 24.0;
/// 网格行列间距（rem），对应 `gap_3()`。
const GRID_GAP_REMS: f32 = 0.75;
/// 内容区左右内边距合计（rem），对应 `p_5()` 两侧。
const CONTENT_PADDING_REMS: f32 = 1.25 * 2.0;

/// 由内容区可用宽度计算共享网格的列数与单列卡片宽度。
///
/// `content_width` 已扣除侧边栏；`rem` 为当前 rem 基准字号，保证缩放正确。
pub(super) fn card_grid_metrics(content_width: Pixels, rem: Pixels) -> (usize, Pixels) {
    let min_width = rem * CARD_MIN_WIDTH_REMS;
    let max_width = rem * CARD_MAX_WIDTH_REMS;
    let gap = rem * GRID_GAP_REMS;
    let available = f32::from(content_width - rem * CONTENT_PADDING_REMS).max(0.0);
    let candidates = ((available + f32::from(gap)) / f32::from(min_width + gap)).floor() as usize;
    let columns = candidates.max(1);
    let card_width = (available - (columns - 1) as f32 * f32::from(gap)) / columns as f32;
    (
        columns,
        px(card_width
            .max(f32::from(min_width))
            .min(f32::from(max_width))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(width: f32, rem: f32) -> (usize, f32) {
        let (columns, card_width) = card_grid_metrics(px(width), px(rem));
        (columns, f32::from(card_width))
    }

    #[test]
    fn four_columns_at_common_window_width() {
        // 1440 窗口 - 222 侧栏 = 1218 内容宽；rem 16
        let (columns, card_width) = metrics(1218.0, 16.0);
        assert_eq!(columns, 4);
        // 列宽 = (1218 - 40 - 3*12) / 4
        assert!((card_width - 285.5).abs() < 0.01);
    }

    #[test]
    fn narrow_content_uses_single_column() {
        let (columns, card_width) = metrics(300.0, 16.0);
        assert_eq!(columns, 1);
        // 单列填满可用内容宽：300 - 2×1.25rem(=40) = 260
        assert!((card_width - 260.0).abs() < 0.01);
    }

    #[test]
    fn wide_content_adds_columns_below_width_cap() {
        // 达到列数阈值前先加列，单列宽度始终不低于最小值
        let (columns, card_width) = metrics(6000.0, 16.0);
        assert_eq!(columns, 22);
        assert!(card_width >= 248.0 && card_width < 285.0);

        for width in [200.0, 400.0, 999.0, 1218.0, 3840.0, 7999.0] {
            let (_, card_width) = metrics(width, 16.0);
            assert!(
                card_width >= 248.0 - 0.01,
                "width {width} too narrow: {card_width}"
            );
        }
    }

    #[test]
    fn columns_scale_with_rem_zoom() {
        // 150% 缩放下同一内容宽应减少列数
        let (zoomed_columns, _) = metrics(1218.0, 24.0);
        let (base_columns, _) = metrics(1218.0, 16.0);
        assert!(zoomed_columns < base_columns);
    }
}
