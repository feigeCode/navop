use std::sync::atomic::{AtomicU64, Ordering};

/// 进程内 X11 组件计数快照，供 dogfood 日志和 UI 只读查询。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct X11ComponentMetrics {
    /// 内置服务首次启动成功次数，不含崩溃重启。
    pub embedded_starts: u64,
    /// 监测到已运行的内置服务退出或状态读取失败次数。
    pub crashes: u64,
    /// 崩溃后成功恢复握手次数。
    pub restarts: u64,
    /// Auto 外部探测不可用而转入内置路径的次数。
    pub auto_external_fallbacks: u64,
    /// Embedded 或 Auto 的内置路径最终返回错误次数。
    pub embedded_failures: u64,
}

#[derive(Clone, Copy)]
pub(crate) enum MetricEvent {
    EmbeddedStart,
    Crash,
    Restart,
    AutoExternalFallback,
    EmbeddedFailure,
}

impl MetricEvent {
    fn name(self) -> &'static str {
        match self {
            Self::EmbeddedStart => "embedded_start",
            Self::Crash => "crash",
            Self::Restart => "restart",
            Self::AutoExternalFallback => "auto_external_fallback",
            Self::EmbeddedFailure => "embedded_failure",
        }
    }
}

static EMBEDDED_STARTS: AtomicU64 = AtomicU64::new(0);
static CRASHES: AtomicU64 = AtomicU64::new(0);
static RESTARTS: AtomicU64 = AtomicU64::new(0);
static AUTO_EXTERNAL_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static EMBEDDED_FAILURES: AtomicU64 = AtomicU64::new(0);

/// 返回当前进程的单调计数；不发送远端遥测。
pub fn metrics() -> X11ComponentMetrics {
    X11ComponentMetrics {
        embedded_starts: EMBEDDED_STARTS.load(Ordering::Relaxed),
        crashes: CRASHES.load(Ordering::Relaxed),
        restarts: RESTARTS.load(Ordering::Relaxed),
        auto_external_fallbacks: AUTO_EXTERNAL_FALLBACKS.load(Ordering::Relaxed),
        embedded_failures: EMBEDDED_FAILURES.load(Ordering::Relaxed),
    }
}

pub(crate) fn record(event: MetricEvent) {
    let counter = match event {
        MetricEvent::EmbeddedStart => &EMBEDDED_STARTS,
        MetricEvent::Crash => &CRASHES,
        MetricEvent::Restart => &RESTARTS,
        MetricEvent::AutoExternalFallback => &AUTO_EXTERNAL_FALLBACKS,
        MetricEvent::EmbeddedFailure => &EMBEDDED_FAILURES,
    };
    counter.fetch_add(1, Ordering::Relaxed);
    let snapshot = metrics();
    tracing::info!(
        target: "x11_component.metrics",
        event = event.name(),
        embedded_starts = snapshot.embedded_starts,
        crashes = snapshot.crashes,
        restarts = snapshot.restarts,
        auto_external_fallbacks = snapshot.auto_external_fallbacks,
        embedded_failures = snapshot.embedded_failures,
        "X11 组件状态转换"
    );
}

#[cfg(test)]
mod tests;
