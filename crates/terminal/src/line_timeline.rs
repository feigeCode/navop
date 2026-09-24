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

    /// 整帧渲染用：一次取出从 `first_id` 开始 `count` 行的标签，只加锁一次。
    ///
    /// 返回长度恒为 `count`，第 `i` 项对应行 id `first_id + i`；还没有任何输出的
    /// 行（以及已滚出保留窗口的行）为 `None`。左边距据此判定该行是否留空。
    pub fn labels(&self, first_id: i64, count: usize) -> Vec<Option<Arc<str>>> {
        match self.0.lock() {
            Ok(timeline) => timeline.labels(first_id, count),
            Err(_) => vec![None; count],
        }
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

    /// 连续行 id 的批量查询：先二分定位起点，再顺序推进，避免逐行查找。
    fn labels(&self, first_id: i64, count: usize) -> Vec<Option<Arc<str>>> {
        let mut labels = Vec::with_capacity(count);
        let mut index = self
            .stamps
            .binary_search_by_key(&first_id, |(stamp_id, _)| *stamp_id)
            .unwrap_or_else(|insert| insert);
        for offset in 0..count as i64 {
            match self.stamps.get(index) {
                Some((stamp_id, label)) if *stamp_id == first_id + offset => {
                    labels.push(Some(label.clone()));
                    index += 1;
                }
                _ => labels.push(None),
            }
        }
        labels
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

    /// 单行断言的包装：批量查询是唯一查找路径。
    fn label_at(timeline: &LineTimeline, id: i64) -> Option<String> {
        let labels = timeline.labels(id, 1);
        labels.into_iter().next().flatten().map(|l| l.to_string())
    }

    #[test]
    fn stamps_cursor_advance_without_renumbering_earlier_lines() {
        let mut timeline = LineTimeline::default();

        timeline.observe_labelled(sample(0, 24, 0), label("10:00:00"));
        timeline.observe_labelled(sample(0, 24, 3), label("10:00:01"));

        assert_eq!(Some("10:00:00"), label_at(&timeline, 0).as_deref());
        assert_eq!(Some("10:00:01"), label_at(&timeline, 3).as_deref());
        assert_eq!(None, label_at(&timeline, 5).as_deref());
    }

    #[test]
    fn scrolling_stamps_new_bottom_lines_only() {
        let mut timeline = LineTimeline::default();

        timeline.observe_labelled(sample(0, 24, 23), label("10:00:00"));
        assert_eq!(Some("10:00:00"), label_at(&timeline, 23).as_deref());

        // 滚屏一行：缓冲区底部前移，只有新行拿到新时间戳。
        timeline.observe_labelled(sample(5, 24, 23), label("10:05:00"));

        assert_eq!(Some("10:00:00"), label_at(&timeline, 23).as_deref());
        assert_eq!(Some("10:05:00"), label_at(&timeline, 28).as_deref());
        assert_eq!(None, label_at(&timeline, 29).as_deref());
    }

    #[test]
    fn resizing_does_not_restamp_reflowed_lines() {
        let mut timeline = LineTimeline::default();

        // 首屏：光标走到第 10 行，0..=10 均已落时间戳。
        timeline.observe_labelled(sample(0, 24, 10), label("10:00:00"));
        assert_eq!(Some("10:00:00"), label_at(&timeline, 5).as_deref());
        assert_eq!(Some("10:00:00"), label_at(&timeline, 10).as_deref());

        // 窗口行数变化（reflow）：只把游标对齐，不给重排出的行补时间戳。
        timeline.observe_labelled(sample(2, 23, 10), label("10:00:05"));
        assert_eq!(Some("10:00:00"), label_at(&timeline, 5).as_deref());
        assert_eq!(None, label_at(&timeline, 11).as_deref());
        assert_eq!(None, label_at(&timeline, 12).as_deref());

        // reflow 之后新到达的行仍然准确。
        timeline.observe_labelled(sample(2, 23, 14), label("10:00:06"));
        assert_eq!(Some("10:00:06"), label_at(&timeline, 14).as_deref());
        assert_eq!(None, label_at(&timeline, 17).as_deref());
    }

    #[test]
    fn clearing_history_restarts_numbering() {
        let mut timeline = LineTimeline::default();

        timeline.observe_labelled(sample(500, 24, 23), label("10:00:00"));
        assert_eq!(Some("10:00:00"), label_at(&timeline, 523).as_deref());

        timeline.observe_labelled(sample(0, 24, 2), label("10:01:00"));

        assert_eq!(None, label_at(&timeline, 523).as_deref());
        assert_eq!(Some("10:01:00"), label_at(&timeline, 2).as_deref());
    }

    #[test]
    fn alternate_screen_is_ignored() {
        let mut timeline = LineTimeline::default();

        timeline.observe_labelled(sample(0, 24, 4), label("10:00:00"));
        let mut alt = sample(0, 24, 4);
        alt.alternate_screen = true;
        timeline.observe_labelled(alt, label("10:00:10"));

        assert_eq!(Some("10:00:00"), label_at(&timeline, 4).as_deref());
        assert_eq!(None, label_at(&timeline, 9).as_deref());
    }

    #[test]
    fn old_stamps_are_trimmed_outside_scrollback_window() {
        let mut timeline = LineTimeline::default();

        timeline.observe_labelled(sample(0, 24, 0), label("10:00:00"));
        timeline.observe_labelled(sample(5000, 24, 23), label("10:10:00"));

        assert_eq!(None, label_at(&timeline, 0).as_deref());
        assert_eq!(Some("10:10:00"), label_at(&timeline, 5023).as_deref());
    }

    #[test]
    fn batch_labels_align_by_line_id_across_a_screen_span() {
        let mut timeline = LineTimeline::default();
        // 光标停在第 2 行：0..=2 已落时间戳，其余屏幕行还没有输出。
        timeline.observe_labelled(sample(0, 24, 2), label("10:00:00"));

        let labels = timeline.labels(0, 5);
        let labels: Vec<Option<&str>> = labels.iter().map(|label| label.as_deref()).collect();
        assert_eq!(
            vec![
                Some("10:00:00"),
                Some("10:00:00"),
                Some("10:00:00"),
                None,
                None
            ],
            labels
        );

        // 窗口起点不在 0 时仍按绝对行 id 对齐（左侧行已滚出保留窗口 / 越界同样为 None）。
        let labels = timeline.labels(2, 4);
        let labels: Vec<Option<&str>> = labels.iter().map(|label| label.as_deref()).collect();
        assert_eq!(vec![Some("10:00:00"), None, None, None], labels);
    }

    #[test]
    fn shared_timeline_exposes_labels() {
        let shared = SharedLineTimeline::default();
        shared.observe(sample(0, 24, 1));

        let labels = shared.labels(0, 3);
        assert_eq!(3, labels.len(), "返回值长度与请求的屏幕行数一致");

        let label = labels[0]
            .as_deref()
            .expect("sampled line should have a label");
        assert_eq!(8, label.len());
        assert_eq!(Some(':'), label.chars().nth(2));
        assert!(labels[1].is_some(), "第 1 行已有输出");
        assert!(labels[2].is_none(), "光标下方还没有输出的行为 None");
    }
}
