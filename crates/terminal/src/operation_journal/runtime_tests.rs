use super::runtime::OperationJournalWorkerTestGate;
use super::{
    OperationGenerationId, OperationJournalAttempt, OperationJournalError,
    OperationJournalHistoryConfig, OperationJournalHistoryStore, OperationJournalPersistencePaths,
    OperationJournalQueueLimits, OperationJournalRuntime, OperationJournalRuntimeConfig,
    OperationJournalRuntimeError, OperationJournalRuntimeHealth, OperationJournalScope,
    OperationJournalSessionId, OperationKind, OperationStatus, SensitiveOperationPayload,
};
use std::fs;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn generation(value: u64) -> OperationGenerationId {
    OperationGenerationId::new(value).expect("generation must be non-zero")
}

fn runtime_config(directory: &tempfile::TempDir) -> OperationJournalRuntimeConfig {
    OperationJournalRuntimeConfig {
        root: directory.path().to_path_buf(),
        queue: OperationJournalQueueLimits::default(),
        persistence: Default::default(),
    }
}

fn recover_single_history(
    directory: &tempfile::TempDir,
    scope: &OperationJournalScope,
) -> super::OperationJournalHistorySnapshot {
    let store = OperationJournalHistoryStore::new(
        directory.path(),
        OperationJournalHistoryConfig::default(),
    )
    .expect("valid history store");
    let discovery = store.discover(scope, &[]);
    assert!(
        discovery.warnings().is_empty(),
        "unexpected recovery warnings: {:?}",
        discovery.warnings()
    );
    assert_eq!(discovery.histories().len(), 1);
    discovery.histories()[0].clone()
}

#[test]
fn accepted_input_is_persisted_as_queued_then_sent_without_raw_payload() {
    let directory = tempfile::tempdir().expect("temp directory");
    let scope = OperationJournalScope::local();
    let session_id = OperationJournalSessionId::from_string("terminal_session_live_runtime_input");
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        session_id.clone(),
        scope.clone(),
        generation(1),
        1_000,
    )
    .expect("spawn journal runtime");

    let payload =
        SensitiveOperationPayload::opaque(b"password=live-runtime-secret".to_vec()).redact();
    let operation_id = runtime
        .record_attempt(
            OperationKind::UserInput,
            None,
            Some(payload),
            1_010,
            OperationJournalAttempt::sent(1_020),
        )
        .expect("accept input journal item");
    runtime
        .shutdown(Duration::from_secs(5))
        .expect("flush and stop journal runtime");

    let history = recover_single_history(&directory, &scope);
    assert_eq!(history.session_id(), &session_id);
    let operation = history
        .journal()
        .operation(&operation_id)
        .expect("recover recorded operation");
    assert_eq!(operation.kind(), OperationKind::UserInput);
    assert_eq!(operation.status(), OperationStatus::Sent);
    assert_eq!(
        operation
            .transitions()
            .iter()
            .map(|transition| transition.status())
            .collect::<Vec<_>>(),
        vec![OperationStatus::Queued, OperationStatus::Sent]
    );
    assert_eq!(
        operation
            .redacted_payload()
            .expect("redacted payload")
            .original_byte_len(),
        b"password=live-runtime-secret".len() as u64
    );

    for entry in fs::read_dir(directory.path()).expect("read journal directory") {
        let entry = entry.expect("journal directory entry");
        let contents = fs::read(entry.path()).expect("read journal artifact");
        assert!(
            !contents
                .windows(b"live-runtime-secret".len())
                .any(|window| window == b"live-runtime-secret"),
            "raw secret leaked into {}",
            entry.path().display()
        );
    }
}

#[test]
fn invalid_structured_raw_payload_is_rejected_before_queueing() {
    let directory = tempfile::tempdir().expect("temp directory");
    let scope = OperationJournalScope::local();
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        OperationJournalSessionId::from_string("terminal_session_live_runtime_invalid_payload"),
        scope.clone(),
        generation(1),
        1_000,
    )
    .expect("spawn journal runtime");

    let error = runtime
        .record_attempt(
            OperationKind::UserInput,
            None,
            Some(
                SensitiveOperationPayload::structured(serde_json::json!({
                    "command": "echo visible"
                }))
                .redact(),
            ),
            1_010,
            OperationJournalAttempt::sent(1_020),
        )
        .expect_err("raw terminal operations require opaque summaries");
    assert!(matches!(
        error,
        OperationJournalRuntimeError::InvalidOperation(
            OperationJournalError::StructuredPayloadNotAllowed {
                operation_kind: OperationKind::UserInput
            }
        )
    ));
    assert_eq!(runtime.snapshot().pending_operations, 0);

    let valid_operation_id = runtime
        .record_attempt(
            OperationKind::UserInput,
            None,
            Some(SensitiveOperationPayload::opaque(b"safe follow-up".to_vec()).redact()),
            1_030,
            OperationJournalAttempt::sent(1_040),
        )
        .expect("runtime remains usable after rejecting invalid payload");
    runtime
        .shutdown(Duration::from_secs(5))
        .expect("flush and stop journal runtime");

    let history = recover_single_history(&directory, &scope);
    assert!(
        history.journal().operation(&valid_operation_id).is_some(),
        "valid follow-up operation was not persisted"
    );
    assert_eq!(history.journal().generations()[0].operations().len(), 1);
}

#[test]
fn reconnect_marks_unfinished_operations_unknown_before_starting_the_new_generation() {
    let directory = tempfile::tempdir().expect("temp directory");
    let scope = OperationJournalScope::local();
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        OperationJournalSessionId::from_string("terminal_session_live_runtime_reconnect"),
        scope.clone(),
        generation(1),
        2_000,
    )
    .expect("spawn journal runtime");

    let operation_id = runtime
        .record_attempt(
            OperationKind::Paste,
            None,
            Some(SensitiveOperationPayload::opaque(b"pending paste".to_vec()).redact()),
            2_010,
            OperationJournalAttempt::sent(2_020),
        )
        .expect("accept sent operation");
    runtime
        .begin_generation(generation(2), 2_100)
        .expect("accept reconnect generation");
    runtime
        .shutdown(Duration::from_secs(5))
        .expect("flush and stop journal runtime");

    let history = recover_single_history(&directory, &scope);
    let journal = history.journal();
    assert_eq!(journal.generations().len(), 2);
    assert!(journal.generations()[0].is_closed());
    assert_eq!(
        journal
            .operation(&operation_id)
            .expect("recover pre-reconnect operation")
            .status(),
        OperationStatus::Unknown
    );
    assert_eq!(journal.current_generation().id(), generation(2));
    assert!(!journal.current_generation().is_closed());
    assert!(journal.current_generation().operations().is_empty());
}

#[test]
fn journal_snapshot_observes_prior_operations_and_generation_boundaries() {
    let directory = tempfile::tempdir().expect("temp directory");
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        OperationJournalSessionId::from_string("terminal_session_live_runtime_snapshot"),
        OperationJournalScope::local(),
        generation(1),
        2_200,
    )
    .expect("spawn journal runtime");

    let operation_id = runtime
        .record_attempt(
            OperationKind::Command,
            None,
            Some(SensitiveOperationPayload::opaque(b"pending command".to_vec()).redact()),
            2_210,
            OperationJournalAttempt::sent(2_220),
        )
        .expect("accept sent operation");
    runtime
        .begin_generation(generation(2), 2_300)
        .expect("accept reconnect generation");

    let journal = runtime
        .journal_snapshot(Duration::from_secs(5))
        .expect("read live journal without shutting down");

    assert_eq!(journal.generations().len(), 2);
    assert!(journal.generations()[0].is_closed());
    assert_eq!(
        journal
            .operation(&operation_id)
            .expect("snapshot pre-reconnect operation")
            .status(),
        OperationStatus::Unknown
    );
    assert_eq!(journal.current_generation().id(), generation(2));
    assert!(!journal.current_generation().is_closed());

    runtime
        .shutdown(Duration::from_secs(5))
        .expect("stop journal runtime");
}

#[test]
fn journal_snapshot_fails_conservatively_after_runtime_shutdown() {
    let directory = tempfile::tempdir().expect("temp directory");
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        OperationJournalSessionId::from_string("terminal_session_live_runtime_closed_snapshot"),
        OperationJournalScope::local(),
        generation(1),
        2_400,
    )
    .expect("spawn journal runtime");
    runtime
        .shutdown(Duration::from_secs(5))
        .expect("stop journal runtime");

    assert_eq!(
        runtime.journal_snapshot(Duration::from_secs(5)),
        Err(OperationJournalRuntimeError::Closed)
    );
}

#[test]
fn bounded_queue_fails_closed_without_blocking_flush_or_shutdown() {
    let directory = tempfile::tempdir().expect("temp directory");
    let scope = OperationJournalScope::local();
    let session_id =
        OperationJournalSessionId::from_string("terminal_session_live_runtime_queue_full");
    let gate = OperationJournalWorkerTestGate::blocked_before_work();
    let mut config = runtime_config(&directory);
    config.queue = OperationJournalQueueLimits {
        max_pending_operations: 1,
        max_pending_bytes: 1024,
        max_pending_controls: 4,
    };
    let runtime = OperationJournalRuntime::new_with_test_gate(
        config,
        session_id,
        scope.clone(),
        generation(1),
        3_000,
        gate.clone(),
    )
    .expect("spawn journal runtime");
    gate.wait_until_worker_is_blocked();

    let accepted_operation_id = runtime
        .record_attempt(
            OperationKind::UserInput,
            None,
            None,
            3_010,
            OperationJournalAttempt::sent(3_020),
        )
        .expect("accept first operation");
    assert_eq!(
        runtime.record_attempt(
            OperationKind::Paste,
            None,
            None,
            3_030,
            OperationJournalAttempt::sent(3_040),
        ),
        Err(OperationJournalRuntimeError::QueueFull)
    );
    assert_eq!(
        runtime.record_attempt(
            OperationKind::ControlSequence,
            None,
            None,
            3_050,
            OperationJournalAttempt::sent(3_060),
        ),
        Err(OperationJournalRuntimeError::Closed)
    );
    assert_eq!(
        runtime.begin_generation(generation(2), 3_100),
        Err(OperationJournalRuntimeError::Closed)
    );

    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.health, OperationJournalRuntimeHealth::QueueFull);
    assert_eq!(snapshot.pending_operations, 1);
    assert_eq!(snapshot.dropped_operations, 1);

    gate.release();
    runtime
        .flush(Duration::from_secs(5))
        .expect("flush accepted operation after journal queue closes");
    runtime
        .shutdown(Duration::from_secs(5))
        .expect("shutdown remains available after queue exhaustion");

    let history = recover_single_history(&directory, &scope);
    assert!(
        history
            .journal()
            .operation(&accepted_operation_id)
            .is_some()
    );
    assert_eq!(history.journal().current_generation().operations().len(), 1);
}

#[test]
fn in_flight_operation_remains_within_queue_budget_until_persisted() {
    let directory = tempfile::tempdir().expect("temp directory");
    let gate = OperationJournalWorkerTestGate::blocked_before_persist();
    let mut config = runtime_config(&directory);
    config.queue = OperationJournalQueueLimits {
        max_pending_operations: 1,
        max_pending_bytes: 1024,
        max_pending_controls: 4,
    };
    let runtime = OperationJournalRuntime::new_with_test_gate(
        config,
        OperationJournalSessionId::from_string("terminal_session_live_runtime_in_flight_budget"),
        OperationJournalScope::local(),
        generation(1),
        3_200,
        gate.clone(),
    )
    .expect("spawn journal runtime");

    runtime
        .record_attempt(
            OperationKind::UserInput,
            None,
            None,
            3_210,
            OperationJournalAttempt::sent(3_220),
        )
        .expect("accept first operation");
    gate.wait_until_worker_is_blocked();

    let snapshot = runtime.snapshot();
    let second_result = runtime.record_attempt(
        OperationKind::Paste,
        None,
        None,
        3_230,
        OperationJournalAttempt::sent(3_240),
    );
    gate.release();
    let shutdown_result = runtime.shutdown(Duration::from_secs(5));

    assert_eq!(snapshot.pending_operations, 1);
    assert_eq!(second_result, Err(OperationJournalRuntimeError::QueueFull));
    shutdown_result.expect("persist the accepted operation");
}

#[test]
fn saturated_control_queue_fails_closed_but_reserves_shutdown() {
    let directory = tempfile::tempdir().expect("temp directory");
    let gate = OperationJournalWorkerTestGate::blocked_before_work();
    let mut config = runtime_config(&directory);
    config.queue = OperationJournalQueueLimits {
        max_pending_operations: 4,
        max_pending_bytes: 4 * 1024,
        max_pending_controls: 1,
    };
    let runtime = OperationJournalRuntime::new_with_test_gate(
        config,
        OperationJournalSessionId::from_string("terminal_session_live_runtime_control_queue_full"),
        OperationJournalScope::local(),
        generation(1),
        3_400,
        gate.clone(),
    )
    .expect("spawn journal runtime");
    gate.wait_until_worker_is_blocked();

    runtime
        .begin_generation(generation(2), 3_500)
        .expect("accept the first control");
    assert_eq!(
        runtime.begin_generation(generation(3), 3_600),
        Err(OperationJournalRuntimeError::ControlQueueFull)
    );
    assert_eq!(
        runtime.record_attempt(
            OperationKind::UserInput,
            None,
            None,
            3_610,
            OperationJournalAttempt::sent(3_620),
        ),
        Err(OperationJournalRuntimeError::Closed)
    );
    let flush_result = runtime.flush(Duration::from_secs(5));

    gate.release();
    let shutdown_result = runtime.shutdown(Duration::from_secs(5));

    assert_eq!(
        flush_result,
        Err(OperationJournalRuntimeError::ControlQueueFull)
    );
    shutdown_result.expect("shutdown uses its reserved control slot");
}

#[test]
fn startup_failure_becomes_unavailable_without_blocking_callers() {
    let directory = tempfile::tempdir().expect("temp directory");
    let unusable_root = directory.path().join("not-a-directory");
    fs::write(&unusable_root, b"file").expect("create unusable journal root");
    let (health_sender, health_receiver) = mpsc::channel();
    let runtime = OperationJournalRuntime::with_observer(
        OperationJournalRuntimeConfig::new(unusable_root),
        OperationJournalSessionId::from_string("terminal_session_live_runtime_startup_unavailable"),
        OperationJournalScope::local(),
        generation(1),
        4_000,
        move |health| {
            let _ = health_sender.send(health);
        },
    )
    .expect("worker spawn is independent from storage availability");

    assert_eq!(
        health_receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("startup health transition"),
        OperationJournalRuntimeHealth::Unavailable
    );
    assert!(matches!(
        runtime.shutdown(Duration::from_secs(5)),
        Err(OperationJournalRuntimeError::Unavailable(_))
    ));
    assert!(matches!(
        runtime.record_attempt(
            OperationKind::UserInput,
            None,
            None,
            4_010,
            OperationJournalAttempt::sent(4_020),
        ),
        Err(OperationJournalRuntimeError::Unavailable(_))
    ));
    assert_eq!(
        runtime.snapshot().health,
        OperationJournalRuntimeHealth::Unavailable
    );
}

#[test]
fn initial_persistence_failure_leaves_manifest_for_safe_retry() {
    let directory = tempfile::tempdir().expect("temp directory");
    let session_id =
        OperationJournalSessionId::from_string("terminal_session_live_runtime_startup_retry");
    let paths = OperationJournalPersistencePaths::for_session(directory.path(), &session_id);
    let mut failing_config = runtime_config(&directory);
    failing_config.persistence.max_entry_bytes = 1;
    let failed_runtime = OperationJournalRuntime::new(
        failing_config,
        session_id.clone(),
        OperationJournalScope::local(),
        generation(1),
        5_000,
    )
    .expect("spawn journal runtime");

    assert!(matches!(
        failed_runtime.shutdown(Duration::from_secs(5)),
        Err(OperationJournalRuntimeError::PersistenceFailed(_))
    ));
    assert_eq!(
        failed_runtime.snapshot().health,
        OperationJournalRuntimeHealth::PersistenceFailed
    );
    drop(failed_runtime);
    assert!(
        paths.session_manifest_path().is_file(),
        "the discoverable manifest must be published before the first journal snapshot"
    );
    assert!(
        !paths.append_log_path().exists(),
        "serialization failure must not publish a journal entry"
    );

    let retry_runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        session_id,
        OperationJournalScope::local(),
        generation(1),
        5_100,
    )
    .expect("retry journal runtime");
    retry_runtime
        .shutdown(Duration::from_secs(5))
        .expect("retry can safely publish the initial journal");
}

#[test]
fn post_start_persistence_failure_disables_later_journal_writes() {
    let directory = tempfile::tempdir().expect("temp directory");
    let session_id =
        OperationJournalSessionId::from_string("terminal_session_live_runtime_write_failure");
    let paths = OperationJournalPersistencePaths::for_session(directory.path(), &session_id);
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        session_id,
        OperationJournalScope::local(),
        generation(1),
        6_000,
    )
    .expect("spawn journal runtime");
    runtime
        .flush(Duration::from_secs(5))
        .expect("wait for initial journal publication");

    fs::remove_file(paths.append_log_path()).expect("remove live append log");
    fs::create_dir(paths.append_log_path()).expect("replace append log with a directory");
    runtime
        .record_attempt(
            OperationKind::UserInput,
            None,
            None,
            6_010,
            OperationJournalAttempt::sent(6_020),
        )
        .expect("producer accepts work before asynchronous persistence fails");
    assert!(matches!(
        runtime.flush(Duration::from_secs(5)),
        Err(OperationJournalRuntimeError::PersistenceFailed(_))
    ));
    assert_eq!(
        runtime.snapshot().health,
        OperationJournalRuntimeHealth::PersistenceFailed
    );
    assert!(matches!(
        runtime.record_attempt(
            OperationKind::Paste,
            None,
            None,
            6_030,
            OperationJournalAttempt::sent(6_040),
        ),
        Err(OperationJournalRuntimeError::PersistenceFailed(_))
    ));
    assert!(matches!(
        runtime.shutdown(Duration::from_secs(5)),
        Err(OperationJournalRuntimeError::PersistenceFailed(_))
    ));
}

#[test]
fn observer_panics_cannot_stop_the_single_writer() {
    let directory = tempfile::tempdir().expect("temp directory");
    let runtime = OperationJournalRuntime::with_observer(
        runtime_config(&directory),
        OperationJournalSessionId::from_string("terminal_session_live_runtime_panicking_observer"),
        OperationJournalScope::local(),
        generation(1),
        7_000,
        |_| panic!("observer failure must stay outside the writer"),
    )
    .expect("spawn journal runtime");

    runtime
        .flush(Duration::from_secs(5))
        .expect("panicking observer cannot strand the queue");
    runtime
        .record_attempt(
            OperationKind::ApplicationOperation,
            None,
            None,
            7_010,
            OperationJournalAttempt::sent(7_020),
        )
        .expect("writer still accepts operations");
    runtime
        .shutdown(Duration::from_secs(5))
        .expect("writer still shuts down");
}

#[test]
fn shutdown_timeout_detaches_without_waiting_for_a_blocked_worker() {
    let directory = tempfile::tempdir().expect("temp directory");
    let gate = OperationJournalWorkerTestGate::blocked_before_work();
    let runtime = OperationJournalRuntime::new_with_test_gate(
        runtime_config(&directory),
        OperationJournalSessionId::from_string("terminal_session_live_runtime_shutdown_timeout"),
        OperationJournalScope::local(),
        generation(1),
        8_000,
        gate.clone(),
    )
    .expect("spawn journal runtime");
    gate.wait_until_worker_is_blocked();

    let started = Instant::now();
    assert_eq!(
        runtime.shutdown(Duration::from_millis(10)),
        Err(OperationJournalRuntimeError::TimedOut)
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "bounded shutdown must not join a blocked worker"
    );
    drop(runtime);

    gate.release();
    assert!(
        gate.wait_until_worker_exited(Duration::from_secs(5)),
        "detached worker must exit after its blocking I/O boundary returns"
    );
}
