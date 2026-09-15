//! 分类 chips 的单行折叠布局：宽度估算 → 计算可见数量 → 生成可见索引计划。

/// xsmall chip 字号估算基准（px）。
pub(crate) const CHIP_FONT_PX: f32 = 12.0;
/// chip 左右内边距合计（px）。
pub(crate) const CHIP_PADDING_PX: f32 = 24.0;
/// chip 间距（px）。
pub(crate) const CHIP_GAP_PX: f32 = 8.0;
/// 折叠态尾部“更多”按钮预留宽度（px）。
pub(crate) const CHIP_MORE_RESERVED_PX: f32 = 72.0;

/// 折叠后的 chip 行计划：可见索引 + 被折叠的数量。
pub(crate) struct ChipRowPlan {
    pub visible: Vec<usize>,
    pub hidden_count: usize,
}

/// 估算 chip 宽度：CJK 等全宽字符按整字宽、拉丁按 0.62 字宽，外加内边距。
pub(crate) fn estimate_chip_width(label: &str) -> f32 {
    let text: f32 = label
        .chars()
        .map(|c| {
            // CJK 及其他全宽字符的 UTF-8 编码 ≥ 3 字节，拉丁/数字/符号 ≤ 2 字节。
            if c.len_utf8() >= 3 {
                CHIP_FONT_PX
            } else {
                CHIP_FONT_PX * 0.62
            }
        })
        .sum();
    text + CHIP_PADDING_PX
}

/// 依次放入 chip，返回不溢出的数量（始终为“更多”按钮预留宽度）。
pub(crate) fn fitting_chip_count(widths: &[f32], available_width: f32) -> usize {
    let mut usable = available_width - CHIP_MORE_RESERVED_PX;
    let mut count = 0;
    for &width in widths {
        let cost = width + CHIP_GAP_PX;
        if cost > usable {
            break;
        }
        usable -= cost;
        count += 1;
    }
    count
}

/// 计算折叠计划：全部放得下则全显；放不下则截断到单行，
/// 若选中的 chip 被截掉，用它与最后一个可见位交换，保证激活项始终可见。
pub(crate) fn plan_chip_row(
    labels: &[&str],
    available_width: f32,
    selected: Option<usize>,
) -> ChipRowPlan {
    let widths: Vec<f32> = labels.iter().map(|label| estimate_chip_width(label)).collect();
    let total_width = widths.iter().sum::<f32>()
        + CHIP_GAP_PX * labels.len().saturating_sub(1) as f32;
    if total_width <= available_width {
        return ChipRowPlan {
            visible: (0..labels.len()).collect(),
            hidden_count: 0,
        };
    }
    let max_visible = fitting_chip_count(&widths, available_width).max(1);
    let visible = visible_chip_indices(labels.len(), max_visible, selected);
    ChipRowPlan {
        hidden_count: labels.len() - visible.len(),
        visible,
    }
}

fn visible_chip_indices(total: usize, max_visible: usize, selected: Option<usize>) -> Vec<usize> {
    if max_visible >= total {
        return (0..total).collect();
    }
    let mut visible: Vec<usize> = (0..max_visible).collect();
    if let Some(selected) = selected {
        if selected >= max_visible {
            let last = max_visible - 1;
            visible[last] = selected;
        }
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(labels: &[&str], available: f32, selected: Option<usize>) -> ChipRowPlan {
        plan_chip_row(labels, available, selected)
    }

    #[test]
    fn wide_chars_cost_more_than_latin() {
        assert!(estimate_chip_width("数据库驱动") > estimate_chip_width("Database"));
    }

    #[test]
    fn fitting_count_reserves_room_for_more_button() {
        // 可用 136px：扣掉“更多”预留 72px 后剩 64px，只放得下 1 个 chip（40+8=48）。
        let widths = vec![40.0, 40.0, 40.0];
        assert_eq!(1, fitting_chip_count(&widths, 40.0 * 3.0 + 8.0 * 2.0));
        // 备足余量时三个全部放下。
        assert_eq!(3, fitting_chip_count(&widths, 40.0 * 3.0 + 8.0 * 2.0 + 200.0));
    }

    #[test]
    fn plan_shows_all_when_row_fits() {
        let result = plan(&["全部", "语言"], 10_000.0, None);
        assert_eq!(vec![0, 1], result.visible);
        assert_eq!(0, result.hidden_count);
    }

    #[test]
    fn plan_truncates_and_reports_hidden_count() {
        let result = plan(&["全部", "语言", "语言包", "数据库驱动", "远程桌面"], 120.0, None);
        assert!(result.hidden_count >= 1, "溢出行应折叠至少一个 chip");
        assert!(result.visible.len() < 5);
        assert_eq!(
            (0..result.visible.len()).collect::<Vec<_>>(),
            result.visible,
            "折叠态应保留前缀顺序"
        );
    }

    #[test]
    fn plan_keeps_selected_chip_visible_by_swapping_into_last_slot() {
        let labels = ["全部", "语言", "语言包", "数据库驱动", "远程桌面"];
        let result = plan(&labels, 120.0, Some(4));
        assert!(result.visible.contains(&4), "选中的 chip 必须可见");
        assert_eq!(labels.len() - result.visible.len(), result.hidden_count);
    }

    #[test]
    fn plan_selected_within_visible_prefix_keeps_order() {
        // 宽度足够容纳前缀两个 chip（48+56+“更多”72+间距 < 200）。
        let labels = ["全部", "语言", "语言包", "数据库驱动"];
        let result = plan(&labels, 200.0, Some(1));
        assert_eq!(Some(&0), result.visible.first());
        assert!(result.visible.contains(&1));
    }

    #[test]
    fn tiny_available_width_still_shows_one_chip() {
        let result = plan(&["全部", "语言"], 10.0, None);
        assert_eq!(1, result.visible.len());
    }
}
