//! 数据库对象列表的批量选择模型。
//!
//! 全选、Shift 区间选择和鼠标拖选都作用在**可见行**（过滤后的行）上，
//! 但删除等批量操作针对的是真实节点，所以选择集必须以“可见行序号”为基准，
//! 由调用方再用 `filtered_rows` 映射回原始行。
//!
//! 拖选需要区分“尚未拖动”和“拖到同一行”，因此这里把按下点、拖选锚点
//! 和最近一次悬停行显式建模，避免把单击误判成拖选。

use std::collections::HashSet;

/// 鼠标行选择的一次交互状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowDragState {
    /// 已按下但尚未判定是否为拖选
    pressed: Option<usize>,
    /// 拖选锚点行
    anchor: Option<usize>,
    /// 最近一次悬停行
    last_hovered: Option<usize>,
}

impl RowDragState {
    pub fn pressed(&self) -> Option<usize> {
        self.pressed
    }

    pub fn is_dragging(&self) -> bool {
        self.anchor.is_some()
    }

    /// 记录按下行。已处于拖选中时忽略，避免拖拽过程中重新起锚。
    pub fn press(&mut self, row_ix: usize, additive: bool) -> PressOutcome {
        if self.is_dragging() {
            return PressOutcome::Ignored;
        }
        self.pressed = Some(row_ix);
        self.last_hovered = Some(row_ix);
        PressOutcome::Pressed { additive }
    }

    /// 悬停到某行。返回是否首次进入拖选状态。
    pub fn hover(&mut self, row_ix: usize) -> bool {
        if self.pressed.is_none() || self.last_hovered == Some(row_ix) {
            return false;
        }
        self.last_hovered = Some(row_ix);
        if self.anchor.is_none() {
            self.anchor = self.pressed;
            return true;
        }
        false
    }

    /// 结束交互，返回拖选锚点（若有）。
    pub fn release(&mut self) -> Option<usize> {
        let anchor = self.anchor.take();
        self.pressed = None;
        self.last_hovered = None;
        anchor
    }

    /// 取消拖选（例如失去焦点），不改变已确认的选择集。
    pub fn cancel(&mut self) {
        self.pressed = None;
        self.anchor = None;
        self.last_hovered = None;
    }
}

/// 判定「点击」升级为「拖选」的最小像素位移，避免手抖把单击变成区间选择。
pub const ROW_DRAG_THRESHOLD_PX: f32 = 3.0;

/// 指针相对按下点的位移是否已达到拖选阈值。
pub fn exceeds_drag_threshold(origin: (f32, f32), position: (f32, f32)) -> bool {
    let moved = (position.0 - origin.0)
        .abs()
        .max((position.1 - origin.1).abs());
    moved >= ROW_DRAG_THRESHOLD_PX
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressOutcome {
    /// 已记录按下点，但尚未确定是单击还是拖选
    Pressed { additive: bool },
    /// 拖选中，忽略这次按下
    Ignored,
}

/// 用锚点到目标行的整段区间替换选择集。
pub fn apply_row_range(selected: &mut HashSet<usize>, anchor: usize, target: usize) {
    selected.clear();
    let (start, end) = if anchor <= target {
        (anchor, target)
    } else {
        (target, anchor)
    };
    selected.extend(start..=end);
}

/// 全选当前可见行。
pub fn select_all_rows(selected: &mut HashSet<usize>, visible_row_count: usize) {
    selected.clear();
    selected.extend(0..visible_row_count);
}

/// 拖选结束时的最终选择集：从锚点到目标行的整段。
pub fn apply_drag_selection(
    selected: &mut HashSet<usize>,
    anchor: usize,
    target: usize,
    visible_row_count: usize,
) {
    if visible_row_count == 0 {
        selected.clear();
        return;
    }
    apply_row_range(
        selected,
        anchor.min(visible_row_count - 1),
        target.min(visible_row_count - 1),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_all_rows_covers_visible_rows_only() {
        let mut selected = HashSet::from([7]);
        select_all_rows(&mut selected, 3);
        assert_eq!(HashSet::from([0, 1, 2]), selected);
    }

    #[test]
    fn select_all_rows_clears_selection_when_list_is_empty() {
        let mut selected = HashSet::from([1, 2]);
        select_all_rows(&mut selected, 0);
        assert!(selected.is_empty());
    }

    #[test]
    fn apply_drag_selection_keeps_the_full_span_in_both_directions() {
        let mut selected = HashSet::new();
        apply_drag_selection(&mut selected, 4, 1, 10);
        assert_eq!(HashSet::from([1, 2, 3, 4]), selected);

        apply_drag_selection(&mut selected, 1, 4, 10);
        assert_eq!(HashSet::from([1, 2, 3, 4]), selected);
    }

    #[test]
    fn apply_drag_selection_clamps_to_visible_rows() {
        let mut selected = HashSet::new();
        apply_drag_selection(&mut selected, 0, 99, 4);
        assert_eq!(HashSet::from([0, 1, 2, 3]), selected);
    }

    #[test]
    fn drag_threshold_ignores_tiny_pointer_wobble() {
        assert!(!exceeds_drag_threshold((10.0, 10.0), (10.5, 10.5)));
        assert!(!exceeds_drag_threshold((10.0, 10.0), (12.9, 10.0)));
    }

    #[test]
    fn drag_threshold_triggers_on_either_axis() {
        assert!(exceeds_drag_threshold((10.0, 10.0), (13.0, 10.0)));
        assert!(exceeds_drag_threshold((10.0, 10.0), (10.0, 20.0)));
        assert!(!exceeds_drag_threshold((10.0, 10.0), (10.0, 12.9)));
    }

    #[test]
    fn press_outcome_records_additive_modifier() {
        let mut state = RowDragState::default();
        assert_eq!(
            PressOutcome::Pressed { additive: true },
            state.press(2, true)
        );
        assert_eq!(Some(2), state.pressed());
        assert!(!state.is_dragging());
    }

    #[test]
    fn hover_starts_dragging_once_and_keeps_the_original_anchor() {
        let mut state = RowDragState::default();
        state.press(3, false);

        assert!(state.hover(4));
        assert!(!state.hover(6));
        // 锚点保持按下行，悬停行持续跟随，拖选才能覆盖锚点到当前位置的整段
        assert_eq!(Some(3), state.pressed());
        assert_eq!(Some(6), state.last_hovered);
    }

    #[test]
    fn hover_without_press_never_starts_dragging() {
        let mut state = RowDragState::default();
        assert!(!state.hover(5));
        assert!(!state.is_dragging());
    }

    #[test]
    fn drag_ignores_a_second_press_until_release() {
        let mut state = RowDragState::default();
        state.press(1, false);
        state.hover(2);

        assert_eq!(PressOutcome::Ignored, state.press(5, false));
        assert_eq!(Some(1), state.pressed());
        assert_eq!(Some(1), state.release());
        assert!(!state.is_dragging());
        assert_eq!(
            PressOutcome::Pressed { additive: false },
            state.press(9, false)
        );
    }
}
