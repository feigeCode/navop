use super::{
    OperationGenerationId, OperationJournal, OperationJournalFileStore,
    OperationJournalHistoryConfig, OperationJournalHistoryError, OperationJournalHistoryStore,
    OperationJournalPersistenceConfig, OperationJournalPersistenceCorruption,
    OperationJournalRecoverySource, OperationJournalRecoveryWarningKind, OperationJournalScope,
    OperationJournalScopeKind, OperationJournalSessionId, OperationJournalSessionManifest,
    OperationKind, OperationStatus,
};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;

fn generation(value: u64) -> OperationGenerationId {
    OperationGenerationId::new(value).expect("generation must be non-zero")
}

fn scope(connection_id: &str) -> OperationJournalScope {
    OperationJournalScope::ssh(connection_id).expect("valid SSH history scope")
}

fn store(
    directory: &tempfile::TempDir,
    config: OperationJournalHistoryConfig,
) -> OperationJournalHistoryStore {
    OperationJournalHistoryStore::new(directory.path(), config).expect("valid history store")
}

fn journal(session_id: &str, started_at_unix_ms: u64) -> OperationJournal {
    OperationJournal::new(
        OperationJournalSessionId::from_string(session_id),
        generation(1),
        started_at_unix_ms,
    )
}

fn persist_history(
    history_store: &OperationJournalHistoryStore,
    scope: &OperationJournalScope,
    journal: &OperationJournal,
    created_at_unix_ms: u64,
    updated_at_unix_ms: u64,
) {
    let paths = history_store.paths_for_session(journal.session_id());
    let (mut file_store, recovery) =
        OperationJournalFileStore::open(paths, OperationJournalPersistenceConfig::default())
            .expect("open journal file store");
    assert!(recovery.journal().is_none());
    file_store.persist(journal).expect("persist journal");

    let mut manifest = OperationJournalSessionManifest::new(
        journal.session_id().clone(),
        scope.clone(),
        created_at_unix_ms,
    )
    .expect("valid manifest");
    manifest
        .touch(updated_at_unix_ms)
        .expect("manifest timestamp advances");
    history_store
        .write_manifest(&manifest)
        .expect("persist manifest");
}

#[test]
fn journal_scopes_are_stable_validated_and_round_trip() {
    let local = OperationJournalScope::local();
    assert_eq!(local.kind(), OperationJournalScopeKind::Local);
    assert_eq!(local.connection_id(), None);
    assert_eq!(local.storage_key(), "local");

    let ssh = OperationJournalScope::ssh("ssh-connection-42").expect("valid SSH scope");
    assert_eq!(ssh.kind(), OperationJournalScopeKind::Ssh);
    assert_eq!(ssh.connection_id(), Some("ssh-connection-42"));
    assert_eq!(ssh.storage_key(), "ssh:ssh-connection-42");

    let serial = OperationJournalScope::serial("serial-connection-7").expect("valid serial scope");
    assert_eq!(serial.kind(), OperationJournalScopeKind::Serial);
    assert_eq!(serial.connection_id(), Some("serial-connection-7"));
    assert_eq!(serial.storage_key(), "serial:serial-connection-7");

    assert!(OperationJournalScope::ssh("").is_err());
    assert!(OperationJournalScope::serial("serial\nconnection").is_err());
    assert!(OperationJournalScope::ssh("x".repeat(257)).is_err());

    let encoded = serde_json::to_string(&ssh).expect("serialize scope");
    let decoded: OperationJournalScope = serde_json::from_str(&encoded).expect("deserialize scope");
    assert_eq!(decoded, ssh);
}

#[test]
fn session_paths_and_manifests_are_isolated_without_sensitive_connection_details() {
    let directory = tempfile::tempdir().expect("temp directory");
    let history_store = store(&directory, OperationJournalHistoryConfig::default());
    let first_id = OperationJournalSessionId::from_string("terminal_session_first");
    let second_id = OperationJournalSessionId::from_string("terminal_session_second");
    let first_paths = history_store.paths_for_session(&first_id);
    let second_paths = history_store.paths_for_session(&second_id);

    assert_ne!(
        first_paths.append_log_path(),
        second_paths.append_log_path()
    );
    assert_ne!(
        first_paths.checkpoint_path(),
        second_paths.checkpoint_path()
    );
    assert_ne!(
        first_paths.session_manifest_path(),
        second_paths.session_manifest_path()
    );

    let manifest =
        OperationJournalSessionManifest::new(first_id, scope("stored-connection-id"), 1_000)
            .expect("valid manifest");
    history_store
        .write_manifest(&manifest)
        .expect("persist manifest");

    let on_disk = fs::read_to_string(first_paths.session_manifest_path()).expect("read manifest");
    assert!(on_disk.contains("stored-connection-id"));
    assert!(!on_disk.contains("example.internal"));
    assert!(!on_disk.contains("username"));
    assert!(!on_disk.contains("password"));
    assert!(!on_disk.contains("secret"));

    let decoded: OperationJournalSessionManifest =
        serde_json::from_str(&on_disk).expect("decode manifest");
    assert_eq!(decoded, manifest);
}

#[test]
fn manifests_reject_unknown_fields_unsupported_schema_and_backwards_timestamps() {
    let manifest = OperationJournalSessionManifest::new(
        OperationJournalSessionId::from_string("terminal_session_manifest_validation"),
        scope("stored-connection-id"),
        1_000,
    )
    .expect("valid manifest");

    let mut unknown_manifest_field =
        serde_json::to_value(&manifest).expect("serialize manifest value");
    unknown_manifest_field
        .as_object_mut()
        .expect("manifest object")
        .insert("host".to_string(), json!("example.internal"));
    assert!(
        serde_json::from_value::<OperationJournalSessionManifest>(unknown_manifest_field).is_err()
    );

    let mut unknown_scope_field =
        serde_json::to_value(&manifest).expect("serialize manifest value");
    unknown_scope_field["scope"]
        .as_object_mut()
        .expect("scope object")
        .insert("password".to_string(), json!("secret"));
    assert!(
        serde_json::from_value::<OperationJournalSessionManifest>(unknown_scope_field).is_err()
    );

    let mut unsupported_schema = serde_json::to_value(&manifest).expect("serialize manifest value");
    unsupported_schema["schema_version"] = json!(2);
    assert!(serde_json::from_value::<OperationJournalSessionManifest>(unsupported_schema).is_err());

    let mut backwards_creation = serde_json::to_value(&manifest).expect("serialize manifest value");
    backwards_creation["updated_at_unix_ms"] = json!(999);
    assert!(serde_json::from_value::<OperationJournalSessionManifest>(backwards_creation).is_err());

    let mut touched = manifest;
    assert!(touched.touch(999).is_err());
    assert_eq!(touched.updated_at_unix_ms(), 1_000);
}

#[test]
fn invalid_history_configs_are_rejected_before_touching_the_root_path() {
    let directory = tempfile::tempdir().expect("temp directory");
    let root = directory.path().join("must-remain-missing");
    let cases = [
        OperationJournalHistoryConfig {
            max_directory_entries: 0,
            ..OperationJournalHistoryConfig::default()
        },
        OperationJournalHistoryConfig {
            max_manifest_bytes: u64::MAX,
            ..OperationJournalHistoryConfig::default()
        },
        OperationJournalHistoryConfig {
            max_history_sessions: 0,
            ..OperationJournalHistoryConfig::default()
        },
        OperationJournalHistoryConfig {
            persistence: OperationJournalPersistenceConfig {
                max_log_entries: 0,
                ..OperationJournalPersistenceConfig::default()
            },
            ..OperationJournalHistoryConfig::default()
        },
    ];

    for config in cases {
        let error = OperationJournalHistoryStore::new(&root, config)
            .expect_err("invalid config must be rejected");
        assert!(matches!(
            error,
            OperationJournalHistoryError::InvalidConfig { .. }
                | OperationJournalHistoryError::InvalidPersistenceConfig { .. }
        ));
        assert!(!root.exists());
    }
}

#[test]
fn missing_history_root_is_an_empty_non_mutating_discovery() {
    let directory = tempfile::tempdir().expect("temp directory");
    let root = directory.path().join("missing-history-root");
    let history_store =
        OperationJournalHistoryStore::new(&root, OperationJournalHistoryConfig::default())
            .expect("valid history store");

    let discovery = history_store.discover(&scope("connection-a"), &[]);
    assert!(discovery.histories().is_empty());
    assert!(discovery.warnings().is_empty());
    assert!(!root.exists());
}

#[test]
fn manifest_filename_must_match_its_session_id_hash() {
    let directory = tempfile::tempdir().expect("temp directory");
    let history_store = store(&directory, OperationJournalHistoryConfig::default());
    let history_scope = scope("connection-a");
    let manifest_session_id =
        OperationJournalSessionId::from_string("terminal_session_manifest_contents");
    let filename_session_id =
        OperationJournalSessionId::from_string("terminal_session_manifest_filename");
    let manifest = OperationJournalSessionManifest::new(
        manifest_session_id.clone(),
        history_scope.clone(),
        1_000,
    )
    .expect("valid manifest");

    fs::create_dir_all(directory.path()).expect("create history root");
    fs::write(
        history_store
            .paths_for_session(&filename_session_id)
            .session_manifest_path(),
        serde_json::to_vec(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest under a mismatched filename");

    let discovery = history_store.discover(&history_scope, &[]);
    assert!(discovery.histories().is_empty());
    assert!(discovery.warnings().iter().any(|warning| {
        warning.kind() == OperationJournalRecoveryWarningKind::InvalidManifest
            && warning.session_id() == Some(&manifest_session_id)
    }));
}

#[test]
fn restart_discovery_is_scope_isolated_excludes_live_sessions_and_orders_deterministically() {
    let directory = tempfile::tempdir().expect("temp directory");
    let history_store = store(&directory, OperationJournalHistoryConfig::default());
    let first_scope = scope("connection-a");
    let second_scope = scope("connection-b");

    let old = journal("terminal_session_old", 1_000);
    let tied_a = journal("terminal_session_tied_a", 2_000);
    let tied_b = journal("terminal_session_tied_b", 2_000);
    let live = journal("terminal_session_live", 3_000);
    let other_scope = journal("terminal_session_other_scope", 4_000);

    persist_history(&history_store, &first_scope, &old, 1_000, 1_100);
    persist_history(&history_store, &first_scope, &tied_b, 2_000, 2_100);
    persist_history(&history_store, &first_scope, &tied_a, 2_000, 2_100);
    persist_history(&history_store, &first_scope, &live, 3_000, 3_100);
    persist_history(&history_store, &second_scope, &other_scope, 4_000, 4_100);

    let discovery = history_store.discover(&first_scope, &[live.session_id().clone()]);
    let session_ids = discovery
        .histories()
        .iter()
        .map(|history| history.session_id().as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        session_ids,
        vec![
            "terminal_session_tied_a",
            "terminal_session_tied_b",
            "terminal_session_old",
        ]
    );
    assert!(discovery.warnings().is_empty());

    let other_discovery = history_store.discover(&second_scope, &[]);
    assert_eq!(other_discovery.histories().len(), 1);
    assert_eq!(
        other_discovery.histories()[0].session_id().as_str(),
        "terminal_session_other_scope"
    );
}

#[test]
fn corrupt_oversized_and_missing_history_is_skipped_without_blocking_valid_recovery() {
    let directory = tempfile::tempdir().expect("temp directory");
    let config = OperationJournalHistoryConfig {
        max_manifest_bytes: 512,
        ..OperationJournalHistoryConfig::default()
    };
    let history_store = store(&directory, config);
    let history_scope = scope("connection-a");

    let valid = journal("terminal_session_valid", 1_000);
    persist_history(&history_store, &history_scope, &valid, 1_000, 1_100);

    let corrupt_id = OperationJournalSessionId::from_string("terminal_session_corrupt");
    fs::create_dir_all(directory.path()).expect("create history directory");
    fs::write(
        history_store
            .paths_for_session(&corrupt_id)
            .session_manifest_path(),
        b"{not-json",
    )
    .expect("write corrupt manifest");

    let oversized_id = OperationJournalSessionId::from_string("terminal_session_oversized");
    fs::write(
        history_store
            .paths_for_session(&oversized_id)
            .session_manifest_path(),
        vec![b'x'; 513],
    )
    .expect("write oversized manifest");

    let missing = journal("terminal_session_missing_journal", 2_000);
    let missing_manifest = OperationJournalSessionManifest::new(
        missing.session_id().clone(),
        history_scope.clone(),
        2_000,
    )
    .expect("valid missing-journal manifest");
    history_store
        .write_manifest(&missing_manifest)
        .expect("persist missing-journal manifest");

    let broken_journal = journal("terminal_session_broken_journal", 3_000);
    let broken_manifest = OperationJournalSessionManifest::new(
        broken_journal.session_id().clone(),
        history_scope.clone(),
        3_000,
    )
    .expect("valid broken-journal manifest");
    history_store
        .write_manifest(&broken_manifest)
        .expect("persist broken-journal manifest");
    fs::write(
        history_store
            .paths_for_session(broken_journal.session_id())
            .append_log_path(),
        b"{not-a-valid-snapshot}\n",
    )
    .expect("write corrupt append log");

    let discovery = history_store.discover(&history_scope, &[]);
    assert_eq!(discovery.histories().len(), 1);
    assert_eq!(discovery.histories()[0].session_id(), valid.session_id());

    let warning_kinds = discovery
        .warnings()
        .iter()
        .map(|warning| warning.kind())
        .collect::<Vec<_>>();
    assert!(warning_kinds.contains(&OperationJournalRecoveryWarningKind::InvalidManifest));
    assert!(warning_kinds.contains(&OperationJournalRecoveryWarningKind::ManifestTooLarge));
    assert!(warning_kinds.contains(&OperationJournalRecoveryWarningKind::JournalMissing));
    assert!(warning_kinds.contains(&OperationJournalRecoveryWarningKind::JournalRecoveryFailed));
}

#[test]
fn journal_session_mismatch_is_reported_without_exposing_the_foreign_snapshot() {
    let directory = tempfile::tempdir().expect("temp directory");
    let history_store = store(&directory, OperationJournalHistoryConfig::default());
    let history_scope = scope("connection-a");
    let expected = journal("terminal_session_expected", 1_000);
    let foreign = journal("terminal_session_foreign", 1_000);

    let foreign_paths = history_store.paths_for_session(foreign.session_id());
    let (mut foreign_store, recovery) = OperationJournalFileStore::open(
        foreign_paths.clone(),
        OperationJournalPersistenceConfig::default(),
    )
    .expect("open foreign journal store");
    assert!(recovery.journal().is_none());
    foreign_store
        .persist(&foreign)
        .expect("persist foreign journal");
    drop(foreign_store);

    let manifest = OperationJournalSessionManifest::new(
        expected.session_id().clone(),
        history_scope.clone(),
        1_000,
    )
    .expect("valid expected manifest");
    history_store
        .write_manifest(&manifest)
        .expect("persist expected manifest");
    fs::copy(
        foreign_paths.append_log_path(),
        history_store
            .paths_for_session(expected.session_id())
            .append_log_path(),
    )
    .expect("place the foreign journal behind the expected manifest");

    let discovery = history_store.discover(&history_scope, &[]);
    assert!(discovery.histories().is_empty());
    assert!(discovery.warnings().iter().any(|warning| {
        warning.kind() == OperationJournalRecoveryWarningKind::JournalSessionMismatch
            && warning.session_id() == Some(expected.session_id())
    }));
    assert!(!discovery.warnings().iter().any(|warning| {
        warning.kind() == OperationJournalRecoveryWarningKind::JournalRecoveryFailed
            && warning.session_id() == Some(expected.session_id())
    }));
}

#[test]
fn checkpoint_rejection_and_truncated_tail_are_visible_on_recovered_history() {
    let directory = tempfile::tempdir().expect("temp directory");
    let history_store = store(&directory, OperationJournalHistoryConfig::default());
    let history_scope = scope("connection-a");
    let session = journal("terminal_session_partial_recovery", 1_000);
    persist_history(&history_store, &history_scope, &session, 1_000, 1_100);
    let paths = history_store.paths_for_session(session.session_id());

    fs::write(paths.checkpoint_path(), b"{\"truncated\":").expect("write rejected checkpoint");
    let truncated_tail = b"{\"incomplete\":\"append-tail\"";
    OpenOptions::new()
        .append(true)
        .open(paths.append_log_path())
        .expect("open append journal")
        .write_all(truncated_tail)
        .expect("append truncated tail");
    let append_log_before_discovery =
        fs::read(paths.append_log_path()).expect("read append log before discovery");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(paths.append_log_path(), fs::Permissions::from_mode(0o400))
            .expect("make append log read-only");
    }

    let discovery = history_store.discover(&history_scope, &[]);
    assert_eq!(discovery.histories().len(), 1);
    let recovered = &discovery.histories()[0];
    assert_eq!(recovered.session_id(), session.session_id());
    assert_eq!(
        recovered.recovery_source(),
        Some(OperationJournalRecoverySource::AppendLog)
    );
    assert_eq!(
        recovered.checkpoint_rejection(),
        Some(OperationJournalPersistenceCorruption::InvalidRecord)
    );
    assert_eq!(
        recovered.discarded_log_tail_bytes(),
        truncated_tail.len() as u64
    );
    assert!(discovery.warnings().iter().any(|warning| {
        warning.kind() == OperationJournalRecoveryWarningKind::CheckpointRejected
            && warning.session_id() == Some(session.session_id())
    }));
    assert!(discovery.warnings().iter().any(|warning| {
        warning.kind() == OperationJournalRecoveryWarningKind::TruncatedLogTailRecovered
            && warning.session_id() == Some(session.session_id())
    }));
    assert_eq!(
        fs::read(paths.append_log_path()).expect("read append log after discovery"),
        append_log_before_discovery,
        "read-only history discovery must not repair or otherwise mutate the live journal files"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(paths.append_log_path(), fs::Permissions::from_mode(0o600))
            .expect("restore append log permissions");
    }
}

#[test]
fn discovery_bounds_directory_work_and_history_results() {
    let directory = tempfile::tempdir().expect("temp directory");
    let config = OperationJournalHistoryConfig {
        max_directory_entries: 6,
        max_history_sessions: 2,
        ..OperationJournalHistoryConfig::default()
    };
    let history_store = store(&directory, config);
    let history_scope = scope("connection-a");

    for index in 0..3 {
        let session = journal(&format!("terminal_session_{index}"), 1_000 + index * 100);
        persist_history(
            &history_store,
            &history_scope,
            &session,
            1_000 + index * 100,
            1_000 + index * 100,
        );
    }

    let discovery = history_store.discover(&history_scope, &[]);
    assert_eq!(discovery.histories().len(), 2);
    assert_eq!(
        discovery.histories()[0].session_id().as_str(),
        "terminal_session_2"
    );
    assert_eq!(
        discovery.histories()[1].session_id().as_str(),
        "terminal_session_1"
    );
    assert!(
        discovery
            .warnings()
            .iter()
            .any(|warning| warning.kind()
                == OperationJournalRecoveryWarningKind::HistoryLimitReached)
    );
    assert!(!discovery.warnings().iter().any(|warning| {
        warning.kind() == OperationJournalRecoveryWarningKind::DirectoryScanLimitReached
    }));

    let capped_store = store(
        &directory,
        OperationJournalHistoryConfig {
            max_directory_entries: 1,
            max_history_sessions: 1,
            ..OperationJournalHistoryConfig::default()
        },
    );
    let capped = capped_store.discover(&history_scope, &[]);
    assert!(capped.histories().len() <= 1);
    assert!(
        capped.warnings().iter().any(|warning| warning.kind()
            == OperationJournalRecoveryWarningKind::DirectoryScanLimitReached)
    );
}

#[test]
fn over_limit_directories_are_rejected_instead_of_recovering_an_arbitrary_subset() {
    let directory = tempfile::tempdir().expect("temp directory");
    let history_scope = scope("connection-a");
    let writer = store(&directory, OperationJournalHistoryConfig::default());

    for index in 0..20 {
        let session = journal(
            &format!("terminal_session_directory_overflow_{index:02}"),
            1_000 + index * 100,
        );
        persist_history(
            &writer,
            &history_scope,
            &session,
            1_000 + index * 100,
            1_000 + index * 100,
        );
    }

    let bounded = store(
        &directory,
        OperationJournalHistoryConfig {
            max_directory_entries: 20,
            ..OperationJournalHistoryConfig::default()
        },
    );
    let discovery = bounded.discover(&history_scope, &[]);

    assert!(
        discovery.histories().is_empty(),
        "an over-limit directory must fail closed instead of exposing whichever manifests the OS returned first"
    );
    assert_eq!(
        discovery
            .warnings()
            .iter()
            .filter(|warning| {
                warning.kind() == OperationJournalRecoveryWarningKind::DirectoryScanLimitReached
            })
            .count(),
        1
    );
}

#[test]
fn recovered_history_snapshots_remain_owned_when_the_live_journal_changes() {
    let directory = tempfile::tempdir().expect("temp directory");
    let history_store = store(&directory, OperationJournalHistoryConfig::default());
    let history_scope = scope("connection-a");
    let mut live_journal = journal("terminal_session_snapshot", 1_000);
    let operation_id = live_journal
        .queue_operation(OperationKind::UserInput, None, 1_010)
        .expect("queue operation");
    persist_history(&history_store, &history_scope, &live_journal, 1_000, 1_010);

    let first_snapshot = history_store.discover(&history_scope, &[]).histories()[0].clone();
    assert_eq!(
        first_snapshot
            .journal()
            .operation(&operation_id)
            .expect("snapshot operation")
            .status(),
        OperationStatus::Queued
    );

    live_journal
        .transition_operation(&operation_id, OperationStatus::Sent, 1_020)
        .expect("mark operation sent");
    let paths = history_store.paths_for_session(live_journal.session_id());
    let (mut file_store, _) =
        OperationJournalFileStore::open(paths, OperationJournalPersistenceConfig::default())
            .expect("reopen journal store");
    file_store
        .persist(&live_journal)
        .expect("persist updated journal");

    assert_eq!(
        first_snapshot
            .journal()
            .operation(&operation_id)
            .expect("owned snapshot operation")
            .status(),
        OperationStatus::Queued
    );
    let refreshed = history_store.discover(&history_scope, &[]);
    assert_eq!(
        refreshed.histories()[0]
            .journal()
            .operation(&operation_id)
            .expect("refreshed operation")
            .status(),
        OperationStatus::Sent
    );
}

#[cfg(unix)]
#[test]
fn symlinked_manifests_are_never_followed() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temp directory");
    let history_store = store(&directory, OperationJournalHistoryConfig::default());
    let history_scope = scope("connection-a");
    let session_id = OperationJournalSessionId::from_string("terminal_session_symlink");
    let manifest =
        OperationJournalSessionManifest::new(session_id.clone(), history_scope.clone(), 1_000)
            .expect("valid manifest");
    let target = directory.path().join("outside.json");
    fs::write(
        &target,
        serde_json::to_vec(&manifest).expect("serialize manifest"),
    )
    .expect("write symlink target");
    symlink(
        &target,
        history_store
            .paths_for_session(&session_id)
            .session_manifest_path(),
    )
    .expect("create manifest symlink");

    let discovery = history_store.discover(&history_scope, &[]);
    assert!(discovery.histories().is_empty());
    assert!(discovery.warnings().iter().any(
        |warning| warning.kind() == OperationJournalRecoveryWarningKind::ManifestNotRegularFile
    ));
}

#[cfg(unix)]
#[test]
fn session_manifests_are_written_with_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().expect("temp directory");
    let history_store = store(&directory, OperationJournalHistoryConfig::default());
    let session_id = OperationJournalSessionId::from_string("terminal_session_permissions");
    let manifest =
        OperationJournalSessionManifest::new(session_id.clone(), scope("connection-a"), 1_000)
            .expect("valid manifest");
    history_store
        .write_manifest(&manifest)
        .expect("persist manifest");

    let mode = fs::metadata(
        history_store
            .paths_for_session(&session_id)
            .session_manifest_path(),
    )
    .expect("manifest metadata")
    .permissions()
    .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}
