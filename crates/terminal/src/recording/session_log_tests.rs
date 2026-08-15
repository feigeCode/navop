use super::{
    RecordingArtifactKind, RecordingBackend, RecordingCompleteness, RecordingEvent,
    RecordingEventKind, RecordingFileConfig, RecordingFileLimits, RecordingFileWriter,
    RecordingMetadata, SessionLogFavorites, load_session_log_favorites, partial_recording_path,
    save_session_log_favorites, scan_session_logs, session_log_path, session_logs_directory,
};
use crate::TerminalSize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::tempdir;

fn metadata(recording_id: &str, started_at_unix_ms: u64) -> RecordingMetadata {
    RecordingMetadata {
        recording_id: recording_id.to_string(),
        session_id: format!("session-{recording_id}"),
        backend: RecordingBackend::Ssh,
        artifact_kind: RecordingArtifactKind::SessionLog,
        application_version: "0.1.0-test".to_string(),
        started_at_unix_ms,
        capture_input: false,
        session: None,
    }
}

fn recording_path(root: &Path, recording_id: &str, started_at_unix_ms: u64) -> PathBuf {
    session_log_path(
        root,
        RecordingBackend::Ssh,
        started_at_unix_ms,
        recording_id,
    )
    .unwrap()
}

fn write_recording(path: &Path, recording_id: &str, started_at: u64, publish: bool) -> PathBuf {
    let mut writer = RecordingFileWriter::create(
        path,
        metadata(recording_id, started_at),
        TerminalSize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        },
        RecordingFileConfig::default(),
    )
    .unwrap();
    writer
        .append(&RecordingEvent {
            elapsed: Duration::from_millis(2_500),
            kind: RecordingEventKind::Output(b"hello".to_vec()),
        })
        .unwrap();
    if publish {
        writer.stop().unwrap();
        path.to_path_buf()
    } else {
        writer.flush().unwrap();
        let partial = partial_recording_path(path).unwrap();
        drop(writer);
        partial
    }
}

#[test]
fn session_log_path_uses_utc_year_month_and_backend() {
    let root = Path::new("/logs");
    let path = session_log_path(
        root,
        RecordingBackend::Serial,
        1_723_651_441_123,
        "recording-id",
    )
    .unwrap();

    assert_eq!(
        root.join("2024/08/20240814-160401-123-serial-recording-id.cast"),
        path
    );
}

#[test]
fn session_log_path_rejects_unsafe_recording_ids() {
    let root = Path::new("/logs");
    for recording_id in [
        "",
        ".",
        "..",
        "../escape",
        "nested/escape",
        r"nested\escape",
        "/absolute",
        "control\ncharacter",
    ] {
        assert!(
            session_log_path(
                root,
                RecordingBackend::Local,
                1_723_651_441_123,
                recording_id,
            )
            .is_none(),
            "recording ID should be rejected: {recording_id:?}"
        );
    }
}

#[test]
fn session_log_path_rejects_excessively_long_recording_ids() {
    let recording_id = "a".repeat(129);
    assert!(
        session_log_path(
            "/logs",
            RecordingBackend::Local,
            1_723_651_441_123,
            &recording_id,
        )
        .is_none()
    );
}

#[test]
fn session_logs_directory_is_scoped_under_application_data() {
    assert_eq!(
        Path::new("/data").join("session-logs"),
        session_logs_directory("/data")
    );
}

#[test]
fn scan_reads_nested_complete_and_partial_logs_in_reverse_start_order() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("session-logs");
    let older = recording_path(&root, "older", 1_700_000_000_000);
    let newer = recording_path(&root, "newer", 1_710_000_000_000);
    write_recording(&older, "older", 1_700_000_000_000, true);
    let partial = write_recording(&newer, "newer", 1_710_000_000_000, false);
    let before = fs::read(&partial).unwrap();

    let catalog = scan_session_logs(
        &root,
        RecordingFileLimits::default(),
        &SessionLogFavorites::default(),
    )
    .unwrap();

    assert!(catalog.skipped.is_empty());
    assert_eq!(2, catalog.entries.len());
    assert_eq!("newer", catalog.entries[0].header.navop.recording_id);
    assert_eq!(Duration::from_millis(2_500), catalog.entries[0].duration);
    assert!(matches!(
        catalog.entries[0].completeness,
        RecordingCompleteness::Partial { .. }
    ));
    assert_eq!("older", catalog.entries[1].header.navop.recording_id);
    assert_eq!(
        RecordingCompleteness::Complete,
        catalog.entries[1].completeness
    );
    assert_eq!(before, fs::read(&partial).unwrap());
}

#[test]
fn scan_reads_header_only_partial_log_immediately() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("session-logs");
    let final_path = recording_path(&root, "header-only", 1_710_000_000_000);
    let writer = RecordingFileWriter::create(
        &final_path,
        metadata("header-only", 1_710_000_000_000),
        TerminalSize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        },
        RecordingFileConfig::default(),
    )
    .unwrap();
    let partial = writer.partial_path().to_path_buf();
    drop(writer);
    let before = fs::read(&partial).unwrap();

    let catalog = scan_session_logs(
        &root,
        RecordingFileLimits::default(),
        &SessionLogFavorites::default(),
    )
    .unwrap();

    assert!(catalog.skipped.is_empty());
    assert_eq!(1, catalog.entries.len());
    assert_eq!("header-only", catalog.entries[0].header.navop.recording_id);
    assert_eq!(Duration::ZERO, catalog.entries[0].duration);
    assert_eq!(
        RecordingCompleteness::Partial { discarded_bytes: 0 },
        catalog.entries[0].completeness
    );
    assert_eq!(before, fs::read(&partial).unwrap());
}

#[test]
fn active_session_log_events_are_visible_without_durable_flush() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("session-logs");
    let final_path = recording_path(&root, "active", 1_710_000_000_000);
    let mut writer = RecordingFileWriter::create(
        &final_path,
        metadata("active", 1_710_000_000_000),
        TerminalSize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        },
        RecordingFileConfig::default(),
    )
    .unwrap();
    writer
        .append(&RecordingEvent {
            elapsed: Duration::from_millis(750),
            kind: RecordingEventKind::Output(b"prompt output".to_vec()),
        })
        .unwrap();

    let catalog = scan_session_logs(
        &root,
        RecordingFileLimits::default(),
        &SessionLogFavorites::default(),
    )
    .unwrap();

    assert!(catalog.skipped.is_empty());
    assert_eq!(1, catalog.entries.len());
    assert_eq!("active", catalog.entries[0].header.navop.recording_id);
    assert_eq!(Duration::from_millis(750), catalog.entries[0].duration);
    assert!(matches!(
        catalog.entries[0].completeness,
        RecordingCompleteness::Partial { .. }
    ));
}

#[test]
fn scan_ignores_non_recordings_and_isolates_corrupt_files() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("session-logs");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("notes.txt"), b"ignore").unwrap();
    fs::write(root.join("broken.cast"), b"not-json\n").unwrap();
    let valid = recording_path(&root, "valid", 1_700_000_000_000);
    write_recording(&valid, "valid", 1_700_000_000_000, true);

    let catalog = scan_session_logs(
        &root,
        RecordingFileLimits::default(),
        &SessionLogFavorites::default(),
    )
    .unwrap();

    assert_eq!(1, catalog.entries.len());
    assert_eq!("valid", catalog.entries[0].header.navop.recording_id);
    assert_eq!(1, catalog.skipped.len());
    assert_eq!(root.join("broken.cast"), catalog.skipped[0].path);
}

#[cfg(unix)]
#[test]
fn scan_isolates_unreadable_nested_directories() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempdir().unwrap();
    let root = temp.path().join("session-logs");
    let valid = recording_path(&root, "valid", 1_700_000_000_000);
    write_recording(&valid, "valid", 1_700_000_000_000, true);
    let unreadable = root.join("unreadable");
    fs::create_dir_all(&unreadable).unwrap();
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();

    let result = scan_session_logs(
        &root,
        RecordingFileLimits::default(),
        &SessionLogFavorites::default(),
    );
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o700)).unwrap();
    let catalog = result.unwrap();

    assert_eq!(1, catalog.entries.len());
    assert_eq!("valid", catalog.entries[0].header.navop.recording_id);
    assert_eq!(1, catalog.skipped.len());
    assert_eq!(unreadable, catalog.skipped[0].path);
}

#[test]
fn favorites_roundtrip_and_apply_to_scanned_entries() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("session-logs");
    let path = recording_path(&root, "favorite", 1_700_000_000_000);
    write_recording(&path, "favorite", 1_700_000_000_000, true);
    let mut favorites = load_session_log_favorites(&root).unwrap();
    assert!(favorites.is_empty());
    assert!(favorites.set("favorite", true));
    assert!(!favorites.set("favorite", true));

    save_session_log_favorites(&root, &favorites).unwrap();
    let loaded = load_session_log_favorites(&root).unwrap();
    let catalog = scan_session_logs(&root, RecordingFileLimits::default(), &loaded).unwrap();

    assert!(loaded.contains("favorite"));
    assert!(catalog.entries[0].favorite);
    assert!(
        fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp"))
    );
}

#[test]
fn favorite_removal_is_persisted_and_invalid_json_is_reported() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("session-logs");
    let mut favorites = SessionLogFavorites::default();
    favorites.set("recording", true);
    assert!(favorites.set("recording", false));
    assert!(!favorites.set("recording", false));
    save_session_log_favorites(&root, &favorites).unwrap();
    assert!(load_session_log_favorites(&root).unwrap().is_empty());

    fs::write(root.join("favorites.json"), b"{").unwrap();
    let error = load_session_log_favorites(&root).unwrap_err();
    assert_eq!(std::io::ErrorKind::InvalidData, error.kind());
}

#[test]
fn saving_favorites_twice_replaces_existing_sidecar() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("session-logs");
    let mut favorites = SessionLogFavorites::default();
    favorites.set("old-recording", true);
    save_session_log_favorites(&root, &favorites).unwrap();

    favorites.set("old-recording", false);
    favorites.set("new-recording", true);
    save_session_log_favorites(&root, &favorites).unwrap();

    let loaded = load_session_log_favorites(&root).unwrap();
    assert!(!loaded.contains("old-recording"));
    assert!(loaded.contains("new-recording"));
}
