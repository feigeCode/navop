use std::sync::Arc;

use gpui::App;

#[derive(Clone)]
pub struct RemoteMutationCallback(Arc<dyn Fn(&mut App) + Send + Sync>);

impl RemoteMutationCallback {
    pub fn new(callback: impl Fn(&mut App) + Send + Sync + 'static) -> Self {
        Self(Arc::new(callback))
    }

    pub fn notify(&self, cx: &mut App) {
        (self.0)(cx);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use gpui::TestAppContext;

    use super::RemoteMutationCallback;

    #[gpui::test]
    fn callback_invokes_registered_refresh(cx: &mut TestAppContext) {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let callback = RemoteMutationCallback::new(move |_| {
            observed.fetch_add(1, Ordering::Relaxed);
        });

        cx.update(|cx| {
            callback.notify(cx);
            callback.notify(cx);
        });

        assert_eq!(2, calls.load(Ordering::Relaxed));
    }
}
