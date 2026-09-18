//! 逐行到达时间，供终端左边距的时间戳 / 行号列使用。
//!
//! 行号与渲染端共用一套坐标：以网格缓冲区的绝对行号作为 id，屏幕第 `r` 行的
//! id = `history_size + r - display_offset`，缓冲区底部固定为
//! `history_size + screen_lines - 1`。滚屏时 `history_size` 增长，id 随之增长，
//! 因此 id 在整个会话内单调递增，既是时间戳的键，也是显示的行号（id + 1）。
//!
//! 采样只读网格状态（不接管输出路径），在任何后端上行为一致：本地 PTY 由
//! alacritty 事件循环喂数据，SSH / Telnet / 串口走各自的 ingress，都无法在
//! 解析点插桩，但都会产生 Wakeup，于是统一在 Wakeup 处采样。
//!
//! 精度边界：
//! - 同一采样周期内到达的多行共享一个时间戳（周期为事件合并节流窗口，约 8ms）。
//! - 窗口尺寸变化触发 reflow 后，旧行的 id 与内容会错位；新行仍然准确。
//! - 备用屏幕（TUI）不记录。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use chrono::Local;

/// 单次采样看到的网格状态。调用方在持有 `Term` 锁时读取，然后立即释放。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineTimelineSample {
    pub history_size: usize,
    pub screen_lines: usize,
    /// 光标所在网格行：0 = 屏幕首行，负数 = 滚屏历史。
    pub cursor_line: i32,
    /// 备用屏幕没有滚屏历史，DECTCEM 之外的行号也没有意义，直接跳过采样。
    pub alternate_screen: bool,
    /// 滚屏历史上限，用于裁剪过期时间戳。
    pub scrollback_lines: usize,
}

/// 终端与视图共享的行时间轴句柄。
#[derive(Clone, Default)]
pub struct SharedLineTimeline(Arc<Mutex<LineTimeline>>);

impl SharedLineTimeline {
    /// 采样一次网格状态；空闲行不产生时间戳。
    pub fn observe(&self, sample: LineTimelineSample) {
        if let Ok(mut timeline) = self.0.lock() {
            timeline.observe(sample);
        }
    }

    /// 查询某个行 id 的时间戳标签（`HH:MM:SS`）。
    pub fn label(&self, id: i64) -> Option<String> {
        let timeline = self.0.lock().ok()?;
        timeline.label(id).map(str::to_string)
    }
}

#[derive(Default)]
struct LineTimeline {
    /// 按 id 升序排列的 `(行 id, 时间戳标签)`。
    stamps: VecDeque<(i64, Arc<str>)>,
    /// 下一个待打时间戳的行 id；`0` 表示尚未打过任何时间戳。
    next_id: i64,
    /// 上一次采样看到的屏幕行数，用于识别窗口尺寸变化。
    screen_lines: usize,
    /// 是否已采样过一次：首次采样不能当成窗口尺寸变化。
    initialized: bool,
}

impl LineTimeline {
    fn observe(&mut self, sample: LineTimelineSample) {
        let label: Arc<str> = Local::now().format("%H:%M:%S").to_string().into();
        self.observe_labelled(sample, label);
    }

    fn observe_labelled(&mut self, sample: LineTimelineSample, label: Arc<str>) {
        if sample.alternate_screen {
            self.screen_lines = sample.screen_lines;
            return;
        }

        let resized = self.initialized && sample.screen_lines != self.screen_lines;
        self.initialized = true;
        self.screen_lines = sample.screen_lines;

        let cursor_id = sample.history_size as i64 + sample.cursor_line as i64;

        if cursor_id + sample.screen_lines as i64 + 1 < self.next_id {
            // 滚屏历史被清空（`clear` / `ESC[3J`）：id 空间整体回退，
            // 行号从 1 重新开始，旧时间戳已无对应行。
            self.stamps.clear();
            self.next_id = 0;
        }

        if resized {
            // reflow 之后旧 id 与内容不再一一对应，只对齐游标、不为重排出的行补时间戳。
            self.next_id = self.next_id.max(cursor_id + 1);
            self.trim(sample);
            return;
        }

        if cursor_id >= self.next_id {
            for id in self.next_id..=cursor_id {
                self.stamps.push_back((id, label.clone()));
            }
            self.next_id = cursor_id + 1;
        }

        self.trim(sample);
    }

    /// 丢弃已经滚出保留窗口的时间戳。
    fn trim(&mut self, sample: LineTimelineSample) {
        let capacity = sample.scrollback_lines + sample.screen_lines + 1;
        let newest = (sample.history_size + sample.screen_lines) as i64 - 1;
        let keep_from = newest - capacity as i64;
        while let Some((id, _)) = self.stamps.front() {
            if *id < keep_from {
                self.stamps.pop_front();
            } else {
                break;
            }
        }
    }

    fn label(&self, id: i64) -> Option<&str> {
        let index = self
            .stamps
            .binary_search_by_key(&id, |(stamp_id, _)| *stamp_id)
            .ok()?;
        self.stamps.get(index).map(|(_, label)| label.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::{LineTimeline, LineTimelineSample, SharedLineTimeline};
    use std::sync::Arc;

    fn sample(history_size: usize, screen_lines: usize, cursor_line: i32) -> LineTimelineSample {
        LineTimelineSample {
            history_size,
            screen_lines,
            cursor_line,
            alternate_screen: false,
            scrollback_lines: 1000,
        }
    }

    fn label(text: &str) -> Arc<str> {
        Arc::from(text)
    }

    #[test]
    fn stamps_cursor_advance_without_renumbering_earlier_lines() {
        let mut timeline = LineTimeline::default();

        timeline.observe_labelled(sample(0, 24, 0), label("10:00:00"));
        timeline.observe_labelled(sample(0, 24, 3), label("10:00:01"));

        assert_eq!(Some("10:00:00"), timeline.label(0));
        assert_eq!(Some("10:00:01"), timeline.label(3));
        assert_eq!(None, timeline.label(5));
    }

    #[test]
    fn scrolling_stamps_new_bottom_lines_only() {
        let mut timeline = LineTimeline::default();

        timeline.observe_labelled(sample(0, 24, 23), label("10:00:00"));
        assert_eq!(Some("10:00:00"), timeline.label(23));

        // 滚屏一行：缓冲区底部前移，只有新行拿到新时间戳。
        timeline.observe_labelled(sample(5, 24, 23), label("10:05:00"));

        assert_eq!(Some("10:00:00"), timeline.label(23));
        assert_eq!(Some("10:05:00"), timeline.label(28));
        assert_eq!(None, timeline.label(29));
    }

    #[test]
    fn resizing_does_not_restamp_reflowed_lines() {
        let mut timeline = LineTimeline::default();

        // 首屏：光标走到第 10 行，0..=10 均已落时间戳。
        timeline.observe_labelled(sample(0, 24, 10), label("10:00:00"));
        assert_eq!(Some("10:00:00"), timeline.label(5));
        assert_eq!(Some("10:00:00"), timeline.label(10));

        // 窗口行数变化（reflow）：只把游标对齐，不给重排出的行补时间戳。
        timeline.observe_labelled(sample(2, 23, 10), label("10:00:05"));
        assert_eq!(Some("10:00:00"), timeline.label(5));
        assert_eq!(None, timeline.label(11));
        assert_eq!(None, timeline.label(12));

        // reflow 之后新到达的行仍然准确。
        timeline.observe_labelled(sample(2, 23, 14), label("10:00:06"));
        assert_eq!(Some("10:00:06"), timeline.label(14));
        assert_eq!(None, timeline.label(17));
    }

    #[test]
    fn clearing_history_restarts_numbering() {
        let mut timeline = LineTimeline::default();

        timeline.observe_labelled(sample(500, 24, 23), label("10:00:00"));
        assert_eq!(Some("10:00:00"), timeline.label(523));

        timeline.observe_labelled(sample(0, 24, 2), label("10:01:00"));

        assert_eq!(None, timeline.label(523));
        assert_eq!(Some("10:01:00"), timeline.label(2));
    }

    #[test]
    fn alternate_screen_is_ignored() {
        let mut timeline = LineTimeline::default();

        timeline.observe_labelled(sample(0, 24, 4), label("10:00:00"));
        let mut alt = sample(0, 24, 4);
        alt.alternate_screen = true;
        timeline.observe_labelled(alt, label("10:00:10"));

        assert_eq!(Some("10:00:00"), timeline.label(4));
        assert_eq!(None, timeline.label(9));
    }

    #[test]
    fn old_stamps_are_trimmed_outside_scrollback_window() {
        let mut timeline = LineTimeline::default();

        timeline.observe_labelled(sample(0, 24, 0), label("10:00:00"));
        timeline.observe_labelled(sample(5000, 24, 23), label("10:10:00"));

        assert_eq!(None, timeline.label(0));
        assert_eq!(Some("10:10:00"), timeline.label(5023));
    }

    #[test]
    fn shared_timeline_exposes_labels() {
        let shared = SharedLineTimeline::default();
        shared.observe(sample(0, 24, 1));

        let label = shared.label(1).expect("sampled line should have a label");
        assert_eq!(8, label.len());
        assert_eq!(Some(':'), label.chars().nth(2));
        assert_eq!(None, shared.label(9));
    }
}
