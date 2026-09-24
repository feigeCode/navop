//! 复用弹窗的生命周期计数（诊断用）。
//!
//! # 为什么需要这些计数
//!
//! macOS 上销毁原生窗口会让 AppKit 的 Touch Bar 观察者去注销一个已经 dealloc 的对象，
//! 抛出的 ObjC 异常无人接住 ⇒ 闪退（见 [`crate::window_close::hide_for_reuse`]）。
//! 因此弹窗的「关闭」被拆成两件事：
//!
//! * **原生窗口**：隐藏并登记复用，进程存续期间不再销毁；
//! * **业务会话**：关闭时立即结束 —— 释放业务 view 及它持有的数据、连接与任务句柄。
//!
//! 只有把这两件事分开计数，才能从一次内存采样里分清「受控的固定保留」和「持续增长」：
//!
//! | 现象 | 说明 |
//! |---|---|
//! | 每类弹窗首次打开时 `live_windows` +1，之后开关不再增长 | 正常的窗口复用 |
//! | 关闭后 `live_sessions` 回落到 0 | 业务会话确实被释放 |
//! | `live_sessions` 随开关次数持续上涨 | 旧会话没卸载（本模块存在的意义） |
//! | `opened_windows` 随开关次数持续上涨 | 复用没命中，退化成「每次新建窗口」 |
//!
//! 「复用键 → 一个窗口」的一对一关系决定了 `live_windows` 的上限是**复用键数量**，
//! 而不是开关次数。这也是为什么不给复用注册表加 LRU 淘汰：淘汰即销毁，等于把崩溃
//! 挪到了淘汰路径上。
//!
//! 计数只记数字，不记标题、路径或任何业务内容。

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// 弹窗生命周期的点态快照。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PopupLifecycleSnapshot {
    /// 登记在册（隐藏后等待复用）的原生窗口数。
    pub live_windows: i64,
    /// 当前仍持有业务 view 的弹窗数 —— 关闭后必须回落。
    pub live_sessions: i64,
    /// 累计创建的原生窗口数。
    pub opened_windows: u64,
    /// 累计打开的业务会话数。
    pub opened_sessions: u64,
}

static LIVE_WINDOWS: AtomicI64 = AtomicI64::new(0);
static LIVE_SESSIONS: AtomicI64 = AtomicI64::new(0);
static OPENED_WINDOWS: AtomicU64 = AtomicU64::new(0);
static OPENED_SESSIONS: AtomicU64 = AtomicU64::new(0);

/// 读取当前计数。用于诊断面板与测试断言。
pub fn snapshot() -> PopupLifecycleSnapshot {
    PopupLifecycleSnapshot {
        live_windows: LIVE_WINDOWS.load(Ordering::Relaxed),
        live_sessions: LIVE_SESSIONS.load(Ordering::Relaxed),
        opened_windows: OPENED_WINDOWS.load(Ordering::Relaxed),
        opened_sessions: OPENED_SESSIONS.load(Ordering::Relaxed),
    }
}

/// 打一条低频生命周期日志。弹窗开关本身就很稀疏，不需要节流。
pub fn log_lifecycle(stage: &'static str) {
    let snapshot = snapshot();
    tracing::info!(
        target: "one_core::popup_lifecycle",
        stage,
        live_windows = snapshot.live_windows,
        live_sessions = snapshot.live_sessions,
        opened_windows = snapshot.opened_windows,
        opened_sessions = snapshot.opened_sessions,
        "popup window lifecycle counters"
    );
}

/// 一个原生窗口被登记进复用注册表。
pub(crate) fn record_window_registered() {
    LIVE_WINDOWS.fetch_add(1, Ordering::Relaxed);
    OPENED_WINDOWS.fetch_add(1, Ordering::Relaxed);
}

/// 一个原生窗口离开复用注册表（真正销毁，或条目已失效被清理）。
pub(crate) fn record_window_unregistered() {
    LIVE_WINDOWS.fetch_sub(1, Ordering::Relaxed);
}

/// 一次业务会话开始（本次打开创建了业务 view）。
pub(crate) fn record_session_opened() {
    LIVE_SESSIONS.fetch_add(1, Ordering::Relaxed);
    OPENED_SESSIONS.fetch_add(1, Ordering::Relaxed);
}

/// 一次业务会话结束（view 被卸载，或窗口连同 view 一起销毁）。
pub(crate) fn record_session_ended() {
    LIVE_SESSIONS.fetch_sub(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 计数是进程级的，测试必须串行观察自己的增量。
    static GAUGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn registering_and_unregistering_windows_moves_only_the_window_gauges() {
        let _lock = GAUGE_LOCK.lock().unwrap();
        let baseline = snapshot();

        record_window_registered();
        let registered = snapshot();
        assert_eq!(baseline.live_windows + 1, registered.live_windows);
        assert_eq!(baseline.opened_windows + 1, registered.opened_windows);
        assert_eq!(baseline.live_sessions, registered.live_sessions);
        assert_eq!(baseline.opened_sessions, registered.opened_sessions);

        record_window_unregistered();
        assert_eq!(baseline.live_windows, snapshot().live_windows);
    }

    #[test]
    fn opening_and_ending_a_session_moves_only_the_session_gauges() {
        let _lock = GAUGE_LOCK.lock().unwrap();
        let baseline = snapshot();

        record_session_opened();
        let opened = snapshot();
        assert_eq!(baseline.live_sessions + 1, opened.live_sessions);
        assert_eq!(baseline.opened_sessions + 1, opened.opened_sessions);
        assert_eq!(baseline.live_windows, opened.live_windows);

        record_session_ended();
        assert_eq!(baseline.live_sessions, snapshot().live_sessions);
    }

    /// 复用窗口重新打开只增加「会话」累计数，不增加「窗口」累计数 ——
    /// 这正是复用生效（而不是每次新建窗口）的判据。
    #[test]
    fn reusing_a_window_opens_a_second_session_without_a_second_window() {
        let _lock = GAUGE_LOCK.lock().unwrap();
        let baseline = snapshot();

        record_window_registered();
        record_session_opened();
        record_session_ended();
        record_session_opened();

        let after_reuse = snapshot();
        assert_eq!(baseline.live_windows + 1, after_reuse.live_windows);
        assert_eq!(baseline.opened_windows + 1, after_reuse.opened_windows);
        assert_eq!(baseline.live_sessions + 1, after_reuse.live_sessions);
        assert_eq!(baseline.opened_sessions + 2, after_reuse.opened_sessions);

        record_session_ended();
        record_window_unregistered();
        // `opened_*` 是累计量，不会回落：这里只要求「存量」回到基线。
        assert_eq!(baseline.live_windows, snapshot().live_windows);
        assert_eq!(baseline.live_sessions, snapshot().live_sessions);
    }
}
