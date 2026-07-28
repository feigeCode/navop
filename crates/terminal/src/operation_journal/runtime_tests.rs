use super::runtime::OperationJournalWorkerTestGate;
use super::{
    OperationGenerationId, OperationId, OperationJournalAttempt, OperationJournalError,
    OperationJournalHistoryConfig, OperationJournalHistoryStore, OperationJournalPersistencePaths,
    OperationJournalQueueLimits, OperationJournalRuntime, OperationJournalRuntimeConfig,
    OperationJournalRuntimeError, OperationJournalRuntimeHealth, OperationJournalScope,
    OperationJournalSessionId, OperationKind, OperationStatus, SensitiveOperationPayload,
};
use std::fs;
use std::sync::mpsc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
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
fn guarded_retry_persists_a_fresh_queued_child_before_dispatch() {
    let directory = tempfile::tempdir().expect("temp directory");
    let scope = OperationJournalScope::local();
    let session_id =
        OperationJournalSessionId::from_string("terminal_session_live_runtime_guarded_retry");
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        session_id,
        scope.clone(),
        generation(1),
        2_500,
    )
    .expect("spawn journal runtime");
    let parent_operation_id = runtime
        .record_attempt(
            OperationKind::Command,
            None,
            Some(SensitiveOperationPayload::opaque(b"original command".to_vec()).redact()),
            2_510,
            OperationJournalAttempt::failed(2_520),
        )
        .expect("record failed parent");
    runtime
        .flush(Duration::from_secs(5))
        .expect("persist failed parent");

    let root = directory.path().to_path_buf();
    let scope_for_dispatch = scope.clone();
    let parent_for_dispatch = parent_operation_id.clone();
    let observed_persisted_child = Arc::new(AtomicBool::new(false));
    let observed_persisted_child_for_dispatch = observed_persisted_child.clone();
    let child_operation_id = runtime
        .execute_guarded_retry(
            generation(1),
            &parent_operation_id,
            SensitiveOperationPayload::opaque(b"manually re-entered command".to_vec()).redact(),
            2_530,
            2_540,
            move || {
                let store = OperationJournalHistoryStore::new(
                    &root,
                    OperationJournalHistoryConfig::default(),
                )
                .expect("open history while dispatching");
                let discovery = store.discover(&scope_for_dispatch, &[]);
                let persisted_child = discovery
                    .histories()
                    .first()
                    .and_then(|history| {
                        history
                            .journal()
                            .current_generation()
                            .operations()
                            .iter()
                            .find(|operation| {
                                operation.parent_operation_id() == Some(&parent_for_dispatch)
                            })
                    })
                    .is_some_and(|operation| operation.status() == OperationStatus::Queued);
                observed_persisted_child_for_dispatch.store(persisted_child, Ordering::Release);
                true
            },
            Duration::from_secs(5),
        )
        .expect("persist then dispatch guarded retry");

    assert!(
        observed_persisted_child.load(Ordering::Acquire),
        "the retry dispatch must not run until its queued child is recoverable"
    );
    assert_ne!(child_operation_id, parent_operation_id);
    let journal = runtime
        .journal_snapshot(Duration::from_secs(5))
        .expect("snapshot guarded retry");
    let parent = journal
        .operation(&parent_operation_id)
        .expect("original operation remains present");
    let child = journal
        .operation(&child_operation_id)
        .expect("fresh retry child");
    assert_eq!(parent.status(), OperationStatus::Failed);
    assert_eq!(child.generation_id(), generation(1));
    assert_eq!(
        child.parent_operation_id(),
        Some(&parent_operation_id),
        "retry lineage must not overwrite the original operation"
    );
    assert_eq!(child.kind(), OperationKind::Command);
    assert_eq!(child.status(), OperationStatus::Sent);
    assert_eq!(
        child
            .transitions()
            .iter()
            .map(|transition| transition.status())
            .collect::<Vec<_>>(),
        vec![OperationStatus::Queued, OperationStatus::Sent]
    );

    runtime
        .shutdown(Duration::from_secs(5))
        .expect("stop journal runtime");
}

#[test]
fn guarded_retry_does_not_dispatch_while_its_first_persistence_is_blocked() {
    let directory = tempfile::tempdir().expect("temp directory");
    let gate = OperationJournalWorkerTestGate::blocked_before_persist();
    let runtime = OperationJournalRuntime::new_with_test_gate(
        runtime_config(&directory),
        OperationJournalSessionId::from_string(
            "terminal_session_live_runtime_guarded_retry_persist_order",
        ),
        OperationJournalScope::local(),
        generation(1),
        2_600,
        gate.clone(),
    )
    .expect("spawn journal runtime");
    let parent_operation_id = runtime
        .record_attempt(
            OperationKind::Command,
            None,
            None,
            2_610,
            OperationJournalAttempt::failed(2_620),
        )
        .expect("queue failed parent");
    gate.wait_until_worker_is_blocked();

    let dispatch_count = Arc::new(AtomicUsize::new(0));
    let dispatch_count_for_retry = dispatch_count.clone();
    let runtime_for_retry = runtime.clone();
    let retry = std::thread::spawn(move || {
        runtime_for_retry.execute_guarded_retry(
            generation(1),
            &parent_operation_id,
            SensitiveOperationPayload::opaque(b"manual retry".to_vec()).redact(),
            2_630,
            2_640,
            move || {
                dispatch_count_for_retry.fetch_add(1, Ordering::AcqRel);
                true
            },
            Duration::from_secs(5),
        )
    });

    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(
        dispatch_count.load(Ordering::Acquire),
        0,
        "dispatch must remain fail-closed before persistence completes"
    );
    gate.release();
    retry
        .join()
        .expect("retry caller thread")
        .expect("retry succeeds after persistence unblocks");
    assert_eq!(dispatch_count.load(Ordering::Acquire), 1);

    runtime
        .shutdown(Duration::from_secs(5))
        .expect("stop journal runtime");
}

#[test]
fn guarded_retry_queue_rejection_never_dispatches() {
    let directory = tempfile::tempdir().expect("temp directory");
    let gate = OperationJournalWorkerTestGate::blocked_before_work();
    let mut config = runtime_config(&directory);
    config.queue = OperationJournalQueueLimits {
        max_pending_operations: 1,
        max_pending_bytes: 1024,
        max_pending_controls: 4,
    };
    let runtime = OperationJournalRuntime::new_with_test_gate(
        config,
        OperationJournalSessionId::from_string(
            "terminal_session_live_runtime_guarded_retry_queue_full",
        ),
        OperationJournalScope::local(),
        generation(1),
        2_700,
        gate.clone(),
    )
    .expect("spawn journal runtime");
    gate.wait_until_worker_is_blocked();
    let parent_operation_id = runtime
        .record_attempt(
            OperationKind::Command,
            None,
            None,
            2_710,
            OperationJournalAttempt::failed(2_720),
        )
        .expect("fill bounded operation queue");

    let dispatch_count = Arc::new(AtomicUsize::new(0));
    let dispatch_count_for_retry = dispatch_count.clone();
    assert_eq!(
        runtime.execute_guarded_retry(
            generation(1),
            &parent_operation_id,
            SensitiveOperationPayload::opaque(b"must not dispatch".to_vec()).redact(),
            2_730,
            2_740,
            move || {
                dispatch_count_for_retry.fetch_add(1, Ordering::AcqRel);
                true
            },
            Duration::from_secs(5),
        ),
        Err(OperationJournalRuntimeError::QueueFull)
    );
    assert_eq!(dispatch_count.load(Ordering::Acquire), 0);

    gate.release();
    runtime
        .shutdown(Duration::from_secs(5))
        .expect("persist accepted parent and stop");
}

#[test]
fn guarded_retry_rejects_missing_or_non_terminal_parents_without_dispatch() {
    let directory = tempfile::tempdir().expect("temp directory");
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        OperationJournalSessionId::from_string(
            "terminal_session_live_runtime_guarded_retry_invalid_parent",
        ),
        OperationJournalScope::local(),
        generation(1),
        2_800,
    )
    .expect("spawn journal runtime");
    let dispatch_count = Arc::new(AtomicUsize::new(0));

    let missing_parent = OperationId::from_string("terminal_operation_missing_retry_parent");
    let missing_dispatch_count = dispatch_count.clone();
    assert!(matches!(
        runtime.execute_guarded_retry(
            generation(1),
            &missing_parent,
            SensitiveOperationPayload::opaque(b"missing parent".to_vec()).redact(),
            2_810,
            2_820,
            move || {
                missing_dispatch_count.fetch_add(1, Ordering::AcqRel);
                true
            },
            Duration::from_secs(5),
        ),
        Err(OperationJournalRuntimeError::InvalidOperation(
            OperationJournalError::ParentOperationNotFound { .. }
        ))
    ));

    let non_terminal_parent = runtime
        .record_attempt(
            OperationKind::Command,
            None,
            None,
            2_830,
            OperationJournalAttempt::sent(2_840),
        )
        .expect("record sent parent");
    runtime
        .flush(Duration::from_secs(5))
        .expect("persist sent parent");
    let non_terminal_dispatch_count = dispatch_count.clone();
    assert!(matches!(
        runtime.execute_guarded_retry(
            generation(1),
            &non_terminal_parent,
            SensitiveOperationPayload::opaque(b"non terminal parent".to_vec()).redact(),
            2_850,
            2_860,
            move || {
                non_terminal_dispatch_count.fetch_add(1, Ordering::AcqRel);
                true
            },
            Duration::from_secs(5),
        ),
        Err(OperationJournalRuntimeError::InvalidOperation(
            OperationJournalError::ParentOperationNotTerminal { .. }
        ))
    ));
    assert_eq!(dispatch_count.load(Ordering::Acquire), 0);

    runtime
        .shutdown(Duration::from_secs(5))
        .expect("invalid retry requests do not stop the worker");
}

#[test]
fn guarded_retry_rejects_stale_generation_and_ineligible_parent_without_dispatch() {
    let directory = tempfile::tempdir().expect("temp directory");
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        OperationJournalSessionId::from_string("terminal_session_live_runtime_guarded_retry_stale"),
        OperationJournalScope::local(),
        generation(1),
        2_900,
    )
    .expect("spawn journal runtime");
    let failed_parent = runtime
        .record_attempt(
            OperationKind::Command,
            None,
            None,
            2_910,
            OperationJournalAttempt::failed(2_920),
        )
        .expect("record retryable parent");
    let canceled_parent = runtime
        .record_attempt(
            OperationKind::Command,
            None,
            None,
            2_930,
            OperationJournalAttempt::canceled(2_940),
        )
        .expect("record ineligible terminal parent");
    runtime
        .begin_generation(generation(2), 2_950)
        .expect("start reconnect generation");
    runtime
        .flush(Duration::from_secs(5))
        .expect("persist generation boundary");

    let dispatch_count = Arc::new(AtomicUsize::new(0));
    let stale_dispatch_count = dispatch_count.clone();
    assert_eq!(
        runtime.execute_guarded_retry(
            generation(1),
            &failed_parent,
            SensitiveOperationPayload::opaque(b"stale retry".to_vec()).redact(),
            2_960,
            2_970,
            move || {
                stale_dispatch_count.fetch_add(1, Ordering::AcqRel);
                true
            },
            Duration::from_secs(5),
        ),
        Err(OperationJournalRuntimeError::RetryGenerationChanged {
            expected_generation_id: generation(1),
            current_generation_id: generation(2),
        })
    );

    let ineligible_dispatch_count = dispatch_count.clone();
    assert_eq!(
        runtime.execute_guarded_retry(
            generation(2),
            &canceled_parent,
            SensitiveOperationPayload::opaque(b"canceled retry".to_vec()).redact(),
            2_980,
            2_990,
            move || {
                ineligible_dispatch_count.fetch_add(1, Ordering::AcqRel);
                true
            },
            Duration::from_secs(5),
        ),
        Err(OperationJournalRuntimeError::RetryParentNotEligible {
            parent_operation_id: canceled_parent,
            kind: OperationKind::Command,
            status: OperationStatus::Canceled,
        })
    );
    assert_eq!(dispatch_count.load(Ordering::Acquire), 0);

    runtime
        .shutdown(Duration::from_secs(5))
        .expect("rejected retries leave runtime usable");
}

#[test]
fn guarded_retry_links_an_old_generation_parent_to_the_current_generation() {
    let directory = tempfile::tempdir().expect("temp directory");
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        OperationJournalSessionId::from_string(
            "terminal_session_live_runtime_guarded_retry_old_generation",
        ),
        OperationJournalScope::local(),
        generation(1),
        3_000,
    )
    .expect("spawn journal runtime");
    let parent_operation_id = runtime
        .record_attempt(
            OperationKind::Paste,
            None,
            None,
            3_010,
            OperationJournalAttempt::failed(3_020),
        )
        .expect("record failed old-generation parent");
    runtime
        .begin_generation(generation(2), 3_030)
        .expect("start current generation");

    let child_operation_id = runtime
        .execute_guarded_retry(
            generation(2),
            &parent_operation_id,
            SensitiveOperationPayload::opaque(b"new command only".to_vec()).redact(),
            3_040,
            3_050,
            || true,
            Duration::from_secs(5),
        )
        .expect("retry old-generation parent in current generation");
    let journal = runtime
        .journal_snapshot(Duration::from_secs(5))
        .expect("snapshot retry lineage");
    assert_eq!(
        journal
            .operation(&parent_operation_id)
            .expect("old parent")
            .status(),
        OperationStatus::Failed
    );
    let child = journal.operation(&child_operation_id).expect("retry child");
    assert_eq!(child.generation_id(), generation(2));
    assert_eq!(child.parent_operation_id(), Some(&parent_operation_id));
    assert_eq!(child.kind(), OperationKind::Command);
    assert_eq!(child.status(), OperationStatus::Sent);

    runtime
        .shutdown(Duration::from_secs(5))
        .expect("stop journal runtime");
}

#[test]
fn guarded_retry_dispatch_failure_records_a_failed_child_and_keeps_worker_usable() {
    let directory = tempfile::tempdir().expect("temp directory");
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        OperationJournalSessionId::from_string(
            "terminal_session_live_runtime_guarded_retry_dispatch_failure",
        ),
        OperationJournalScope::local(),
        generation(1),
        3_100,
    )
    .expect("spawn journal runtime");
    let parent_operation_id = runtime
        .record_attempt(
            OperationKind::Command,
            None,
            None,
            3_110,
            OperationJournalAttempt::failed(3_120),
        )
        .expect("record failed parent");
    runtime
        .flush(Duration::from_secs(5))
        .expect("persist failed parent");

    let error = runtime
        .execute_guarded_retry(
            generation(1),
            &parent_operation_id,
            SensitiveOperationPayload::opaque(b"manual retry".to_vec()).redact(),
            3_130,
            3_140,
            || false,
            Duration::from_secs(5),
        )
        .expect_err("failed backend dispatch must be explicit");
    let OperationJournalRuntimeError::RetryDispatchFailed {
        operation_id: child_operation_id,
    } = error
    else {
        panic!("unexpected retry error: {error}");
    };

    let journal = runtime
        .journal_snapshot(Duration::from_secs(5))
        .expect("worker remains available after dispatch failure");
    let child = journal
        .operation(&child_operation_id)
        .expect("failed retry child");
    assert_eq!(child.parent_operation_id(), Some(&parent_operation_id));
    assert_eq!(child.status(), OperationStatus::Failed);
    assert_eq!(
        child
            .transitions()
            .iter()
            .map(|transition| transition.status())
            .collect::<Vec<_>>(),
        vec![OperationStatus::Queued, OperationStatus::Failed]
    );

    runtime
        .shutdown(Duration::from_secs(5))
        .expect("stop journal runtime");
}

#[test]
fn guarded_retry_contains_dispatch_panics_as_failed_attempts() {
    let directory = tempfile::tempdir().expect("temp directory");
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        OperationJournalSessionId::from_string(
            "terminal_session_live_runtime_guarded_retry_dispatch_panic",
        ),
        OperationJournalScope::local(),
        generation(1),
        3_200,
    )
    .expect("spawn journal runtime");
    let parent_operation_id = runtime
        .record_attempt(
            OperationKind::Paste,
            None,
            None,
            3_210,
            OperationJournalAttempt::failed(3_220),
        )
        .expect("record failed parent");
    runtime
        .flush(Duration::from_secs(5))
        .expect("persist failed parent");

    assert!(matches!(
        runtime.execute_guarded_retry(
            generation(1),
            &parent_operation_id,
            SensitiveOperationPayload::opaque(b"panic retry".to_vec()).redact(),
            3_230,
            3_240,
            || panic!("backend retry dispatch panicked"),
            Duration::from_secs(5),
        ),
        Err(OperationJournalRuntimeError::RetryDispatchFailed { .. })
    ));
    assert_eq!(
        runtime.snapshot().health,
        OperationJournalRuntimeHealth::Healthy
    );

    runtime
        .shutdown(Duration::from_secs(5))
        .expect("dispatch panic must not stop the journal worker");
}

#[test]
fn guarded_retry_first_persistence_failure_never_dispatches() {
    let directory = tempfile::tempdir().expect("temp directory");
    let session_id = OperationJournalSessionId::from_string(
        "terminal_session_live_runtime_guarded_retry_persistence_failure",
    );
    let paths = OperationJournalPersistencePaths::for_session(directory.path(), &session_id);
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        session_id,
        OperationJournalScope::local(),
        generation(1),
        3_300,
    )
    .expect("spawn journal runtime");
    let parent_operation_id = runtime
        .record_attempt(
            OperationKind::Command,
            None,
            None,
            3_310,
            OperationJournalAttempt::failed(3_320),
        )
        .expect("record failed parent");
    runtime
        .flush(Duration::from_secs(5))
        .expect("persist failed parent");

    fs::remove_file(paths.append_log_path()).expect("remove live append log");
    fs::create_dir(paths.append_log_path()).expect("replace append log with a directory");
    let dispatch_count = Arc::new(AtomicUsize::new(0));
    let dispatch_count_for_retry = dispatch_count.clone();
    assert!(matches!(
        runtime.execute_guarded_retry(
            generation(1),
            &parent_operation_id,
            SensitiveOperationPayload::opaque(b"must remain local".to_vec()).redact(),
            3_330,
            3_340,
            move || {
                dispatch_count_for_retry.fetch_add(1, Ordering::AcqRel);
                true
            },
            Duration::from_secs(5),
        ),
        Err(OperationJournalRuntimeError::PersistenceFailed(_))
    ));
    assert_eq!(
        dispatch_count.load(Ordering::Acquire),
        0,
        "the backend must not receive a retry whose lineage was not durably published"
    );
    assert_eq!(
        runtime.snapshot().health,
        OperationJournalRuntimeHealth::PersistenceFailed
    );
}

#[test]
fn guarded_retry_second_persistence_failure_reports_unknown_completion() {
    let directory = tempfile::tempdir().expect("temp directory");
    let session_id = OperationJournalSessionId::from_string(
        "terminal_session_live_runtime_guarded_retry_post_dispatch_persistence_failure",
    );
    let paths = OperationJournalPersistencePaths::for_session(directory.path(), &session_id);
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        session_id,
        OperationJournalScope::local(),
        generation(1),
        3_350,
    )
    .expect("spawn journal runtime");
    let parent_operation_id = runtime
        .record_attempt(
            OperationKind::Command,
            None,
            None,
            3_360,
            OperationJournalAttempt::failed(3_370),
        )
        .expect("record failed parent");
    runtime
        .flush(Duration::from_secs(5))
        .expect("persist failed parent");

    let dispatch_count = Arc::new(AtomicUsize::new(0));
    let dispatch_count_for_retry = dispatch_count.clone();
    let result = runtime.execute_guarded_retry(
        generation(1),
        &parent_operation_id,
        SensitiveOperationPayload::opaque(b"manual replacement command".to_vec()).redact(),
        3_380,
        3_390,
        move || {
            dispatch_count_for_retry.fetch_add(1, Ordering::AcqRel);
            fs::remove_file(paths.append_log_path()).expect("remove live append log");
            fs::create_dir(paths.append_log_path())
                .expect("replace append log with a directory after dispatch");
            true
        },
        Duration::from_secs(5),
    );

    assert!(matches!(
        result,
        Err(OperationJournalRuntimeError::RetryCompletionUnknown { .. })
    ));
    assert_eq!(
        dispatch_count.load(Ordering::Acquire),
        1,
        "the caller must be told completion is unknown once dispatch has begun"
    );
    assert_eq!(
        runtime.snapshot().health,
        OperationJournalRuntimeHealth::PersistenceFailed
    );
}

#[test]
fn guarded_retry_after_shutdown_never_dispatches() {
    let directory = tempfile::tempdir().expect("temp directory");
    let runtime = OperationJournalRuntime::new(
        runtime_config(&directory),
        OperationJournalSessionId::from_string(
            "terminal_session_live_runtime_guarded_retry_closed",
        ),
        OperationJournalScope::local(),
        generation(1),
        3_400,
    )
    .expect("spawn journal runtime");
    runtime
        .shutdown(Duration::from_secs(5))
        .expect("stop journal runtime");

    let dispatch_count = Arc::new(AtomicUsize::new(0));
    let dispatch_count_for_retry = dispatch_count.clone();
    assert_eq!(
        runtime.execute_guarded_retry(
            generation(1),
            &OperationId::from_string("terminal_operation_closed_parent"),
            SensitiveOperationPayload::opaque(b"closed runtime".to_vec()).redact(),
            3_410,
            3_420,
            move || {
                dispatch_count_for_retry.fetch_add(1, Ordering::AcqRel);
                true
            },
            Duration::from_secs(5),
        ),
        Err(OperationJournalRuntimeError::Closed)
    );
    assert_eq!(dispatch_count.load(Ordering::Acquire), 0);
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
