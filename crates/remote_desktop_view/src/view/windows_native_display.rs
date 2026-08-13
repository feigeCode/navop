use std::time::{Duration, Instant};

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(400);
const LOGIN_COMPENSATION_DELAY: Duration = Duration::from_millis(300);
const RETRY_DELAY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WindowsNativeViewportSettings {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) desktop_scale_factor: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WindowsNativeDisplayFlushReason {
    LoginComplete,
    Reconnected,
    LoginCompensation,
    Resize,
    Retry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WindowsNativeDisplayRequest {
    pub(super) generation: u64,
    pub(super) settings: WindowsNativeViewportSettings,
    pub(super) reason: WindowsNativeDisplayFlushReason,
}

#[derive(Default)]
pub(super) struct WindowsNativeDisplayState {
    generation: Option<u64>,
    ready: bool,
    latest: Option<WindowsNativeViewportSettings>,
    pending_since: Option<Instant>,
    last_sent: Option<WindowsNativeViewportSettings>,
    force_pending: Option<WindowsNativeDisplayFlushReason>,
    compensation_deadline: Option<Instant>,
    retry_after: Option<Instant>,
}

impl WindowsNativeDisplayState {
    pub(super) fn attach(&mut self, generation: u64) {
        self.reset();
        self.generation = Some(generation);
    }

    pub(super) fn observe(&mut self, settings: WindowsNativeViewportSettings, now: Instant) {
        if self.latest == Some(settings) {
            return;
        }
        self.latest = Some(settings);
        if self.ready {
            self.pending_since = Some(now);
        }
    }

    pub(super) fn login_complete(&mut self, generation: u64, now: Instant) {
        self.mark_ready(
            generation,
            now,
            WindowsNativeDisplayFlushReason::LoginComplete,
        );
    }

    pub(super) fn reconnecting(&mut self, generation: u64) {
        if self.generation != Some(generation) {
            return;
        }
        self.ready = false;
        self.pending_since = None;
        self.last_sent = None;
        self.force_pending = None;
        self.compensation_deadline = None;
        self.retry_after = None;
    }

    pub(super) fn reconnected(&mut self, generation: u64, now: Instant) {
        self.mark_ready(
            generation,
            now,
            WindowsNativeDisplayFlushReason::Reconnected,
        );
    }

    pub(super) fn take_request(&mut self, now: Instant) -> Option<WindowsNativeDisplayRequest> {
        let generation = self.generation?;
        let settings = self.latest?;
        if !self.ready || self.retry_after.is_some_and(|deadline| now < deadline) {
            return None;
        }
        if self.retry_after.take().is_some() {
            self.consume_overdue_compensation(now);
            return Some(Self::request(
                generation,
                settings,
                WindowsNativeDisplayFlushReason::Retry,
            ));
        }
        if let Some(reason) = self.force_pending.take() {
            return Some(Self::request(generation, settings, reason));
        }
        if self
            .compensation_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.compensation_deadline = None;
            return Some(Self::request(
                generation,
                settings,
                WindowsNativeDisplayFlushReason::LoginCompensation,
            ));
        }
        self.take_resize_request(generation, settings, now)
    }

    pub(super) fn request_succeeded(&mut self, request: WindowsNativeDisplayRequest) {
        if self.generation != Some(request.generation) {
            return;
        }
        self.last_sent = Some(request.settings);
        self.retry_after = None;
        if self.latest == Some(request.settings) {
            self.pending_since = None;
        }
    }

    pub(super) fn request_failed(&mut self, request: WindowsNativeDisplayRequest, now: Instant) {
        if self.generation == Some(request.generation) && self.ready {
            self.retry_after = Some(now + RETRY_DELAY);
        }
    }

    pub(super) fn suspend(&mut self) {
        self.ready = false;
        self.pending_since = None;
        self.last_sent = None;
        self.force_pending = None;
        self.compensation_deadline = None;
        self.retry_after = None;
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    fn mark_ready(
        &mut self,
        generation: u64,
        now: Instant,
        reason: WindowsNativeDisplayFlushReason,
    ) {
        if self.generation != Some(generation) || self.ready {
            return;
        }
        self.ready = true;
        self.force_pending = Some(reason);
        self.compensation_deadline = Some(now + LOGIN_COMPENSATION_DELAY);
        self.retry_after = None;
    }

    fn take_resize_request(
        &self,
        generation: u64,
        settings: WindowsNativeViewportSettings,
        now: Instant,
    ) -> Option<WindowsNativeDisplayRequest> {
        let pending_since = self.pending_since?;
        if now.duration_since(pending_since) < RESIZE_DEBOUNCE || self.last_sent == Some(settings) {
            return None;
        }
        Some(Self::request(
            generation,
            settings,
            WindowsNativeDisplayFlushReason::Resize,
        ))
    }

    fn consume_overdue_compensation(&mut self, now: Instant) {
        if self
            .compensation_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.compensation_deadline = None;
        }
    }

    const fn request(
        generation: u64,
        settings: WindowsNativeViewportSettings,
        reason: WindowsNativeDisplayFlushReason,
    ) -> WindowsNativeDisplayRequest {
        WindowsNativeDisplayRequest {
            generation,
            settings,
            reason,
        }
    }
}

#[cfg(test)]
#[path = "windows_native_display_tests.rs"]
mod tests;
