use std::time::{Duration, Instant};

use super::{
    WindowsNativeDisplayFlushReason as Reason, WindowsNativeDisplayState,
    WindowsNativeViewportSettings as Settings,
};

const GENERATION: u64 = 7;
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const SCALE: u32 = 150;

fn settings() -> Settings {
    Settings {
        width: WIDTH,
        height: HEIGHT,
        desktop_scale_factor: SCALE,
    }
}

fn attached(started_at: Instant) -> WindowsNativeDisplayState {
    let mut state = WindowsNativeDisplayState::default();
    state.attach(GENERATION);
    state.observe(settings(), started_at);
    state
}

fn ready(now: Instant) -> WindowsNativeDisplayState {
    let mut state = attached(now);
    state.login_complete(GENERATION, now);
    let request = state.take_request(now).expect("immediate request");
    state.request_succeeded(request);
    state
}

fn failed(now: Instant) -> WindowsNativeDisplayState {
    let mut state = attached(now);
    state.login_complete(GENERATION, now);
    let request = state.take_request(now).expect("immediate request");
    state.request_failed(request, now);
    state
}

#[test]
fn resize_before_login_only_caches_viewport() {
    let now = Instant::now();
    let mut state = attached(now);

    assert_eq!(None, state.take_request(now + Duration::from_secs(1)));
}

#[test]
fn login_complete_forces_immediate_request() {
    let now = Instant::now();
    let mut state = attached(now);
    state.login_complete(GENERATION, now);

    let request = state.take_request(now).expect("immediate request");
    assert_eq!(Reason::LoginComplete, request.reason);
    assert_eq!(settings(), request.settings);
}

#[test]
fn successful_login_request_keeps_compensation_armed() {
    let now = Instant::now();
    let mut state = ready(now);

    assert_eq!(None, state.take_request(now + Duration::from_millis(299)));
    assert_eq!(
        Reason::LoginCompensation,
        state
            .take_request(now + Duration::from_millis(300))
            .expect("compensation")
            .reason
    );
}

#[test]
fn resize_waits_for_full_debounce() {
    let now = Instant::now();
    let mut state = ready(now);
    let _ = state.take_request(now + Duration::from_millis(300));
    state.observe(
        Settings {
            width: 1600,
            ..settings()
        },
        now + Duration::from_millis(500),
    );

    assert_eq!(None, state.take_request(now + Duration::from_millis(899)));
    assert_eq!(
        Reason::Resize,
        state
            .take_request(now + Duration::from_millis(900))
            .expect("debounced resize")
            .reason
    );
}

#[test]
fn identical_resize_does_not_restart_debounce() {
    let now = Instant::now();
    let mut state = ready(now);
    let _ = state.take_request(now + Duration::from_millis(300));
    let changed = Settings {
        width: 1600,
        ..settings()
    };
    state.observe(changed, now + Duration::from_millis(500));
    state.observe(changed, now + Duration::from_millis(700));

    assert_eq!(
        changed,
        state
            .take_request(now + Duration::from_millis(900))
            .expect("original debounce deadline")
            .settings
    );
}

#[test]
fn successful_normal_send_deduplicates_same_settings() {
    let now = Instant::now();
    let mut state = ready(now);
    let request = state
        .take_request(now + Duration::from_millis(300))
        .expect("compensation");
    state.request_succeeded(request);
    state.observe(settings(), now + Duration::from_secs(1));

    assert_eq!(None, state.take_request(now + Duration::from_secs(2)));
}

#[test]
fn failure_retries_after_five_hundred_milliseconds() {
    let now = Instant::now();
    let mut state = failed(now);

    assert_eq!(None, state.take_request(now + Duration::from_millis(499)));
    assert_eq!(
        Reason::Retry,
        state
            .take_request(now + Duration::from_millis(500))
            .expect("retry")
            .reason
    );
}

#[test]
fn retry_gate_blocks_due_compensation() {
    let now = Instant::now();
    let mut state = failed(now);

    assert_eq!(None, state.take_request(now + Duration::from_millis(300)));
}

#[test]
fn compensation_uses_latest_viewport() {
    let now = Instant::now();
    let mut state = ready(now);
    let latest = Settings {
        height: 900,
        ..settings()
    };
    state.observe(latest, now + Duration::from_millis(100));

    assert_eq!(
        latest,
        state
            .take_request(now + Duration::from_millis(300))
            .expect("compensation")
            .settings
    );
}

#[test]
fn attaching_new_generation_clears_old_work() {
    let now = Instant::now();
    let mut state = attached(now);
    state.login_complete(GENERATION, now);
    state.attach(GENERATION + 1);

    assert_eq!(None, state.take_request(now + Duration::from_secs(1)));
}

#[test]
fn reset_clears_all_pending_work() {
    let now = Instant::now();
    let mut state = attached(now);
    state.login_complete(GENERATION, now);
    state.reset();

    assert_eq!(None, state.take_request(now + Duration::from_secs(1)));
}

#[test]
fn suspend_preserves_session_for_reconnect() {
    let now = Instant::now();
    let mut state = ready(now);
    let latest = Settings {
        width: 1600,
        ..settings()
    };
    state.suspend();
    state.observe(latest, now + Duration::from_millis(100));
    state.reconnected(GENERATION, now + Duration::from_millis(200));

    let request = state
        .take_request(now + Duration::from_millis(200))
        .expect("reconnected request");
    assert_eq!(GENERATION, request.generation);
    assert_eq!(latest, request.settings);
}

#[test]
fn stale_generation_event_is_ignored() {
    let now = Instant::now();
    let mut state = attached(now);
    state.login_complete(GENERATION + 1, now);

    assert_eq!(None, state.take_request(now));
}

#[test]
fn reconnecting_suspends_display_updates() {
    let now = Instant::now();
    let mut state = attached(now);
    state.login_complete(GENERATION, now);
    state.reconnecting(GENERATION);
    state.observe(
        Settings {
            width: 1600,
            ..settings()
        },
        now,
    );

    assert_eq!(None, state.take_request(now + Duration::from_secs(1)));
}

#[test]
fn reconnected_forces_current_viewport() {
    let now = Instant::now();
    let mut state = attached(now);
    state.login_complete(GENERATION, now);
    state.reconnecting(GENERATION);
    let latest = Settings {
        width: 1600,
        ..settings()
    };
    state.observe(latest, now);
    state.reconnected(GENERATION, now);

    let request = state.take_request(now).expect("reconnected request");
    assert_eq!(Reason::Reconnected, request.reason);
    assert_eq!(latest, request.settings);
}

#[test]
fn duplicate_login_complete_does_not_extend_compensation() {
    let now = Instant::now();
    let mut state = ready(now);
    state.login_complete(GENERATION, now + Duration::from_millis(200));

    assert_eq!(
        Reason::LoginCompensation,
        state
            .take_request(now + Duration::from_millis(300))
            .expect("original compensation")
            .reason
    );
}

#[test]
fn retry_success_consumes_overdue_compensation() {
    let now = Instant::now();
    let mut state = failed(now);
    let retry = state
        .take_request(now + Duration::from_millis(500))
        .expect("retry");
    state.request_succeeded(retry);

    assert_eq!(None, state.take_request(now + Duration::from_millis(501)));
}
