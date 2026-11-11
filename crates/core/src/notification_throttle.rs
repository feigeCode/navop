//! 相同通知的短时间节流。
//!
//! 故障场景下同一条错误可能在极短时间内重复产生（典型：远端会话断开后反复
//! 读目录）。弹窗层没有去重能力，用户既看不清也关不掉，因此由调用方在推送前
//! 过一层节流：**只按「同一份文案 + 时间窗」去重**，不同文案必须照常显示。
//!
//! 被节流的调用不会刷新时间戳，否则持续报错会把自己永久压制住。

use std::time::{Duration, Instant};

/// 同一份通知文案在时间窗内最多显示一次。
#[derive(Debug, Clone)]
pub struct NotificationThrottle {
    window: Duration,
    last: Option<(String, Instant)>,
}

impl NotificationThrottle {
    pub fn new(window: Duration) -> Self {
        Self { window, last: None }
    }

    /// 用显式时间点判断是否应展示，便于测试。
    pub fn should_show_at(&mut self, message: &str, now: Instant) -> bool {
        if let Some((last_message, last_at)) = self.last.as_ref()
            && last_message == message
            && now.saturating_duration_since(*last_at) < self.window
        {
            return false;
        }

        self.last = Some((message.to_string(), now));
        true
    }

    pub fn should_show(&mut self, message: &str) -> bool {
        self.should_show_at(message, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::NotificationThrottle;
    use std::time::{Duration, Instant};

    #[test]
    fn identical_messages_inside_the_window_are_suppressed() {
        let mut throttle = NotificationThrottle::new(Duration::from_secs(8));
        let start = Instant::now();

        assert!(throttle.should_show_at("读取目录失败", start));
        assert!(!throttle.should_show_at("读取目录失败", start + Duration::from_secs(1)));
        assert!(!throttle.should_show_at("读取目录失败", start + Duration::from_secs(7)));
        assert!(throttle.should_show_at("读取目录失败", start + Duration::from_secs(8)));
    }

    #[test]
    fn different_messages_are_never_suppressed() {
        let mut throttle = NotificationThrottle::new(Duration::from_secs(8));
        let start = Instant::now();

        assert!(throttle.should_show_at("读取目录失败", start));
        assert!(throttle.should_show_at("连接失败", start + Duration::from_millis(1)));
        assert!(throttle.should_show_at("读取目录失败", start + Duration::from_millis(2)));
    }

    #[test]
    fn suppressed_calls_do_not_extend_the_window() {
        let mut throttle = NotificationThrottle::new(Duration::from_secs(8));
        let start = Instant::now();

        assert!(throttle.should_show_at("读取目录失败", start));
        // 持续报错不能靠不断命中节流把通知永久压住。
        for second in 1..8 {
            assert!(!throttle.should_show_at("读取目录失败", start + Duration::from_secs(second)));
        }
        assert!(throttle.should_show_at("读取目录失败", start + Duration::from_secs(8)));
    }
}
