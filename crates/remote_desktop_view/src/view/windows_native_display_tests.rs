use std::time::{Duration, Instant};

use super::{
    WindowsNativeDisplayFlushReason as Reason, WindowsNativeDisplayState,
    WindowsNativeViewportSettings as Settings, connect_desktop_scale_factor, request_smart_sizing,
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

#[test]
fn repeated_display_failures_back_off_and_then_give_up() {
    let now = Instant::now();
    let mut state = attached(now);
    state.login_complete(GENERATION, now);

    let mut request = state.take_request(now).expect("immediate request");
    let mut at = now;
    for (index, backoff_ms) in [500u64, 1_000, 2_000, 4_000].into_iter().enumerate() {
        assert!(
            !state.request_failed(request, at),
            "attempt {} must still retry",
            index + 1
        );
        assert_eq!(
            None,
            state.take_request(at + Duration::from_millis(backoff_ms - 1)),
            "a {backoff_ms}ms backoff must gate the retry"
        );
        at += Duration::from_millis(backoff_ms);
        request = state.take_request(at).expect("backed-off retry");
        assert_eq!(Reason::Retry, request.reason);
    }

    // The viewport was rejected five times: stop re-asserting the desktop size
    // and scale for it instead of hammering the session at the poll rate.
    assert!(state.request_failed(request, at));
    assert_eq!(None, state.take_request(at + Duration::from_secs(30)));
}

#[test]
fn a_new_viewport_rearms_display_updates_after_giving_up() {
    let now = Instant::now();
    let mut state = attached(now);
    state.login_complete(GENERATION, now);
    let request = state.take_request(now).expect("immediate request");
    let mut at = now;
    for _ in 0..5 {
        state.request_failed(request, at);
        at += Duration::from_millis(120);
    }
    assert_eq!(None, state.take_request(at + Duration::from_secs(10)));

    let resized = Settings {
        width: 1600,
        ..settings()
    };
    state.observe(resized, at + Duration::from_secs(10));

    assert_eq!(
        resized,
        state
            .take_request(at + Duration::from_millis(10_400))
            .expect("a changed viewport is a fresh attempt")
            .settings
    );
}

#[test]
fn reconnect_reapplies_the_viewport_after_giving_up() {
    let now = Instant::now();
    let mut state = attached(now);
    state.login_complete(GENERATION, now);
    let request = state.take_request(now).expect("immediate request");
    let mut at = now;
    for _ in 0..5 {
        state.request_failed(request, at);
        at += Duration::from_millis(120);
    }
    assert_eq!(None, state.take_request(at + Duration::from_secs(10)));

    state.reconnecting(GENERATION);
    state.reconnected(GENERATION, at + Duration::from_secs(11));

    assert_eq!(
        Reason::Reconnected,
        state
            .take_request(at + Duration::from_secs(11))
            .expect("the new session gets a fresh attempt")
            .reason
    );
}

#[test]
fn dynamic_sessions_connect_with_the_local_display_scale() {
    // A configured 100% would otherwise be re-scoped to the local 150% by the
    // first viewport flush, re-rendering the session right after login.
    assert_eq!(150, connect_desktop_scale_factor(true, 100, 1.5));
    assert_eq!(100, connect_desktop_scale_factor(true, 100, 1.0));
    assert_eq!(200, connect_desktop_scale_factor(true, 180, 2.0));
    // Fixed-size sessions never receive viewport flushes, so they keep the
    // configured scale.
    assert_eq!(180, connect_desktop_scale_factor(false, 180, 1.5));
}

#[test]
fn dynamic_sessions_never_request_client_side_smart_sizing() {
    // A viewport-sized session has nothing to fit; smart sizing would only
    // stretch the session (pointer included) on a transient size mismatch.
    assert!(!request_smart_sizing(true, true));
    assert!(!request_smart_sizing(true, false));
    // A fixed-size session keeps the user's choice.
    assert!(request_smart_sizing(false, true));
    assert!(!request_smart_sizing(false, false));
}
