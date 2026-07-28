use super::{
    OperationGenerationId, OperationJournal, OperationJournalFileStore,
    OperationJournalPersistOutcome, OperationJournalPersistenceConfig,
    OperationJournalPersistenceCorruption, OperationJournalPersistenceError,
    OperationJournalPersistencePaths, OperationJournalRecoverySource, OperationJournalSessionId,
    OperationKind, OperationStatus, SensitiveOperationPayload,
};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::symlink;

fn generation(value: u64) -> OperationGenerationId {
    OperationGenerationId::new(value).expect("generation must be non-zero")
}

fn journal() -> OperationJournal {
    OperationJournal::new(
        OperationJournalSessionId::from_string("terminal_session_persistence_test"),
        generation(1),
        1_000,
    )
}

fn paths(
    directory: &tempfile::TempDir,
    journal: &OperationJournal,
) -> OperationJournalPersistencePaths {
    OperationJournalPersistencePaths::for_session(directory.path(), journal.session_id())
}

fn small_config() -> OperationJournalPersistenceConfig {
    OperationJournalPersistenceConfig {
        max_log_entries: 2,
        max_entry_bytes: 64 * 1024,
        max_log_bytes: 128 * 1024,
        max_checkpoint_bytes: 64 * 1024,
    }
}

fn compact_after_one_entry_config() -> OperationJournalPersistenceConfig {
    OperationJournalPersistenceConfig {
        max_log_entries: 1,
        ..small_config()
    }
}

#[test]
fn unreasonably_large_limits_are_rejected_before_opening_journal_paths() {
    let cases = [
        (
            "log entries",
            OperationJournalPersistenceConfig {
                max_log_entries: 257,
                ..OperationJournalPersistenceConfig::default()
            },
            "max_log_entries exceeds the hard limit",
        ),
        (
            "entry bytes",
            OperationJournalPersistenceConfig {
                max_entry_bytes: 4 * 1024 * 1024 + 1,
                ..OperationJournalPersistenceConfig::default()
            },
            "max_entry_bytes exceeds the hard limit",
        ),
        (
            "log bytes",
            OperationJournalPersistenceConfig {
                max_log_bytes: 32 * 1024 * 1024 + 1,
                ..OperationJournalPersistenceConfig::default()
            },
            "max_log_bytes exceeds the hard limit",
        ),
        (
            "checkpoint bytes",
            OperationJournalPersistenceConfig {
                max_checkpoint_bytes: 4 * 1024 * 1024 + 1,
                ..OperationJournalPersistenceConfig::default()
            },
            "max_checkpoint_bytes exceeds the hard limit",
        ),
    ];

    for (case, config, expected_reason) in cases {
        let directory = tempfile::tempdir().expect("temp directory");
        let journal = journal();
        let persistence_paths = paths(&directory, &journal);
        fs::create_dir(persistence_paths.append_log_path())
            .expect("create a path that would fail if opened");

        let error = OperationJournalFileStore::open(persistence_paths, config)
            .expect_err("hard limits must reject unsafe allocation bounds");
        assert!(
            matches!(
                error,
                OperationJournalPersistenceError::InvalidConfig { reason }
                    if reason == expected_reason
            ),
            "unexpected error for {case}: {error}"
        );
    }
}

#[test]
fn append_log_roundtrips_the_latest_validated_snapshot_without_raw_secrets() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let payload = SensitiveOperationPayload::opaque(b"password=plain-secret".to_vec()).redact();
    let operation_id = journal
        .queue_operation_with_payload(OperationKind::Command, None, payload, 1_010)
        .expect("queue operation");
    let persistence_paths = paths(&directory, &journal);

    let (mut store, initial) = OperationJournalFileStore::open(
        persistence_paths.clone(),
        OperationJournalPersistenceConfig::default(),
    )
    .expect("open empty journal store");
    assert!(initial.journal().is_none());

    assert_eq!(
        store.persist(&journal).expect("persist queued operation"),
        OperationJournalPersistOutcome::Appended
    );
    journal
        .transition_operation(&operation_id, OperationStatus::Sent, 1_020)
        .expect("mark sent");
    store.persist(&journal).expect("persist sent operation");
    drop(store);

    let on_disk =
        fs::read_to_string(persistence_paths.append_log_path()).expect("read append journal");
    assert!(!on_disk.contains("plain-secret"));
    assert!(on_disk.contains("opaque_summary"));

    let (_, recovered) = OperationJournalFileStore::open(
        persistence_paths,
        OperationJournalPersistenceConfig::default(),
    )
    .expect("reopen journal store");
    assert_eq!(
        recovered.source(),
        Some(OperationJournalRecoverySource::AppendLog)
    );
    assert_eq!(recovered.journal(), Some(&journal));
    assert_eq!(recovered.discarded_log_tail_bytes(), 0);
}

#[test]
fn bounded_log_compacts_to_an_atomic_checkpoint_and_stays_within_limits() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = small_config();
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
        .expect("open journal store");

    let operation_id = journal
        .queue_operation(OperationKind::ApplicationOperation, None, 1_010)
        .expect("queue operation");
    store.persist(&journal).expect("persist queued");
    journal
        .transition_operation(&operation_id, OperationStatus::Sent, 1_020)
        .expect("mark sent");
    store.persist(&journal).expect("persist sent");
    journal
        .transition_operation(&operation_id, OperationStatus::Succeeded, 1_030)
        .expect("mark succeeded");

    assert_eq!(
        store.persist(&journal).expect("compact bounded log"),
        OperationJournalPersistOutcome::Compacted
    );
    assert!(store.log_entry_count() <= config.max_log_entries);
    assert!(store.log_bytes() <= config.max_log_bytes);
    assert!(
        fs::metadata(persistence_paths.checkpoint_path())
            .expect("checkpoint metadata")
            .len()
            <= config.max_checkpoint_bytes
    );
    assert!(
        fs::metadata(persistence_paths.append_log_path())
            .expect("append log metadata")
            .len()
            <= config.max_log_bytes
    );

    drop(store);
    let (_, recovered) = OperationJournalFileStore::open(persistence_paths, config)
        .expect("recover compacted journal");
    assert_eq!(recovered.journal(), Some(&journal));
    assert_eq!(
        recovered.source(),
        Some(OperationJournalRecoverySource::CheckpointAndAppendLog)
    );
}

#[test]
fn truncated_tail_is_discarded_and_repaired_before_future_appends() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = OperationJournalPersistenceConfig::default();
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
        .expect("open journal store");

    journal
        .queue_operation(OperationKind::Paste, None, 1_010)
        .expect("queue operation");
    store.persist(&journal).expect("persist operation");
    let valid_len = fs::metadata(persistence_paths.append_log_path())
        .expect("append log metadata")
        .len();
    drop(store);

    let truncated_tail = br#"{"format":"navop_terminal_operation_journal_snapshot""#;
    OpenOptions::new()
        .append(true)
        .open(persistence_paths.append_log_path())
        .expect("open append log")
        .write_all(truncated_tail)
        .expect("append truncated tail");

    let (mut reopened, recovered) =
        OperationJournalFileStore::open(persistence_paths.clone(), config)
            .expect("recover truncated append tail");
    assert_eq!(recovered.journal(), Some(&journal));
    assert_eq!(
        recovered.discarded_log_tail_bytes(),
        truncated_tail.len() as u64
    );
    assert_eq!(
        fs::metadata(persistence_paths.append_log_path())
            .expect("repaired append log metadata")
            .len(),
        valid_len
    );

    journal
        .begin_generation(generation(2), 2_000)
        .expect("begin reconnect generation");
    reopened
        .persist(&journal)
        .expect("append after truncated tail repair");
}

#[cfg(unix)]
#[test]
fn append_log_symlink_is_rejected_without_reading_or_truncating_the_target() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = OperationJournalPersistenceConfig::default();
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
        .expect("open journal store");

    journal
        .queue_operation(OperationKind::Paste, None, 1_010)
        .expect("queue operation");
    store.persist(&journal).expect("persist operation");
    drop(store);

    let mut external_bytes =
        fs::read(persistence_paths.append_log_path()).expect("read valid append log");
    external_bytes.extend_from_slice(br#"{"format":"navop_terminal_operation_journal_snapshot""#);
    let external_path = directory.path().join("external-append-target");
    fs::write(&external_path, &external_bytes).expect("write external append target");
    fs::remove_file(persistence_paths.append_log_path()).expect("remove real append log");
    symlink(&external_path, persistence_paths.append_log_path())
        .expect("replace append log with symlink");

    let result = OperationJournalFileStore::open(persistence_paths, config);

    assert_eq!(
        fs::read(&external_path).expect("read external append target after recovery"),
        external_bytes,
        "recovery must never repair or truncate a symlink target"
    );
    assert!(result.is_err(), "append log symlinks must be rejected");
}

#[cfg(unix)]
#[test]
fn checkpoint_symlink_is_rejected_without_recovering_the_target() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = compact_after_one_entry_config();
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
        .expect("open journal store");

    let operation_id = journal
        .queue_operation(OperationKind::ApplicationOperation, None, 1_010)
        .expect("queue operation");
    store.persist(&journal).expect("persist queued operation");
    journal
        .transition_operation(&operation_id, OperationStatus::Sent, 1_020)
        .expect("mark operation sent");
    store
        .persist(&journal)
        .expect("compact journal and publish checkpoint");
    drop(store);

    let external_bytes =
        fs::read(persistence_paths.checkpoint_path()).expect("read valid checkpoint");
    let external_path = directory.path().join("external-checkpoint-target");
    fs::write(&external_path, &external_bytes).expect("write external checkpoint target");
    fs::remove_file(persistence_paths.checkpoint_path()).expect("remove real checkpoint");
    fs::remove_file(persistence_paths.append_log_path()).expect("remove append log fallback");
    symlink(&external_path, persistence_paths.checkpoint_path())
        .expect("replace checkpoint with symlink");

    let result = OperationJournalFileStore::open(persistence_paths, config);

    assert_eq!(
        fs::read(&external_path).expect("read external checkpoint target after recovery"),
        external_bytes,
        "checkpoint recovery must never mutate a symlink target"
    );
    assert!(result.is_err(), "checkpoint symlinks must be rejected");
}

#[cfg(unix)]
#[test]
fn append_rejects_a_same_length_symlink_replacement_without_writing_the_target() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = OperationJournalPersistenceConfig::default();
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config)
        .expect("open journal store");

    journal
        .queue_operation(OperationKind::Command, None, 1_010)
        .expect("queue operation");
    store.persist(&journal).expect("persist initial operation");

    let external_bytes =
        fs::read(persistence_paths.append_log_path()).expect("read initial append log");
    let external_path = directory.path().join("external-same-length-target");
    fs::write(&external_path, &external_bytes).expect("write same-length external target");
    fs::remove_file(persistence_paths.append_log_path()).expect("remove real append log");
    symlink(&external_path, persistence_paths.append_log_path())
        .expect("replace append log with symlink");

    journal
        .begin_generation(generation(2), 2_000)
        .expect("begin reconnect generation");
    let result = store.persist(&journal);

    assert_eq!(
        fs::read(&external_path).expect("read external target after append"),
        external_bytes,
        "append must never follow a replacement symlink"
    );
    assert!(
        matches!(
            result,
            Err(OperationJournalPersistenceError::AppendLogChanged)
        ),
        "unexpected persist result after append log replacement: {result:?}"
    );
}

#[test]
fn corrupt_checkpoint_falls_back_to_the_complete_append_log() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = small_config();
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
        .expect("open journal store");

    let operation_id = journal
        .queue_operation(OperationKind::FileOperation, None, 1_010)
        .expect("queue operation");
    store.persist(&journal).expect("persist queued");
    journal
        .transition_operation(&operation_id, OperationStatus::Sent, 1_020)
        .expect("mark sent");
    store.persist(&journal).expect("persist sent");
    journal
        .transition_operation(&operation_id, OperationStatus::Succeeded, 1_030)
        .expect("mark succeeded");
    store.persist(&journal).expect("compact journal");
    drop(store);

    fs::write(
        persistence_paths.checkpoint_path(),
        b"{\"schema_version\":1,\"truncated\":",
    )
    .expect("corrupt checkpoint");

    let (_, recovered) = OperationJournalFileStore::open(persistence_paths, config)
        .expect("fall back to append log");
    assert!(recovered.checkpoint_was_rejected());
    assert_eq!(
        recovered.checkpoint_rejection(),
        Some(OperationJournalPersistenceCorruption::InvalidRecord)
    );
    assert_eq!(
        recovered.source(),
        Some(OperationJournalRecoverySource::AppendLog)
    );
    assert_eq!(recovered.journal(), Some(&journal));
}

#[test]
fn conflicting_checkpoint_at_the_same_sequence_is_rejected_in_favor_of_the_append_log() {
    let log_directory = tempfile::tempdir().expect("append log directory");
    let checkpoint_directory = tempfile::tempdir().expect("checkpoint directory");
    let config = compact_after_one_entry_config();

    let mut log_journal = journal();
    let log_paths = paths(&log_directory, &log_journal);
    let log_operation = log_journal
        .queue_operation(OperationKind::Command, None, 1_010)
        .expect("queue append log operation");
    let (mut log_store, _) = OperationJournalFileStore::open(log_paths.clone(), config.clone())
        .expect("open append log store");
    log_store
        .persist(&log_journal)
        .expect("persist first append log snapshot");
    log_journal
        .transition_operation(&log_operation, OperationStatus::Sent, 1_020)
        .expect("mark append log operation sent");
    assert_eq!(
        log_store
            .persist(&log_journal)
            .expect("compact append log snapshot"),
        OperationJournalPersistOutcome::Compacted
    );
    drop(log_store);

    let mut checkpoint_journal = journal();
    let checkpoint_paths = paths(&checkpoint_directory, &checkpoint_journal);
    let checkpoint_operation = checkpoint_journal
        .queue_operation(OperationKind::ApplicationOperation, None, 1_010)
        .expect("queue checkpoint operation");
    let (mut checkpoint_store, _) =
        OperationJournalFileStore::open(checkpoint_paths.clone(), config.clone())
            .expect("open checkpoint store");
    checkpoint_store
        .persist(&checkpoint_journal)
        .expect("persist first checkpoint snapshot");
    checkpoint_journal
        .transition_operation(&checkpoint_operation, OperationStatus::Failed, 1_020)
        .expect("mark checkpoint operation failed");
    checkpoint_store
        .persist(&checkpoint_journal)
        .expect("compact conflicting checkpoint snapshot");
    drop(checkpoint_store);

    fs::copy(
        checkpoint_paths.checkpoint_path(),
        log_paths.checkpoint_path(),
    )
    .expect("replace checkpoint with a valid conflicting snapshot");

    let (_, recovered) = OperationJournalFileStore::open(log_paths, config)
        .expect("recover append log instead of conflicting checkpoint");
    assert_eq!(recovered.journal(), Some(&log_journal));
    assert_eq!(
        recovered.source(),
        Some(OperationJournalRecoverySource::AppendLog)
    );
    assert_eq!(
        recovered.checkpoint_rejection(),
        Some(OperationJournalPersistenceCorruption::ConflictingSnapshot)
    );
}

#[test]
fn checkpoint_only_recovery_continues_with_a_self_contained_append_snapshot() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = compact_after_one_entry_config();
    let operation_id = journal
        .queue_operation(OperationKind::ApplicationOperation, None, 1_010)
        .expect("queue operation");
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
        .expect("open journal store");
    store.persist(&journal).expect("persist queued operation");
    journal
        .transition_operation(&operation_id, OperationStatus::Sent, 1_020)
        .expect("mark operation sent");
    store
        .persist(&journal)
        .expect("compact journal and publish checkpoint");
    drop(store);

    fs::remove_file(persistence_paths.append_log_path()).expect("remove compacted append log");
    let (mut checkpoint_store, recovered) =
        OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
            .expect("recover checkpoint without append log");
    assert_eq!(recovered.journal(), Some(&journal));
    assert_eq!(
        recovered.source(),
        Some(OperationJournalRecoverySource::Checkpoint)
    );

    journal
        .transition_operation(&operation_id, OperationStatus::Succeeded, 1_030)
        .expect("mark operation succeeded");
    assert_eq!(
        checkpoint_store
            .persist(&journal)
            .expect("persist after checkpoint-only recovery"),
        OperationJournalPersistOutcome::Appended
    );
    drop(checkpoint_store);

    fs::remove_file(persistence_paths.checkpoint_path()).expect("remove stale checkpoint");
    let (_, recovered) = OperationJournalFileStore::open(persistence_paths, config)
        .expect("recover the new self-contained append snapshot");
    assert_eq!(recovered.journal(), Some(&journal));
    assert_eq!(
        recovered.source(),
        Some(OperationJournalRecoverySource::AppendLog)
    );
}

#[test]
fn a_newer_checkpoint_forces_compaction_before_the_next_persist() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = small_config();
    let operation_id = journal
        .queue_operation(OperationKind::ApplicationOperation, None, 1_010)
        .expect("queue operation");
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
        .expect("open journal store");
    store.persist(&journal).expect("persist queued operation");
    journal
        .transition_operation(&operation_id, OperationStatus::Sent, 1_020)
        .expect("mark operation sent");
    store.persist(&journal).expect("persist sent operation");
    let stale_log =
        fs::read(persistence_paths.append_log_path()).expect("capture pre-checkpoint append log");
    journal
        .transition_operation(&operation_id, OperationStatus::Succeeded, 1_030)
        .expect("mark operation succeeded");
    store
        .persist(&journal)
        .expect("compact latest snapshot and publish checkpoint");
    drop(store);

    fs::write(persistence_paths.append_log_path(), stale_log)
        .expect("restore append log older than checkpoint");
    let (mut reopened, recovered) =
        OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
            .expect("recover newer checkpoint");
    assert_eq!(recovered.journal(), Some(&journal));
    assert_eq!(
        recovered.source(),
        Some(OperationJournalRecoverySource::CheckpointAndAppendLog)
    );

    journal
        .begin_generation(generation(2), 2_000)
        .expect("begin next reconnect generation");
    assert_eq!(
        reopened
            .persist(&journal)
            .expect("compact before persisting after a newer checkpoint"),
        OperationJournalPersistOutcome::Compacted
    );
    assert_eq!(reopened.log_entry_count(), 1);
    drop(reopened);

    let (_, recovered) = OperationJournalFileStore::open(persistence_paths, config)
        .expect("reopen the compacted log without a sequence gap");
    assert_eq!(recovered.journal(), Some(&journal));
}

#[test]
fn complete_middle_corruption_is_never_skipped_to_a_later_snapshot() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = OperationJournalPersistenceConfig::default();
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
        .expect("open journal store");

    let operation_id = journal
        .queue_operation(OperationKind::Command, None, 1_010)
        .expect("queue operation");
    store.persist(&journal).expect("persist queued");
    journal
        .transition_operation(&operation_id, OperationStatus::Sent, 1_020)
        .expect("mark sent");
    store.persist(&journal).expect("persist sent");
    drop(store);

    let original = fs::read(persistence_paths.append_log_path()).expect("read append log");
    let first_newline = original
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("first complete record");
    let mut corrupted = Vec::new();
    corrupted.extend_from_slice(&original[..=first_newline]);
    corrupted.extend_from_slice(b"{\"complete\":\"but invalid\"}\n");
    corrupted.extend_from_slice(&original[first_newline + 1..]);
    fs::write(persistence_paths.append_log_path(), corrupted).expect("write middle corruption");

    let error = OperationJournalFileStore::open(persistence_paths, config)
        .expect_err("complete corrupt records cannot be treated as an incomplete tail");
    assert!(matches!(
        error,
        OperationJournalPersistenceError::CorruptLogEntry {
            line_number: 2,
            corruption: OperationJournalPersistenceCorruption::InvalidRecord,
        }
    ));
}

#[test]
fn a_complete_sequence_gap_is_rejected_instead_of_skipping_history() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = OperationJournalPersistenceConfig::default();
    let operation_id = journal
        .queue_operation(OperationKind::Command, None, 1_010)
        .expect("queue operation");
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
        .expect("open journal store");
    store.persist(&journal).expect("persist queued operation");
    journal
        .transition_operation(&operation_id, OperationStatus::Sent, 1_020)
        .expect("mark operation sent");
    store.persist(&journal).expect("persist sent operation");
    journal
        .transition_operation(&operation_id, OperationStatus::Acknowledged, 1_030)
        .expect("mark operation acknowledged");
    store
        .persist(&journal)
        .expect("persist acknowledged operation");
    drop(store);

    let snapshots = fs::read(persistence_paths.append_log_path()).expect("read append log");
    let snapshots = snapshots
        .split_inclusive(|byte| *byte == b'\n')
        .collect::<Vec<_>>();
    assert_eq!(snapshots.len(), 3);
    let mut with_gap = Vec::new();
    with_gap.extend_from_slice(snapshots[0]);
    with_gap.extend_from_slice(snapshots[2]);
    fs::write(persistence_paths.append_log_path(), with_gap)
        .expect("remove the middle persistence sequence");

    let error = OperationJournalFileStore::open(persistence_paths, config)
        .expect_err("complete sequence gaps must be rejected");
    assert!(matches!(
        error,
        OperationJournalPersistenceError::CorruptLogEntry {
            line_number: 2,
            corruption: OperationJournalPersistenceCorruption::InvalidSequence,
        }
    ));
}

#[test]
fn checksum_rejects_valid_json_tampering() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = OperationJournalPersistenceConfig::default();
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
        .expect("open journal store");

    journal
        .queue_operation(OperationKind::Command, None, 1_010)
        .expect("queue operation");
    store.persist(&journal).expect("persist operation");
    drop(store);

    let original =
        fs::read_to_string(persistence_paths.append_log_path()).expect("read append log");
    let tampered = original.replacen("\"command\"", "\"user_input\"", 1);
    fs::write(persistence_paths.append_log_path(), tampered).expect("tamper append log");

    let error = OperationJournalFileStore::open(persistence_paths, config)
        .expect_err("valid JSON mutations require a valid checksum");
    assert!(matches!(
        error,
        OperationJournalPersistenceError::CorruptLogEntry {
            line_number: 1,
            corruption: OperationJournalPersistenceCorruption::ChecksumMismatch,
        }
    ));
}

#[test]
fn oversized_snapshots_are_rejected_before_growing_the_disk_log() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = OperationJournalPersistenceConfig {
        max_entry_bytes: 512,
        max_checkpoint_bytes: 512,
        max_log_bytes: 1_024,
        max_log_entries: 4,
    };
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
        .expect("open journal store");

    let payload =
        SensitiveOperationPayload::structured(json!({"safe": "x".repeat(2_000)})).redact();
    journal
        .queue_operation_with_payload(OperationKind::ApplicationOperation, None, payload, 1_010)
        .expect("queue large redacted payload");

    assert!(matches!(
        store
            .persist(&journal)
            .expect_err("oversized snapshots must not be written"),
        OperationJournalPersistenceError::EntryTooLarge { max_bytes: 512, .. }
    ));
    assert!(!persistence_paths.append_log_path().exists());
    assert!(!persistence_paths.checkpoint_path().exists());
}

#[test]
fn checkpoint_publish_failure_keeps_the_new_log_snapshot_recoverable() {
    let directory = tempfile::tempdir().expect("temp directory");
    let mut journal = journal();
    let persistence_paths = paths(&directory, &journal);
    let config = small_config();
    let (mut store, _) = OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
        .expect("open journal store");

    let operation_id = journal
        .queue_operation(OperationKind::ApplicationOperation, None, 1_010)
        .expect("queue operation");
    store.persist(&journal).expect("persist queued");
    journal
        .transition_operation(&operation_id, OperationStatus::Sent, 1_020)
        .expect("mark sent");
    store.persist(&journal).expect("persist sent");

    fs::create_dir(persistence_paths.checkpoint_path())
        .expect("block checkpoint publish with a directory");
    journal
        .transition_operation(&operation_id, OperationStatus::Failed, 1_030)
        .expect("mark failed");
    assert!(matches!(
        store
            .persist(&journal)
            .expect_err("checkpoint publish should report its failure"),
        OperationJournalPersistenceError::CheckpointWrite { .. }
    ));
    assert!(matches!(
        store
            .persist(&journal)
            .expect_err("checkpoint failure must disable further writes"),
        OperationJournalPersistenceError::WriteDisabledAfterFailure
    ));
    drop(store);

    fs::remove_dir(persistence_paths.checkpoint_path()).expect("remove blocking directory");
    let (_, recovered) = OperationJournalFileStore::open(persistence_paths, config)
        .expect("new compacted log remains recoverable");
    assert_eq!(recovered.journal(), Some(&journal));
    assert_eq!(
        recovered.source(),
        Some(OperationJournalRecoverySource::AppendLog)
    );
}

#[test]
fn recovered_unknown_and_needs_review_states_cannot_be_upgraded_to_success() {
    for terminal_status in [OperationStatus::Unknown, OperationStatus::NeedsReview] {
        let directory = tempfile::tempdir().expect("temp directory");
        let mut journal = journal();
        let persistence_paths = paths(&directory, &journal);
        let config = OperationJournalPersistenceConfig::default();
        let operation_id = journal
            .queue_operation(OperationKind::Unconfirmable, None, 1_010)
            .expect("queue operation");
        journal
            .transition_operation(&operation_id, terminal_status, 1_020)
            .expect("mark conservative terminal state");

        let (mut store, _) =
            OperationJournalFileStore::open(persistence_paths.clone(), config.clone())
                .expect("open journal store");
        store.persist(&journal).expect("persist journal");
        drop(store);

        let (_, recovered) =
            OperationJournalFileStore::open(persistence_paths, config).expect("recover journal");
        let mut recovered = recovered.into_journal().expect("recovered journal");
        assert_eq!(
            recovered
                .operation(&operation_id)
                .expect("operation")
                .status(),
            terminal_status
        );
        assert!(
            recovered
                .transition_operation(&operation_id, OperationStatus::Succeeded, 1_030)
                .is_err()
        );
    }
}
