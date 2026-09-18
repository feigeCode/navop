//! 会话滚动的纯状态机。
//!
//! 三态 + 两条边界纪律（照 Waku 的实现结论）：
//! - 未知不能当 `false`：测不到尾行时 [`TranscriptScrollState::should_show_scroll_to_bottom`] 返回 `None`，
//!   调用方沿用上一帧答案，否则按钮会按帧闪烁。
//! - 未测量高度不是 0：尾行高度缺失时沿用上次 `end_space`，否则会被读成「回复已填满视口」。

use gpui::Pixels;

/// 距底部多少像素以内仍算「贴着尾巴」。
pub const FOLLOW_THRESHOLD: Pixels = gpui::px(24.0);
/// 尾行未测量时保留的上一帧 `end_space`。
pub const UNMEASURED_END_SPACE: Pixels = gpui::px(0.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowState {
    /// 跟随流式输出。
    FollowingTail,
    /// 用户在读历史；期间不得自动跳转。
    ReadingHistory,
    /// 正在执行一次跳转（锚点定位）；落地前同样不跟随。
    JumpingToAnchor,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TranscriptScrollState {
    follow: FollowState,
    /// 上一帧的「是否显示回到最新」答案；`None` 表示还没有有效测量。
    last_visible_answer: Option<bool>,
    /// 上一次有效测量到的尾部留白。
    end_space: Pixels,
    /// 滚动条是否正被拖拽。
    dragging_scrollbar: bool,
    /// 上一帧观测到的内容可滚动量（`max_offset`）。
    last_max_offset: Pixels,
    /// 是否已经观测过至少一帧。
    observed: bool,
}

impl Default for TranscriptScrollState {
    fn default() -> Self {
        Self {
            follow: FollowState::FollowingTail,
            last_visible_answer: None,
            end_space: UNMEASURED_END_SPACE,
            dragging_scrollbar: false,
            last_max_offset: UNMEASURED_END_SPACE,
            observed: false,
        }
    }
}

impl TranscriptScrollState {
    pub fn follow_state(&self) -> FollowState {
        self.follow
    }

    pub fn is_following(&self) -> bool {
        self.follow == FollowState::FollowingTail
    }

    /// 内容滚动后的状态更新：贴近底部重新跟随，否则进入阅读历史。
    pub fn on_scroll(&mut self, distance_from_bottom: Pixels) {
        if self.dragging_scrollbar {
            self.follow = FollowState::ReadingHistory;
            return;
        }
        self.follow = if distance_from_bottom <= FOLLOW_THRESHOLD {
            FollowState::FollowingTail
        } else {
            FollowState::ReadingHistory
        };
    }

    /// 滚动条拖拽开始/结束。拖拽期间强制脱离跟随，松手后再判一次。
    pub fn set_scrollbar_dragging(&mut self, dragging: bool, distance_from_bottom: Pixels) {
        self.dragging_scrollbar = dragging;
        if dragging {
            self.follow = FollowState::ReadingHistory;
        } else {
            self.on_scroll(distance_from_bottom);
        }
    }

    /// 用户点击「回到最新」或显式跳转结束后调用。
    pub fn jump_to_tail(&mut self) {
        self.follow = FollowState::FollowingTail;
    }

    /// 开始一次锚点跳转。
    pub fn begin_anchor_jump(&mut self) {
        self.follow = FollowState::JumpingToAnchor;
    }

    /// 「回到最新」按钮是否可见：`None` 表示本帧测不到，调用方沿用上一帧。
    pub fn should_show_scroll_to_bottom(&mut self, measurement: Option<bool>) -> Option<bool> {
        match measurement {
            Some(value) => {
                self.last_visible_answer = Some(value);
                Some(value)
            }
            None => self.last_visible_answer,
        }
    }

    /// 计算尾部留白：测量缺失时沿用上次的有效值（不是 0）。
    pub fn end_space(&mut self, measured: Option<Pixels>) -> Pixels {
        if let Some(measured) = measured {
            self.end_space = measured;
        }
        self.end_space
    }

    /// 观测一帧滚动几何，返回「是否显示回到最新」。
    ///
    /// `offset` 为当前滚动位置，`max_offset` 为可滚动总量（内容高度 − 视口高度）。
    ///
    /// **核心纪律：内容变长不算用户滚动。** 流式输出每一帧都会让 `max_offset` 变大；
    /// 若只看「距底距离」就会把每一帧都判成「用户在读历史」，于是自动跟随被自己关掉，
    /// 表现为「回复到一半就不往下滚了」。所以只有 `max_offset` 没变而位置变了，
    /// 才认定为用户移动了阅读位置。
    pub fn observe(&mut self, offset: Pixels, max_offset: Pixels) -> Option<bool> {
        let distance = (max_offset - offset).max(gpui::px(0.0));
        let content_grew = self.observed && max_offset > self.last_max_offset;
        let first_observation = !self.observed;

        self.observed = true;
        self.last_max_offset = max_offset;

        if first_observation || !content_grew || self.dragging_scrollbar {
            self.on_scroll(distance);
        }

        let show = self.follow != FollowState::FollowingTail;
        self.should_show_scroll_to_bottom(Some(show))
    }

    /// 是否存在内容延伸之外的「用户移动」信号（供测试与调试读取）。
    pub fn last_max_offset(&self) -> Pixels {
        self.last_max_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn near_bottom_keeps_following_tail() {
        let mut state = TranscriptScrollState::default();

        state.on_scroll(px(4.0));

        assert_eq!(FollowState::FollowingTail, state.follow_state());
        assert!(state.is_following());
    }

    #[test]
    fn leaving_bottom_switches_to_reading_history() {
        let mut state = TranscriptScrollState::default();

        state.on_scroll(px(400.0));

        assert_eq!(FollowState::ReadingHistory, state.follow_state());
        assert!(!state.is_following());
    }

    #[test]
    fn scrollbar_drag_forces_reading_and_resumes_after_release() {
        let mut state = TranscriptScrollState::default();

        state.set_scrollbar_dragging(true, px(0.0));
        assert_eq!(FollowState::ReadingHistory, state.follow_state());

        state.set_scrollbar_dragging(false, px(2.0));
        assert!(state.is_following());
    }

    #[test]
    fn unknown_measurement_reuses_previous_answer() {
        let mut state = TranscriptScrollState::default();

        assert_eq!(None, state.should_show_scroll_to_bottom(None));
        assert_eq!(Some(true), state.should_show_scroll_to_bottom(Some(true)));
        assert_eq!(Some(true), state.should_show_scroll_to_bottom(None));
        assert_eq!(Some(false), state.should_show_scroll_to_bottom(Some(false)));
        assert_eq!(Some(false), state.should_show_scroll_to_bottom(None));
    }

    #[test]
    fn missing_measurement_preserves_last_end_space() {
        let mut state = TranscriptScrollState::default();

        assert_eq!(px(120.0), state.end_space(Some(px(120.0))));
        assert_eq!(px(120.0), state.end_space(None));
        assert_eq!(px(48.0), state.end_space(Some(px(48.0))));
    }

    #[test]
    fn anchor_jump_does_not_follow_until_it_lands() {
        let mut state = TranscriptScrollState::default();

        state.begin_anchor_jump();
        assert_eq!(FollowState::JumpingToAnchor, state.follow_state());

        state.on_scroll(px(900.0));
        assert_eq!(FollowState::ReadingHistory, state.follow_state());

        state.jump_to_tail();
        assert!(state.is_following());
    }

    #[test]
    fn content_growth_alone_does_not_break_following() {
        let mut state = TranscriptScrollState::default();

        // 首帧：内容还没超出视口，贴着尾巴。
        assert_eq!(Some(false), state.observe(px(0.0), px(0.0)));

        // 流式输出让内容变长，但自动跟随还没来得及落地 —— 此刻距底很远。
        assert_eq!(Some(false), state.observe(px(0.0), px(600.0)));
        assert!(
            state.is_following(),
            "内容变长本身不能把跟随态关掉，否则回复到一半就停止下滚"
        );

        // 自动跟随落地后仍在底部。
        assert_eq!(Some(false), state.observe(px(600.0), px(600.0)));
        assert!(state.is_following());
    }

    #[test]
    fn user_scroll_without_content_growth_breaks_following() {
        let mut state = TranscriptScrollState::default();
        state.observe(px(0.0), px(600.0));
        state.observe(px(600.0), px(600.0));

        // 内容长度不变，位置回退 —— 用户主动上翻。
        let show = state.observe(px(200.0), px(600.0));

        assert_eq!(Some(true), show);
        assert_eq!(FollowState::ReadingHistory, state.follow_state());
    }

    #[test]
    fn read_history_survives_further_content_growth() {
        let mut state = TranscriptScrollState::default();
        state.observe(px(0.0), px(600.0));
        state.observe(px(100.0), px(600.0));
        assert!(!state.is_following());

        // 用户正在读历史；期间新内容继续追加，不得把他拽回底部。
        assert_eq!(Some(true), state.observe(px(100.0), px(900.0)));
        assert!(!state.is_following());
    }

    #[test]
    fn jumping_to_tail_after_content_growth_resumes_following() {
        let mut state = TranscriptScrollState::default();
        state.observe(px(0.0), px(600.0));
        state.observe(px(10.0), px(600.0));
        assert!(!state.is_following());

        state.jump_to_tail();
        let show = state.observe(px(900.0), px(900.0));

        assert_eq!(Some(false), show);
        assert!(state.is_following());
    }
}
