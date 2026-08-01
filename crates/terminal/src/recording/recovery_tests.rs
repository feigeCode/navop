use super::{
    RecordingBackend, RecordingCompleteness, RecordingEvent, RecordingEventKind,
    RecordingFileConfig, RecordingFileError, RecordingFileLimit, RecordingFileLimits,
    RecordingFileWriter, RecordingMetadata, RecordingPlayback, RecordingPlaybackLimits,
    partial_recording_path, read_recording_for_playback,
};
use crate::TerminalSize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::tempdir;

fn metadata() -> RecordingMetadata {
    RecordingMetadata {
        recording_id: "playback-recording".to_string(),
        session_id: "playback-session".to_string(),
        backend: RecordingBackend::Ssh,
        application_version: "0.1.0-test".to_string(),
        started_at_unix_ms: 1_700_000_000_123,
        capture_input: false,
    }
}

fn writer(final_path: &Path) -> RecordingFileWriter {
    RecordingFileWriter::create(
        final_path,
        metadata(),
        TerminalSize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        },
        RecordingFileConfig {
            flush_every_events: 1,
            ..RecordingFileConfig::default()
        },
    )
    .unwrap()
}

fn append_output(writer: &mut RecordingFileWriter, seconds: u64, output: &[u8]) {
    writer
        .append(&RecordingEvent {
            elapsed: Duration::from_secs(seconds),
            kind: RecordingEventKind::Output(output.to_vec()),
        })
        .unwrap();
}

fn crashed_recording_with_tail(tail: &[u8]) -> (tempfile::TempDir, PathBuf, Vec<u8>, u64) {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("session.cast");
    let partial_path = partial_recording_path(&final_path).unwrap();
    let mut recording_writer = writer(&final_path);
    append_output(&mut recording_writer, 1, b"complete");
    recording_writer.flush().unwrap();
    drop(recording_writer);
    let valid_bytes = fs::metadata(&partial_path).unwrap().len();
    OpenOptions::new()
        .append(true)
        .open(&partial_path)
        .unwrap()
        .write_all(tail)
        .unwrap();
    let original = fs::read(&partial_path).unwrap();
    (temp, partial_path, original, valid_bytes)
}

#[test]
fn playback_read_accepts_complete_recording_without_mutation() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("complete.cast");
    let mut recording_writer = writer(&final_path);
    append_output(&mut recording_writer, 1, b"complete");
    recording_writer.stop().unwrap();
    let original = fs::read(&final_path).unwrap();

    let recording =
        read_recording_for_playback(&final_path, RecordingFileLimits::default()).unwrap();

    assert_eq!(RecordingCompleteness::Complete, recording.completeness);
    assert_eq!(1, recording.events.len());
    assert_eq!(original, fs::read(&final_path).unwrap());
}

#[test]
fn playback_read_recovers_a_partial_tail_without_mutating_the_file() {
    let (_temp, partial_path, original, valid_bytes) =
        crashed_recording_with_tail(b"[2.0,\"o\",\"truncated");

    let recording =
        read_recording_for_playback(&partial_path, RecordingFileLimits::default()).unwrap();

    let discarded_bytes = original.len() as u64 - valid_bytes;
    assert_eq!(
        RecordingCompleteness::Partial { discarded_bytes },
        recording.completeness
    );
    assert_eq!(1, recording.events.len());
    assert_eq!(
        original.len() as u64,
        fs::metadata(&partial_path).unwrap().len()
    );
    assert_eq!(original, fs::read(&partial_path).unwrap());
}

#[test]
fn playback_read_keeps_an_unpublished_complete_prefix_marked_partial() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("unfinished.cast");
    let partial_path = partial_recording_path(&final_path).unwrap();
    let mut recording_writer = writer(&final_path);
    append_output(&mut recording_writer, 1, b"complete prefix");
    recording_writer.flush().unwrap();
    drop(recording_writer);
    let original = fs::read(&partial_path).unwrap();

    let recording =
        read_recording_for_playback(&partial_path, RecordingFileLimits::default()).unwrap();

    assert_eq!(
        RecordingCompleteness::Partial { discarded_bytes: 0 },
        recording.completeness
    );
    assert_eq!(original, fs::read(&partial_path).unwrap());
}

#[test]
fn playback_open_uses_non_destructive_partial_recovery() {
    let (_temp, partial_path, original, _valid_bytes) =
        crashed_recording_with_tail(b"[2.0,\"o\",\"truncated");

    let playback = RecordingPlayback::open(
        &partial_path,
        RecordingFileLimits::default(),
        RecordingPlaybackLimits::default(),
    )
    .unwrap();

    assert!(matches!(
        playback.completeness(),
        RecordingCompleteness::Partial {
            discarded_bytes
        } if *discarded_bytes > 0
    ));
    assert_eq!(original, fs::read(&partial_path).unwrap());
}

#[test]
fn playback_read_does_not_recover_a_truncated_published_recording() {
    let (temp, partial_path, original, _valid_bytes) =
        crashed_recording_with_tail(b"[2.0,\"o\",\"truncated");
    let published_path = temp.path().join("published.cast");
    fs::rename(&partial_path, &published_path).unwrap();

    let error =
        read_recording_for_playback(&published_path, RecordingFileLimits::default()).unwrap_err();

    assert!(matches!(error, RecordingFileError::InvalidEvent { .. }));
    assert_eq!(original, fs::read(&published_path).unwrap());
}

#[test]
fn playback_read_rejects_newline_terminated_corruption_without_mutation() {
    let (_temp, partial_path, original, _valid_bytes) = crashed_recording_with_tail(b"not-json\n");

    let error =
        read_recording_for_playback(&partial_path, RecordingFileLimits::default()).unwrap_err();

    assert!(matches!(
        error,
        RecordingFileError::Json(_) | RecordingFileError::InvalidEvent { .. }
    ));
    assert_eq!(original, fs::read(&partial_path).unwrap());
}

#[test]
fn playback_read_preserves_event_and_file_limits_without_mutation() {
    let (_temp, partial_path, original, _valid_bytes) =
        crashed_recording_with_tail(format!("[2.0,\"o\",\"{}\"]\n", "x".repeat(128)).as_bytes());

    let event_error = read_recording_for_playback(
        &partial_path,
        RecordingFileLimits {
            max_serialized_event_bytes: 32,
            ..RecordingFileLimits::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        event_error,
        RecordingFileError::LimitReached(RecordingFileLimit::EventBytes)
    ));

    let file_error = read_recording_for_playback(
        &partial_path,
        RecordingFileLimits {
            max_file_bytes: original.len() as u64 - 1,
            ..RecordingFileLimits::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        file_error,
        RecordingFileError::LimitReached(RecordingFileLimit::FileBytes)
    ));
    assert_eq!(original, fs::read(&partial_path).unwrap());
}

#[test]
fn playback_read_rejects_unknown_versions_and_backwards_timestamps() {
    let temp = tempdir().unwrap();
    let unknown_path = temp.path().join("unknown.cast.partial");
    fs::write(
        &unknown_path,
        concat!(
            "{\"version\":99,\"width\":80,\"height\":24,\"timestamp\":1700000000,",
            "\"navop\":{\"format_version\":1,\"recording_id\":\"r\",",
            "\"session_id\":\"s\",\"backend\":\"local\",",
            "\"application_version\":\"test\",\"started_at_unix_ms\":1700000000000,",
            "\"capture_input\":false,\"event_stream\":\"terminal_parser_input_v1\"}}\n"
        ),
    )
    .unwrap();
    let unknown_original = fs::read(&unknown_path).unwrap();

    let version_error =
        read_recording_for_playback(&unknown_path, RecordingFileLimits::default()).unwrap_err();
    assert!(matches!(
        version_error,
        RecordingFileError::UnknownAsciicastVersion(99)
    ));
    assert_eq!(unknown_original, fs::read(&unknown_path).unwrap());

    let final_path = temp.path().join("backwards.cast");
    let partial_path = partial_recording_path(&final_path).unwrap();
    let mut recording_writer = writer(&final_path);
    append_output(&mut recording_writer, 2, b"later");
    drop(recording_writer);
    OpenOptions::new()
        .append(true)
        .open(&partial_path)
        .unwrap()
        .write_all(b"[1.0,\"o\",\"earlier\"]\n")
        .unwrap();
    let backwards_original = fs::read(&partial_path).unwrap();

    let timestamp_error =
        read_recording_for_playback(&partial_path, RecordingFileLimits::default()).unwrap_err();
    assert!(matches!(
        timestamp_error,
        RecordingFileError::InvalidEvent { .. }
    ));
    assert_eq!(backwards_original, fs::read(&partial_path).unwrap());
}
