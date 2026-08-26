use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub(super) struct CancellationWatcherGuard(Arc<AtomicUsize>);

impl CancellationWatcherGuard {
    pub(super) fn new(active_watchers: Arc<AtomicUsize>) -> Self {
        active_watchers.fetch_add(1, Ordering::Relaxed);
        Self(active_watchers)
    }
}

impl Drop for CancellationWatcherGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
