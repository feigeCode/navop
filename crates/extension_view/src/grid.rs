//! 扩展卡片网格几何：与主页连接卡同一套列宽策略，保证缩放与窄窗口行为一致。

use gpui::{Pixels, px};

/// 卡片最小可用列宽（rem）。
const CARD_MIN_WIDTH_REMS: f32 = 16.5;
/// 卡片最大列宽（rem），超过后留白而不继续拉宽。
const CARD_MAX_WIDTH_REMS: f32 = 24.0;
/// 网格行列间距（rem），与卡片 `gap_4` 对齐。
const GRID_GAP_REMS: f32 = 1.0;

/// 由内容区可用宽度计算列数与单列卡片宽度。
pub(crate) fn card_grid_metrics(content_width: Pixels, rem: Pixels) -> (usize, Pixels) {
    let min_width = rem * CARD_MIN_WIDTH_REMS;
    let max_width = rem * CARD_MAX_WIDTH_REMS;
    let gap = rem * GRID_GAP_REMS;
    let available = f32::from(content_width).max(0.0);
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
    fn common_widths_use_multi_column_grid() {
        let (columns, card_width) = metrics(1100.0, 16.0);
        assert!(columns >= 2, "expected multi-column at 1100, got {columns}");
        assert!(card_width >= 264.0 - 0.01);
        assert!(card_width <= 384.0 + 0.01);
    }

    #[test]
    fn narrow_content_collapses_to_single_column() {
        let (columns, card_width) = metrics(280.0, 16.0);
        assert_eq!(1, columns);
        assert!((card_width - 280.0).abs() < 0.01);
    }

    #[test]
    fn columns_scale_with_rem_zoom() {
        let (zoomed, _) = metrics(1100.0, 24.0);
        let (base, _) = metrics(1100.0, 16.0);
        assert!(zoomed < base);
    }
}
