use std::time::{Duration, Instant};

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(400);
const LOGIN_COMPENSATION_DELAY: Duration = Duration::from_millis(300);
const RETRY_DELAY: Duration = Duration::from_millis(500);
/// Retries allowed for one viewport before the session keeps its current
/// geometry.
///
/// `UpdateSessionDisplaySettings` is optional for the host and can be rejected
/// for the whole session. Retrying it forever re-issues the desktop size and
/// scale at the maintenance poll rate, which makes the remote session re-render
/// (pointer included) over and over, so give up until the viewport, login phase
/// or session generation actually changes.
const MAX_DISPLAY_RETRIES: u32 = 4;
const RETRY_BACKOFF_CAP: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WindowsNativeViewportSettings {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) desktop_scale_factor: u32,
}

/// Whether the native session should be created with client-side smart sizing.
///
/// Dynamic display sessions are resized to the viewport, so there is never a
/// desktop/control mismatch to fit. Leaving smart sizing on only lets mstscax
/// stretch the session bitmap — pointer included — whenever the two sizes
/// disagree (for example while a display update is pending or was rejected),
/// which is the client-side rescaling of the remote cursor. Fixed-size sessions
/// keep the configured value, where fitting is the point.
pub(super) fn request_smart_sizing(dynamic_display: bool, configured: bool) -> bool {
    configured && !dynamic_display
}

/// Display scale the native session is created with.
///
/// Dynamic display sessions are re-scoped to the local display scale by
/// [`WindowsNativeDisplayState`] as soon as login completes, so creating them
/// with a different (configured) value only buys one extra remote-side rescale
/// right after login — mstscax re-renders the whole session for it, the pointer
/// included. Fixed-size sessions are never re-scoped, so they keep the
/// configured value.
pub(super) fn connect_desktop_scale_factor(
    dynamic_display: bool,
    configured_scale_factor: u32,
    display_scale_factor: f32,
) -> u32 {
    if dynamic_display {
        super::resize::scale_factor_percent(display_scale_factor)
    } else {
        configured_scale_factor
    }
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
    failed_attempts: u32,
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
        // A different viewport is a fresh attempt, even for a host that rejected
        // every previous display update.
        self.failed_attempts = 0;
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
        self.failed_attempts = 0;
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
        self.failed_attempts = 0;
        if self.latest == Some(request.settings) {
            self.pending_since = None;
        }
    }

    /// Records a rejected display update.
    ///
    /// Returns whether the retries for this viewport are now exhausted; the
    /// caller is expected to surface that once so a host that cannot apply
    /// dynamic display updates is not hammered at the poll rate.
    pub(super) fn request_failed(
        &mut self,
        request: WindowsNativeDisplayRequest,
        now: Instant,
    ) -> bool {
        if self.generation != Some(request.generation) || !self.ready {
            return false;
        }
        self.failed_attempts = self.failed_attempts.saturating_add(1);
        if self.failed_attempts <= MAX_DISPLAY_RETRIES {
            self.retry_after = Some(now + retry_backoff(self.failed_attempts));
            return false;
        }
        self.retry_after = None;
        // Stop offering the rejected geometry: `take_resize_request` would
        // otherwise re-request it as soon as the debounce deadline is due.
        self.pending_since = None;
        self.force_pending = None;
        self.compensation_deadline = None;
        true
    }

    pub(super) fn suspend(&mut self) {
        self.ready = false;
        self.pending_since = None;
        self.last_sent = None;
        self.force_pending = None;
        self.compensation_deadline = None;
        self.retry_after = None;
        self.failed_attempts = 0;
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
        self.failed_attempts = 0;
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

/// Doubling backoff for repeated display-update failures, capped so a session
/// that eventually recovers is retried soon after.
fn retry_backoff(failed_attempts: u32) -> Duration {
    let exponent = failed_attempts.saturating_sub(1).min(8);
    RETRY_DELAY
        .saturating_mul(1u32 << exponent)
        .min(RETRY_BACKOFF_CAP)
}

#[cfg(test)]
#[path = "windows_native_display_tests.rs"]
mod tests;
