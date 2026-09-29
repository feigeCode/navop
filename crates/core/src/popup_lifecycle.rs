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
//! | 复用键弹窗首次打开时 `live_windows` +1，之后开关不再增长 | 正常的窗口复用 |
//! | 一次性弹窗每打开一次 `live_windows` +1（只增不减） | 符合预期的**停放**：没有复用键，但同样不销毁 |
//! | 关闭后 `live_sessions` 回落到 0 | 业务会话确实被释放 |
//! | `live_sessions` 随开关次数持续上涨 | 旧会话没卸载（本模块存在的意义） |
//! | 复用键弹窗 `opened_windows` 随开关次数持续上涨 | 复用没命中，退化成「每次新建窗口」 |
//!
//! `live_windows` 因此有两个来源：一是「复用键 → 一个窗口」的一对一关系（上限是**复用键
//! 数量**），二是**一次性弹窗的停放**（上限是打开次数，见
//! `crate::popup_window::PARKED_POPUPS`）。两者都不销毁 —— 「销毁即崩」是这两条路的共同
//! 前提（#308 / #314），也是为什么不给注册表加 LRU 淘汰：淘汰即销毁，等于把崩溃挪到了
//! 淘汰路径上。想把第二项压下去，得把热点弹窗改成 `open_reusable_popup_window`。
//!
//! 两个来源的**上限来源不同**，所以分开计量（`reusable_windows` / `parked_windows`）：
//! 混在一个总数里，一个正常的复用窗口会被误读成泄漏，而真正的泄漏也会被「反正是停放」
//! 遮掉。判断内存增长是否受控时看这两项各自的走势，而不是只看 `live_windows`。
//!
//! 计数只记数字，不记标题、路径或任何业务内容。

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// 弹窗登记表的种类。
///
/// 两者都受控、都不销毁，但**上限来源不同**，判断「增长是否正常」时必须分开看：
/// 复用键窗口的上限是**复用键数量**（同一目标反复开关不增长），停放窗口的上限是
/// **这类弹窗的打开次数**（只增不减）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PopupWindowKind {
    /// 有复用键：关闭后隐藏，同一个键再次打开时重新显示。
    Reusable,
    /// 一次性弹窗：关闭后隐藏停放，下一次打开是新建窗口。
    Parked,
}

/// 弹窗生命周期的点态快照。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PopupLifecycleSnapshot {
    /// 登记在册的弹窗窗口数（复用键弹窗 + 一次性停放弹窗）。
    pub live_windows: i64,
    /// 其中按复用键复用那一部分。
    pub reusable_windows: i64,
    /// 其中一次性停放那一部分。
    pub parked_windows: i64,
    /// 当前仍持有业务 view 的弹窗数 —— 关闭后必须回落。
    pub live_sessions: i64,
    /// 累计创建的原生窗口数。
    pub opened_windows: u64,
    /// 累计打开的业务会话数。
    pub opened_sessions: u64,
}

static LIVE_WINDOWS: AtomicI64 = AtomicI64::new(0);
static LIVE_REUSABLE_WINDOWS: AtomicI64 = AtomicI64::new(0);
static LIVE_PARKED_WINDOWS: AtomicI64 = AtomicI64::new(0);
static LIVE_SESSIONS: AtomicI64 = AtomicI64::new(0);
static OPENED_WINDOWS: AtomicU64 = AtomicU64::new(0);
static OPENED_SESSIONS: AtomicU64 = AtomicU64::new(0);

/// 读取当前计数。用于诊断面板与测试断言。
pub fn snapshot() -> PopupLifecycleSnapshot {
    PopupLifecycleSnapshot {
        live_windows: LIVE_WINDOWS.load(Ordering::Relaxed),
        reusable_windows: LIVE_REUSABLE_WINDOWS.load(Ordering::Relaxed),
        parked_windows: LIVE_PARKED_WINDOWS.load(Ordering::Relaxed),
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
        reusable_windows = snapshot.reusable_windows,
        parked_windows = snapshot.parked_windows,
        live_sessions = snapshot.live_sessions,
        opened_windows = snapshot.opened_windows,
        opened_sessions = snapshot.opened_sessions,
        "popup window lifecycle counters"
    );
}

/// 一个原生窗口被登记进复用注册表（`kind` 决定它记在哪一类上）。
pub(crate) fn record_window_registered(kind: PopupWindowKind) {
    LIVE_WINDOWS.fetch_add(1, Ordering::Relaxed);
    kind_gauge(kind).fetch_add(1, Ordering::Relaxed);
    OPENED_WINDOWS.fetch_add(1, Ordering::Relaxed);
}

/// 一个原生窗口离开复用注册表（真正销毁，或条目已失效被清理）。
pub(crate) fn record_window_unregistered(kind: PopupWindowKind) {
    LIVE_WINDOWS.fetch_sub(1, Ordering::Relaxed);
    kind_gauge(kind).fetch_sub(1, Ordering::Relaxed);
}

/// 这一类窗口的存量计价器。
fn kind_gauge(kind: PopupWindowKind) -> &'static AtomicI64 {
    match kind {
        PopupWindowKind::Reusable => &LIVE_REUSABLE_WINDOWS,
        PopupWindowKind::Parked => &LIVE_PARKED_WINDOWS,
    }
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

        record_window_registered(PopupWindowKind::Reusable);
        let registered = snapshot();
        assert_eq!(baseline.live_windows + 1, registered.live_windows);
        assert_eq!(baseline.opened_windows + 1, registered.opened_windows);
        assert_eq!(
            baseline.reusable_windows + 1,
            registered.reusable_windows
        );
        assert_eq!(baseline.parked_windows, registered.parked_windows);
        assert_eq!(baseline.live_sessions, registered.live_sessions);
        assert_eq!(baseline.opened_sessions, registered.opened_sessions);

        record_window_unregistered(PopupWindowKind::Reusable);
        assert_eq!(baseline.live_windows, snapshot().live_windows);
        assert_eq!(baseline.reusable_windows, snapshot().reusable_windows);
    }

    /// 复用键窗口与停放窗口必须分开计量：两者上限来源不同，混在 `live_windows` 里
    /// 一个正常的复用会被误读成泄漏，而真正的泄漏也会被「反正是停放」遮掉。
    #[test]
    fn reusable_and_parked_windows_are_counted_separately() {
        let _lock = GAUGE_LOCK.lock().unwrap();
        let baseline = snapshot();

        record_window_registered(PopupWindowKind::Reusable);
        record_window_registered(PopupWindowKind::Parked);
        record_window_registered(PopupWindowKind::Parked);

        let snapshot_after = snapshot();
        assert_eq!(baseline.live_windows + 3, snapshot_after.live_windows);
        assert_eq!(
            baseline.reusable_windows + 1,
            snapshot_after.reusable_windows
        );
        assert_eq!(baseline.parked_windows + 2, snapshot_after.parked_windows);
        // 总数始终等于两类之和，分开计量不能把总数算乱。
        assert_eq!(
            snapshot_after.live_windows,
            snapshot_after.reusable_windows + snapshot_after.parked_windows
        );

        for kind in [
            PopupWindowKind::Reusable,
            PopupWindowKind::Parked,
            PopupWindowKind::Parked,
        ] {
            record_window_unregistered(kind);
        }
        assert_eq!(baseline.live_windows, snapshot().live_windows);
        assert_eq!(baseline.reusable_windows, snapshot().reusable_windows);
        assert_eq!(baseline.parked_windows, snapshot().parked_windows);
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

        record_window_registered(PopupWindowKind::Reusable);
        record_session_opened();
        record_session_ended();
        record_session_opened();

        let after_reuse = snapshot();
        assert_eq!(baseline.live_windows + 1, after_reuse.live_windows);
        assert_eq!(baseline.opened_windows + 1, after_reuse.opened_windows);
        assert_eq!(baseline.live_sessions + 1, after_reuse.live_sessions);
        assert_eq!(baseline.opened_sessions + 2, after_reuse.opened_sessions);

        record_session_ended();
        record_window_unregistered(PopupWindowKind::Reusable);
        // `opened_*` 是累计量，不会回落：这里只要求「存量」回到基线。
        assert_eq!(baseline.live_windows, snapshot().live_windows);
        assert_eq!(baseline.live_sessions, snapshot().live_sessions);
    }
}
