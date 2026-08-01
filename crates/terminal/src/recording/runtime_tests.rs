use super::{
    RecordingBackend, RecordingCompleteness, RecordingFileConfig, RecordingFileLimits,
    RecordingLimit, RecordingMetadata, RecordingQueueLimits, RecordingRuntime,
    RecordingRuntimeConfig, RecordingRuntimeError, RecordingStartRequest, RecordingState,
    RecordingTapOutcome, RecordingTransition, RecordingWorkerTestGate, partial_recording_path,
    read_recording,
};
use crate::TerminalSize;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tempfile::tempdir;

fn metadata(capture_input: bool) -> RecordingMetadata {
    RecordingMetadata {
        recording_id: "runtime-recording".to_string(),
        session_id: "runtime-session".to_string(),
        backend: RecordingBackend::Local,
        application_version: "0.1.0-test".to_string(),
        started_at_unix_ms: 1_700_000_000_123,
        capture_input,
    }
}

fn initial_size() -> TerminalSize {
    TerminalSize {
        rows: 24,
        cols: 80,
        pixel_width: 640,
        pixel_height: 480,
    }
}

fn start_request(
    final_path: impl Into<std::path::PathBuf>,
    capture_input: bool,
) -> RecordingStartRequest {
    RecordingStartRequest {
        final_path: final_path.into(),
        metadata: metadata(capture_input),
        initial_size: initial_size(),
        recording: super::RecordingConfig {
            capture_input,
            ..super::RecordingConfig::default()
        },
    }
}

#[test]
fn inactive_tap_has_no_queue_or_payload_side_effects() {
    let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default()).unwrap();
    let tap = runtime.tap();

    assert_eq!(
        RecordingTapOutcome::Inactive,
        tap.record_output(b"not-copied")
    );
    assert_eq!(
        RecordingTapOutcome::Inactive,
        tap.record_input(b"not-copied")
    );
    assert_eq!(
        RecordingTapOutcome::Inactive,
        tap.record_resize(initial_size())
    );
    assert_eq!(
        RecordingTapOutcome::Inactive,
        tap.record_marker("not-copied")
    );
    assert_eq!(0, runtime.queue_snapshot().pending_events);
    assert_eq!(0, runtime.queue_snapshot().pending_bytes);

    runtime.shutdown().unwrap();
}

#[test]
fn runtime_records_one_ordered_timeline_across_pause_and_resume() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("lifecycle.cast");
    let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default()).unwrap();
    let tap = runtime.tap();

    assert_eq!(
        RecordingTransition::Changed,
        runtime.start(start_request(&final_path, false)).unwrap()
    );
    assert_eq!(
        RecordingTapOutcome::Accepted,
        tap.record_output(b"before-pause")
    );
    assert_eq!(
        RecordingTapOutcome::Accepted,
        tap.record_resize(TerminalSize {
            rows: 30,
            cols: 100,
            pixel_width: 900,
            pixel_height: 600,
        })
    );
    assert_eq!(RecordingTransition::Changed, runtime.pause().unwrap());
    assert_eq!(
        RecordingTapOutcome::Inactive,
        tap.record_output(b"while-paused")
    );
    assert_eq!(RecordingTransition::Changed, runtime.resume().unwrap());
    assert_eq!(
        RecordingTapOutcome::Accepted,
        tap.record_marker("after-resume")
    );
    assert_eq!(
        RecordingTapOutcome::Accepted,
        tap.record_output(b"after-resume")
    );

    assert_eq!(RecordingTransition::Changed, runtime.stop().unwrap());
    let snapshot = runtime.snapshot();
    assert_eq!(RecordingState::Stopped, snapshot.state);
    assert_eq!(4, snapshot.event_count);
    assert!(final_path.exists());
    assert!(!partial_recording_path(&final_path).unwrap().exists());

    let recording = read_recording(&final_path, RecordingFileLimits::default()).unwrap();
    assert_eq!(RecordingCompleteness::Complete, recording.completeness);
    assert_eq!(4, recording.events.len());
    assert!(matches!(
        &recording.events[0].kind,
        super::RecordingEventKind::Output(data) if data == b"before-pause"
    ));
    assert!(matches!(
        recording.events[1].kind,
        super::RecordingEventKind::Resize(_)
    ));
    assert!(matches!(
        &recording.events[2].kind,
        super::RecordingEventKind::Marker(marker) if marker == "after-resume"
    ));
    assert!(matches!(
        &recording.events[3].kind,
        super::RecordingEventKind::Output(data) if data == b"after-resume"
    ));

    runtime.shutdown().unwrap();
}

#[test]
fn input_capture_is_never_queued_without_explicit_opt_in() {
    let temp = tempdir().unwrap();
    let private_path = temp.path().join("private.cast");
    let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default()).unwrap();
    let tap = runtime.tap();
    runtime.start(start_request(&private_path, false)).unwrap();

    assert_eq!(
        RecordingTapOutcome::InputDisabled,
        tap.record_input(b"secret")
    );
    assert_eq!(0, runtime.queue_snapshot().pending_events);
    runtime.stop().unwrap();
    let recording = read_recording(&private_path, RecordingFileLimits::default()).unwrap();
    assert!(recording.events.is_empty());
    runtime.shutdown().unwrap();

    let opted_in_path = temp.path().join("opted-in.cast");
    let opted_in = RecordingRuntime::new(RecordingRuntimeConfig::default()).unwrap();
    let opted_in_tap = opted_in.tap();
    opted_in.start(start_request(&opted_in_path, true)).unwrap();
    assert_eq!(
        RecordingTapOutcome::Accepted,
        opted_in_tap.record_input(b"disclosed-input")
    );
    opted_in.stop().unwrap();
    let recording = read_recording(&opted_in_path, RecordingFileLimits::default()).unwrap();
    assert!(matches!(
        &recording.events[0].kind,
        super::RecordingEventKind::Input(data) if data == b"disclosed-input"
    ));
    opted_in.shutdown().unwrap();
}

#[test]
fn pending_byte_overflow_fails_closed_without_blocking_the_producer() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("overflow.cast");
    let runtime = RecordingRuntime::new(RecordingRuntimeConfig {
        queue: RecordingQueueLimits {
            max_pending_events: 8,
            max_pending_bytes: 3,
            max_pending_controls: 8,
        },
        ..RecordingRuntimeConfig::default()
    })
    .unwrap();
    let tap = runtime.tap();
    runtime.start(start_request(&final_path, false)).unwrap();

    let started = Instant::now();
    assert_eq!(
        RecordingTapOutcome::QueueFull(RecordingLimit::PendingBytes),
        tap.record_output(b"four")
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(
        runtime.snapshot().state,
        RecordingState::Failed(super::RecordingFailure::LimitReached(
            RecordingLimit::PendingBytes
        ))
    ));
    assert!(matches!(
        runtime.stop(),
        Err(RecordingRuntimeError::Recording(
            super::RecordingFailure::LimitReached(RecordingLimit::PendingBytes)
        ))
    ));
    assert!(!final_path.exists());
    assert!(partial_recording_path(&final_path).unwrap().exists());

    runtime.shutdown().unwrap();
}

#[test]
fn pending_event_overflow_counts_in_flight_writer_work() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("event-overflow.cast");
    let gate = RecordingWorkerTestGate::blocked();
    let runtime = RecordingRuntime::new_with_test_gate(
        RecordingRuntimeConfig {
            queue: RecordingQueueLimits {
                max_pending_events: 1,
                max_pending_bytes: 1024,
                max_pending_controls: 8,
            },
            ..RecordingRuntimeConfig::default()
        },
        gate.clone(),
    )
    .unwrap();
    let tap = runtime.tap();
    runtime.start(start_request(&final_path, false)).unwrap();

    assert_eq!(RecordingTapOutcome::Accepted, tap.record_output(b"first"));
    gate.wait_until_worker_is_blocked();
    assert_eq!(
        RecordingTapOutcome::QueueFull(RecordingLimit::PendingEvents),
        tap.record_output(b"second")
    );
    assert_eq!(1, runtime.queue_snapshot().pending_events);

    gate.release();
    assert!(matches!(
        runtime.stop(),
        Err(RecordingRuntimeError::Recording(
            super::RecordingFailure::LimitReached(RecordingLimit::PendingEvents)
        ))
    ));
    assert!(!final_path.exists());
    assert!(partial_recording_path(&final_path).unwrap().exists());
    runtime.shutdown().unwrap();
}

#[test]
fn writer_failure_becomes_failed_and_never_publishes_the_partial() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("writer-failure.cast");
    let (states_tx, states_rx) = mpsc::channel();
    let runtime = RecordingRuntime::with_observer(
        RecordingRuntimeConfig {
            file: RecordingFileConfig {
                limits: RecordingFileLimits {
                    max_serialized_event_bytes: 16,
                    ..RecordingFileLimits::default()
                },
                ..RecordingFileConfig::default()
            },
            ..RecordingRuntimeConfig::default()
        },
        move |snapshot| {
            let _ = states_tx.send(snapshot.state);
        },
    )
    .unwrap();
    let tap = runtime.tap();
    runtime.start(start_request(&final_path, false)).unwrap();

    assert_eq!(
        RecordingTapOutcome::Accepted,
        tap.record_output(b"this event cannot fit on the wire")
    );
    assert!(matches!(
        runtime.pause(),
        Err(RecordingRuntimeError::Recording(
            super::RecordingFailure::Storage(_)
        ))
    ));
    assert!(matches!(
        runtime.snapshot().state,
        RecordingState::Failed(super::RecordingFailure::Storage(_))
    ));
    assert!(!final_path.exists());
    assert!(partial_recording_path(&final_path).unwrap().exists());
    assert!(
        states_rx
            .try_iter()
            .any(|state| matches!(state, RecordingState::Failed(_)))
    );

    runtime.shutdown().unwrap();
}

#[test]
fn stop_and_shutdown_are_idempotent_and_publish_once() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("idempotent.cast");
    let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default()).unwrap();
    runtime.start(start_request(&final_path, false)).unwrap();
    runtime.tap().record_output(b"published-once");

    assert_eq!(RecordingTransition::Changed, runtime.stop().unwrap());
    assert_eq!(RecordingTransition::Unchanged, runtime.stop().unwrap());
    assert_eq!(RecordingTransition::Unchanged, runtime.shutdown().unwrap());
    assert_eq!(RecordingTransition::Unchanged, runtime.shutdown().unwrap());
    assert!(final_path.exists());
}

#[test]
fn concurrent_stop_and_shutdown_converge_without_double_publish() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("concurrent-stop.cast");
    let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default()).unwrap();
    runtime.start(start_request(&final_path, false)).unwrap();
    runtime.tap().record_output(b"one");

    let stop_runtime = runtime.clone();
    let stop_thread = std::thread::spawn(move || stop_runtime.stop());
    let shutdown_runtime = runtime.clone();
    let shutdown_thread = std::thread::spawn(move || shutdown_runtime.shutdown());

    assert!(stop_thread.join().unwrap().is_ok());
    assert!(shutdown_thread.join().unwrap().is_ok());
    assert_eq!(RecordingState::Stopped, runtime.snapshot().state);
    assert!(final_path.exists());
    assert!(!partial_recording_path(&final_path).unwrap().exists());
}

#[test]
fn dropping_a_live_runtime_preserves_partial_identity() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("abandoned.cast");
    let partial_path = partial_recording_path(&final_path).unwrap();
    {
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default()).unwrap();
        runtime.start(start_request(&final_path, false)).unwrap();
        assert_eq!(
            RecordingTapOutcome::Accepted,
            runtime.tap().record_output(b"recoverable")
        );
        runtime.pause().unwrap();
    }

    assert!(!final_path.exists());
    assert!(partial_path.exists());
}
