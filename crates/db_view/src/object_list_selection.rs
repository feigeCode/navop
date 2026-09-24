//! 数据库对象列表的批量选择模型。
//!
//! 全选、Shift 区间选择和鼠标拖选都作用在**可见行**（过滤后的行）上，
//! 但删除等批量操作针对的是真实节点，所以选择集必须以“可见行序号”为基准，
//! 由调用方再用 `filtered_rows` 映射回原始行。
//!
//! 拖选需要区分“尚未拖动”和“拖到同一行”，因此这里把按下行、拖选锚点和
//! 加选基准快照显式建模，避免把单击误判成拖选。
//!
//! 注意：gpui 的鼠标事件只发给指针命中的元素，指针在列表外松开时视图收不到
//! `mouse_up`（详见 `DatabaseObjects::end_row_drag` 的说明），所以每次按下都
//! 必须能重新起锚，模型不依赖“一定会收到释放事件”。

use std::collections::HashSet;

/// 鼠标行选择的一次交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowDragState {
    /// 本次按下的行；为 `None` 表示没有进行中的按下或拖选
    pressed: Option<usize>,
    /// 拖选锚点行，进入拖选后固定为按下行
    anchor: Option<usize>,
    /// Cmd/Ctrl 加选时按下瞬间的选择集快照，拖选区间叠加在它之上
    additive_base: Option<HashSet<usize>>,
}

impl RowDragState {
    pub fn pressed(&self) -> Option<usize> {
        self.pressed
    }

    pub fn is_dragging(&self) -> bool {
        self.anchor.is_some()
    }

    /// 记录按下行，开始一次新的交互。
    ///
    /// 这里无条件丢弃上一次交互的锚点：指针在列表外松开时拿不到 `mouse_up`，
    /// 沿用旧锚点会让下一次拖选截出错误的区间。
    pub fn press(&mut self, row_ix: usize, additive_base: Option<HashSet<usize>>) {
        self.pressed = Some(row_ix);
        self.anchor = None;
        self.additive_base = additive_base;
    }

    /// 进入拖选状态：把按下行确立为锚点，返回锚点行。
    pub fn start_drag(&mut self) -> Option<usize> {
        if self.anchor.is_none() {
            self.anchor = self.pressed;
        }
        self.anchor
    }

    /// 拖选到 `target` 行时重算选择集。
    ///
    /// 普通拖选替换整个选择集；Cmd/Ctrl 加选时在按下快照之上叠加区间，
    /// 这样来回拖动时区间能正常收缩，而不是只增不减。
    pub fn apply_drag_to(
        &self,
        target: usize,
        selected: &mut HashSet<usize>,
        visible_row_count: usize,
    ) {
        let Some(anchor) = self.anchor else {
            return;
        };
        let span = VisibleRowSpan::new(anchor, target, visible_row_count);
        match self.additive_base.as_ref() {
            Some(base) => extend_with_span(selected, base, span),
            None => replace_with_span(selected, span),
        }
    }

    /// 结束交互，返回拖选锚点（若有）。
    pub fn release(&mut self) -> Option<usize> {
        let anchor = self.anchor.take();
        *self = Self::default();
        anchor
    }

    /// 取消拖选（例如失去焦点），不改变已确认的选择集。
    pub fn cancel(&mut self) {
        *self = Self::default();
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

/// 一段连续的可见行（闭区间），构造时已 clamp 到可见行范围。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibleRowSpan {
    start: usize,
    end: usize,
}

impl VisibleRowSpan {
    /// 由锚点行与目标行构造；列表为空时返回 `None`。
    pub fn new(anchor: usize, target: usize, visible_row_count: usize) -> Option<Self> {
        let last = visible_row_count.checked_sub(1)?;
        let anchor = anchor.min(last);
        let target = target.min(last);
        Some(Self {
            start: anchor.min(target),
            end: anchor.max(target),
        })
    }
}

/// 用区间替换选择集；`span` 为 `None`（列表为空）时清空选择。
pub fn replace_with_span(selected: &mut HashSet<usize>, span: Option<VisibleRowSpan>) {
    selected.clear();
    if let Some(span) = span {
        selected.extend(span.start..=span.end);
    }
}

/// 在 `base` 之上叠加区间（Cmd/Ctrl 加选拖选）。
pub fn extend_with_span(
    selected: &mut HashSet<usize>,
    base: &HashSet<usize>,
    span: Option<VisibleRowSpan>,
) {
    selected.clone_from(base);
    if let Some(span) = span {
        selected.extend(span.start..=span.end);
    }
}

/// 全选当前可见行。
pub fn select_all_rows(selected: &mut HashSet<usize>, visible_row_count: usize) {
    selected.clear();
    selected.extend(0..visible_row_count);
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
    fn replace_with_span_keeps_the_full_span_in_both_directions() {
        let mut selected = HashSet::new();
        replace_with_span(&mut selected, VisibleRowSpan::new(4, 1, 10));
        assert_eq!(HashSet::from([1, 2, 3, 4]), selected);

        replace_with_span(&mut selected, VisibleRowSpan::new(1, 4, 10));
        assert_eq!(HashSet::from([1, 2, 3, 4]), selected);
    }

    #[test]
    fn replace_with_span_clamps_to_visible_rows() {
        let mut selected = HashSet::new();
        replace_with_span(&mut selected, VisibleRowSpan::new(0, 99, 4));
        assert_eq!(HashSet::from([0, 1, 2, 3]), selected);
    }

    #[test]
    fn visible_row_span_requires_a_non_empty_list() {
        assert_eq!(None, VisibleRowSpan::new(0, 3, 0));
    }

    #[test]
    fn replace_with_span_clears_selection_when_list_is_empty() {
        let mut selected = HashSet::from([1, 2]);
        replace_with_span(&mut selected, None);
        assert!(selected.is_empty());
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
    fn press_starts_from_clean_state_until_drag_begins() {
        let mut state = RowDragState::default();
        state.press(2, None);
        assert_eq!(Some(2), state.pressed());
        assert!(!state.is_dragging());
        assert_eq!(Some(2), state.start_drag());
        assert!(state.is_dragging());
    }

    #[test]
    fn press_re_anchors_after_a_missed_mouse_up() {
        let mut state = RowDragState::default();
        state.press(1, None);
        assert_eq!(Some(1), state.start_drag());

        // 指针在列表外松开：视图收不到 mouse_up，随后直接在另一行按下
        state.press(5, None);
        assert_eq!(Some(5), state.start_drag());
    }

    #[test]
    fn drag_without_press_never_starts_dragging() {
        let mut state = RowDragState::default();
        assert_eq!(None, state.start_drag());
        assert!(!state.is_dragging());
    }

    #[test]
    fn plain_drag_replaces_the_selection() {
        let mut selected = HashSet::from([7]);
        let mut state = RowDragState::default();
        state.press(4, None);
        state.start_drag();

        state.apply_drag_to(1, &mut selected, 10);
        assert_eq!(HashSet::from([1, 2, 3, 4]), selected);
    }

    #[test]
    fn additive_drag_keeps_the_press_time_snapshot() {
        let mut selected = HashSet::from([7]);
        let mut state = RowDragState::default();
        state.press(1, Some(selected.clone()));
        state.start_drag();

        state.apply_drag_to(3, &mut selected, 10);
        assert_eq!(HashSet::from([1, 2, 3, 7]), selected);

        // 往回拖时区间收缩，但快照里的第 7 行不会被丢掉
        state.apply_drag_to(2, &mut selected, 10);
        assert_eq!(HashSet::from([1, 2, 7]), selected);
    }

    #[test]
    fn drag_on_an_empty_list_clears_the_selection() {
        let mut selected = HashSet::from([3]);
        let mut state = RowDragState::default();
        state.press(0, None);
        state.start_drag();

        state.apply_drag_to(0, &mut selected, 0);
        assert!(selected.is_empty());
    }

    #[test]
    fn release_clears_the_interaction_and_returns_the_anchor() {
        let mut state = RowDragState::default();
        state.press(1, Some(HashSet::from([1])));
        state.start_drag();

        assert_eq!(Some(1), state.release());
        assert!(!state.is_dragging());
        assert_eq!(None, state.pressed());
        assert_eq!(None, state.release());
    }
}
