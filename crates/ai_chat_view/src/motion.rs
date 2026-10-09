//! 面板级进场动效。
//!
//! 只做到「整块面板」这一粒度：滑入 + 淡入。逐元素/逐段落做要自研渲染管线，
//! 收益也远不如这两个面板的显眼程度。
//!
//! 位移和淡入都挂在 gpui 的 `with_animation` 上——`App::reduce_motion` 打开时
//! 它自己会跳到终态，不需要在这里分支判断。

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, ElementId, IntoElement, Pixels, Styled, ease_out_quint, px,
};

/// 面板滑入时长。
///
/// 200ms：比消息进场（180ms）略长一点——面板个头更大，位移若和消息一样快
/// 会显得「跳」；但也不能再长，那会挡住键盘操作。
pub const PANEL_SLIDE_DURATION: Duration = Duration::from_millis(200);

/// 面板默认起始位移（从上方落下）。
pub const PANEL_SLIDE_OFFSET: Pixels = px(8.0);

/// 给一块面板挂上「滑入 + 淡入」进场动画。
///
/// `offset` 是起始位移，随进度归零：正数从下方浮起（浮层从内容里长出来），
/// 负数从上方落下（下拉式面板）。`id` 必须稳定，否则每帧重放。
pub fn panel_entrance(
    panel: impl Styled + IntoElement + 'static,
    id: impl Into<ElementId>,
    offset: Pixels,
) -> AnyElement {
    panel
        .with_animation(
            id,
            Animation::new(PANEL_SLIDE_DURATION).with_easing(ease_out_quint()),
            move |panel, delta| panel.opacity(delta).top(offset * (1.0 - delta)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slide_duration_stays_in_the_short_range() {
        // 超过 250ms 的转场会在快速开合（cmd-p 连按）时糊成一片。
        assert!(PANEL_SLIDE_DURATION >= Duration::from_millis(120));
        assert!(PANEL_SLIDE_DURATION <= Duration::from_millis(250));
    }
}
