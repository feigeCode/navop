use std::collections::VecDeque;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(10 * 60);
const MAX_RESTARTS: usize = 3;

/// 崩溃重启滑动窗口；第四次崩溃会关闭本次运行的内置路径。
#[derive(Default)]
pub struct RestartBudget {
    crashes: VecDeque<Instant>,
}

impl RestartBudget {
    pub fn record_crash(&mut self, now: Instant) -> bool {
        while self
            .crashes
            .front()
            .is_some_and(|time| now.saturating_duration_since(*time) >= WINDOW)
        {
            self.crashes.pop_front();
        }
        if self.crashes.len() >= MAX_RESTARTS {
            return false;
        }
        self.crashes.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests;
