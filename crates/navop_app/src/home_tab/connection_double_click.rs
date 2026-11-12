//! 连接卡片 / 连接行的双击判定。
//!
//! GPUI 的 `click_count` 由平台按「两次点击之间的时间与坐标」计算，不区分第一次
//! 点击落在哪个元素上。快捷打开对话框关闭时，双击的第二次点击可能穿透到下方的
//! 首页卡片，平台依旧把它标记为双击，卡片就会再开一次连接——表现为「双击一条
//! 连接，开出两个 SSH 会话」（issue #461 修好单次点击重复之后仍然复现）。
//!
//! 这里记录卡片 / 行真正收到的点击：只有同一元素上紧邻的两次点击才算双击。

use std::time::{Duration, Instant};

/// 双击判定窗口。macOS 默认的双击间隔是 500ms，取同一上限即可。
const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(500);

/// 卡片 / 连接行的双击判定器。
#[derive(Default)]
pub(crate) struct ConnectionDoubleClickTracker {
    last_click: Option<(i64, Instant)>,
}

impl ConnectionDoubleClickTracker {
    /// 记录卡片 / 连接行收到的一次点击；临时连接没有 id，归并为 0。
    pub(crate) fn record_click(&mut self, connection_id: Option<i64>, now: Instant) {
        self.last_click = Some((connection_id.unwrap_or(0), now));
    }

    /// 当前点击是否为同一元素上紧邻的第二次点击。
    ///
    /// 调用方必须保证本次点击自己的 `record_click` 还没执行（渲染时
    /// `on_double_click` 先于 `on_click` 注册，GPUI 也按注册顺序派发），
    /// 否则读到的就是本次点击本身。
    pub(crate) fn is_double_click(&self, connection_id: Option<i64>, now: Instant) -> bool {
        let Some((last_id, last_click_at)) = self.last_click else {
            return false;
        };
        last_id == connection_id.unwrap_or(0)
            && now.saturating_duration_since(last_click_at) <= DOUBLE_CLICK_WINDOW
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_click_on_the_same_card_is_a_double_click() {
        let mut tracker = ConnectionDoubleClickTracker::default();
        let first = Instant::now();
        tracker.record_click(Some(7), first);

        assert!(tracker.is_double_click(Some(7), first + Duration::from_millis(180)));
    }

    #[test]
    fn first_click_that_landed_elsewhere_does_not_make_a_double_click() {
        let mut tracker = ConnectionDoubleClickTracker::default();
        let first = Instant::now();
        // 第一次点击落在别处（例如刚关闭的快捷打开对话框），第二次点击
        // 穿透到这张卡片时平台仍会给出 click_count == 2。
        tracker.record_click(Some(9), first);

        assert!(!tracker.is_double_click(Some(7), first + Duration::from_millis(180)));
    }

    #[test]
    fn click_without_a_previous_card_click_is_not_a_double_click() {
        let tracker = ConnectionDoubleClickTracker::default();

        assert!(!tracker.is_double_click(Some(7), Instant::now()));
    }

    #[test]
    fn stale_first_click_does_not_count() {
        let mut tracker = ConnectionDoubleClickTracker::default();
        let first = Instant::now();
        tracker.record_click(Some(7), first);

        assert!(!tracker.is_double_click(Some(7), first + Duration::from_millis(800)));
    }
}
