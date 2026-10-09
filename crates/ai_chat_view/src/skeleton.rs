//! 会话历史的骨架占位。
//!
//! 用在「面板已经打开、内容还没到」的那一段：ACP 连接正在
//! Starting / Initializing / CreatingSession，历史要靠 agent 回放，
//! 这段时间转录是空的。空面板会让人以为坏了，骨架屏则说明「在等数据」。
//!
//! 只在**确实在等**的时候画：[`crate::agent_view`] 那边把它挂在
//! 「转录为空 + ACP 还没 Ready + 没有在飞轮次」三个条件同时成立时，
//! 普通空会话不会被塞一堆假占位条。

use gpui::{
    Animation, AnimationExt, AnyElement, InteractiveElement, IntoElement, ParentElement, Styled,
    div, relative,
};
use gpui_component::v_flex;

use crate::message_view::{MessageListLayout, message_column};
use crate::theme::{AgentChatTheme, sp};

/// 骨架屏呼吸周期。
///
/// 1.2s 是「像在等」又不「像闪烁故障」的中间值：再快会像加载失败闪烁，
/// 再慢会像界面卡死。
const SKELETON_PULSE: std::time::Duration = std::time::Duration::from_millis(1200);

/// 一行里两条占位条的长度比例轮换表。
///
/// 长度必须有变化——等长的工字条看起来像表格，长短交错才像正文。
const BAR_WIDTHS: [f32; 6] = [0.92, 0.7, 0.85, 0.52, 0.78, 0.62];

/// 呼吸曲线：`0.45 → 1.0 → 0.45` 的三角波。
///
/// 不用线性往返：线性在两端会「顿」一下，看起来像掉帧。
fn skeleton_opacity(delta: f32) -> f32 {
    let wave = 1.0 - (2.0 * delta - 1.0).abs();
    0.45 + 0.55 * wave
}

/// 会话历史骨架屏：`rows` 组占位条。
///
/// 用 [`message_column`] 做容器，与真实转录同一套宽度/居中规则，
/// 从骨架切到真内容时不会跳宽度。
pub fn transcript_skeleton(
    theme: &AgentChatTheme,
    layout: MessageListLayout,
    rows: usize,
) -> AnyElement {
    let rows: Vec<AnyElement> = (0..rows).map(|index| skeleton_row(theme, index)).collect();
    let column = message_column(layout)
        .debug_selector(|| "ai-chat-transcript-skeleton".to_string())
        .py_6()
        .gap_4()
        .children(rows);
    // 整列挂在同一个动画 id 上：所有占位条同步呼吸，不会各闪各的。
    column
        .with_animation(
            "ai-chat-transcript-skeleton-pulse",
            Animation::new(SKELETON_PULSE).repeat(),
            |column, delta| column.opacity(skeleton_opacity(delta)),
        )
        .into_any_element()
}

/// 一组占位：两条长短不同的条，模拟一行文字。
fn skeleton_row(theme: &AgentChatTheme, index: usize) -> AnyElement {
    let first = BAR_WIDTHS[index % BAR_WIDTHS.len()];
    let second = BAR_WIDTHS[(index + 3) % BAR_WIDTHS.len()];
    v_flex()
        .w_full()
        .gap_2()
        .child(skeleton_bar(theme, first))
        .child(skeleton_bar(theme, second))
        .into_any_element()
}

/// 占位条高度（px 基准）。
///
/// 12px 接近正文行高的一半，能读出「这里有一行字」但不抢眼。
const SKELETON_BAR_HEIGHT: f32 = 12.0;

fn skeleton_bar(theme: &AgentChatTheme, fraction: f32) -> AnyElement {
    div()
        .h(sp(SKELETON_BAR_HEIGHT))
        .w(relative(fraction))
        .rounded_md()
        .bg(theme.skeleton)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulse_stays_inside_the_visible_band() {
        // 两端与峰值的取值：不能到 0（看起来像内容消失），也不超过 1。
        assert!((skeleton_opacity(0.0) - 0.45).abs() < f32::EPSILON);
        assert!((skeleton_opacity(1.0) - 0.45).abs() < f32::EPSILON);
        assert!((skeleton_opacity(0.5) - 1.0).abs() < f32::EPSILON);
        for step in 0..=100 {
            let value = skeleton_opacity(step as f32 / 100.0);
            assert!((0.45..=1.0).contains(&value), "越界: {value}");
        }
    }

    #[test]
    fn bar_widths_are_not_all_equal() {
        // 等长的占位条看着像表格，不像正文。
        let unique: std::collections::BTreeSet<u32> = BAR_WIDTHS
            .iter()
            .map(|width| (width * 1000.0) as u32)
            .collect();
        assert!(unique.len() >= 4, "长度档位太少: {BAR_WIDTHS:?}");
        assert!(BAR_WIDTHS.iter().all(|width| *width > 0.0 && *width <= 1.0));
    }

    #[test]
    fn bar_height_is_a_scalable_rem_value() {
        // 12px 基准 = 12/14 rem：跟着 ui_scale 走，不是写死像素。
        assert!(sp(SKELETON_BAR_HEIGHT).to_pixels(gpui::px(14.0)) > gpui::px(0.0));
    }
}
