use super::{
    RecordingConfig, RecordingController, RecordingEventKind, RecordingFailure, RecordingLimit,
    RecordingLimits, RecordingState, RecordingTransition,
};
use crate::TerminalSize;
use std::time::Duration;

fn seconds(value: u64) -> Duration {
    Duration::from_secs(value)
}

#[test]
fn recording_lifecycle_excludes_paused_time_and_stops_idempotently() {
    let mut recorder = RecordingController::new(RecordingConfig::default());

    assert_eq!(
        RecordingTransition::Changed,
        recorder.start(seconds(10)).unwrap()
    );
    let first = recorder
        .record_output(seconds(12), b"first".to_vec())
        .unwrap()
        .unwrap();
    assert_eq!(seconds(2), first.elapsed);

    assert_eq!(
        RecordingTransition::Changed,
        recorder.pause(seconds(13)).unwrap()
    );
    assert_eq!(
        None,
        recorder
            .record_output(seconds(20), b"ignored".to_vec())
            .unwrap()
    );
    assert_eq!(
        RecordingTransition::Changed,
        recorder.resume(seconds(23)).unwrap()
    );

    let resumed = recorder
        .record_output(seconds(25), b"second".to_vec())
        .unwrap()
        .unwrap();
    assert_eq!(seconds(5), resumed.elapsed);

    assert_eq!(
        RecordingTransition::Changed,
        recorder.request_stop(seconds(26)).unwrap()
    );
    assert_eq!(
        RecordingTransition::Unchanged,
        recorder.request_stop(seconds(99)).unwrap()
    );
    assert_eq!(RecordingTransition::Changed, recorder.complete_stop());
    assert_eq!(RecordingTransition::Unchanged, recorder.complete_stop());
    assert_eq!(&RecordingState::Stopped, recorder.state());
}

#[test]
fn input_capture_is_disabled_by_default_and_requires_explicit_opt_in() {
    let mut default_recorder = RecordingController::new(RecordingConfig::default());
    default_recorder.start(Duration::ZERO).unwrap();

    assert_eq!(
        None,
        default_recorder
            .record_input(seconds(1), b"secret".to_vec())
            .unwrap()
    );
    assert_eq!(0, default_recorder.event_count());

    let mut recorder = RecordingController::new(RecordingConfig {
        capture_input: true,
        ..RecordingConfig::default()
    });
    recorder.start(Duration::ZERO).unwrap();

    let input = recorder
        .record_input(seconds(1), b"visible".to_vec())
        .unwrap()
        .unwrap();
    assert!(matches!(
        input.kind,
        RecordingEventKind::Input(ref data) if data == b"visible"
    ));
}

#[test]
fn output_resize_and_marker_events_share_one_monotonic_timeline() {
    let mut recorder = RecordingController::new(RecordingConfig::default());
    recorder.start(seconds(4)).unwrap();

    let output = recorder
        .record_output(seconds(5), b"hello".to_vec())
        .unwrap()
        .unwrap();
    let resize = recorder
        .record_resize(
            seconds(6),
            TerminalSize {
                rows: 30,
                cols: 100,
                pixel_width: 900,
                pixel_height: 600,
            },
        )
        .unwrap()
        .unwrap();
    let marker = recorder
        .record_marker(seconds(7), "deploy".to_string())
        .unwrap()
        .unwrap();

    assert!(matches!(output.kind, RecordingEventKind::Output(_)));
    assert!(matches!(resize.kind, RecordingEventKind::Resize(_)));
    assert!(matches!(marker.kind, RecordingEventKind::Marker(_)));
    assert_eq!(
        vec![seconds(1), seconds(2), seconds(3)],
        vec![output.elapsed, resize.elapsed, marker.elapsed]
    );
}

#[test]
fn event_count_limit_fails_the_recording_before_accepting_an_extra_event() {
    let mut recorder = RecordingController::new(RecordingConfig {
        limits: RecordingLimits {
            max_events: 1,
            ..RecordingLimits::default()
        },
        ..RecordingConfig::default()
    });
    recorder.start(Duration::ZERO).unwrap();
    recorder
        .record_output(seconds(1), b"accepted".to_vec())
        .unwrap();

    let failure = recorder
        .record_output(seconds(2), b"rejected".to_vec())
        .unwrap_err();

    assert_eq!(
        RecordingFailure::LimitReached(RecordingLimit::EventCount),
        failure
    );
    assert_eq!(&RecordingState::Failed(failure), recorder.state());
    assert_eq!(1, recorder.event_count());
}

#[test]
fn per_event_and_total_payload_limits_are_enforced_independently() {
    let mut per_event = RecordingController::new(RecordingConfig {
        limits: RecordingLimits {
            max_event_bytes: 3,
            ..RecordingLimits::default()
        },
        ..RecordingConfig::default()
    });
    per_event.start(Duration::ZERO).unwrap();
    assert_eq!(
        RecordingFailure::LimitReached(RecordingLimit::EventBytes),
        per_event
            .record_output(seconds(1), b"four".to_vec())
            .unwrap_err()
    );

    let mut total = RecordingController::new(RecordingConfig {
        limits: RecordingLimits {
            max_event_bytes: 8,
            max_payload_bytes: 5,
            ..RecordingLimits::default()
        },
        ..RecordingConfig::default()
    });
    total.start(Duration::ZERO).unwrap();
    total.record_output(seconds(1), b"123".to_vec()).unwrap();
    assert_eq!(
        RecordingFailure::LimitReached(RecordingLimit::PayloadBytes),
        total
            .record_output(seconds(2), b"456".to_vec())
            .unwrap_err()
    );
    assert_eq!(3, total.payload_bytes());
}

#[test]
fn duration_limit_and_backwards_clock_fail_closed() {
    let mut duration_limited = RecordingController::new(RecordingConfig {
        limits: RecordingLimits {
            max_duration: seconds(2),
            ..RecordingLimits::default()
        },
        ..RecordingConfig::default()
    });
    duration_limited.start(seconds(10)).unwrap();
    assert_eq!(
        RecordingFailure::LimitReached(RecordingLimit::Duration),
        duration_limited
            .record_output(seconds(13), b"late".to_vec())
            .unwrap_err()
    );

    let mut backwards = RecordingController::new(RecordingConfig::default());
    backwards.start(seconds(10)).unwrap();
    backwards
        .record_output(seconds(12), b"first".to_vec())
        .unwrap();
    assert_eq!(
        RecordingFailure::ClockMovedBackwards,
        backwards
            .record_output(seconds(11), b"second".to_vec())
            .unwrap_err()
    );
    assert_eq!(
        &RecordingState::Failed(RecordingFailure::ClockMovedBackwards),
        backwards.state()
    );
}
