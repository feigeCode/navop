use super::{
    ASCIICAST_VERSION, NAVOP_RECORDING_FORMAT_VERSION, RecordingBackend, RecordingCompleteness,
    RecordingEvent, RecordingEventKind, RecordingFileConfig, RecordingFileError,
    RecordingFileLimit, RecordingFileLimits, RecordingFileState, RecordingFileTransition,
    RecordingFileWriter, RecordingMetadata, partial_recording_path, read_recording,
    recover_partial_recording,
};
use crate::TerminalSize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::time::Duration;
use tempfile::tempdir;

fn metadata(capture_input: bool) -> RecordingMetadata {
    RecordingMetadata {
        recording_id: "recording-1".to_string(),
        session_id: "session-1".to_string(),
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
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn event(elapsed_seconds: u64, kind: RecordingEventKind) -> RecordingEvent {
    RecordingEvent {
        elapsed: Duration::from_secs(elapsed_seconds),
        kind,
    }
}

#[test]
fn durable_writer_publishes_a_versioned_recording_only_after_stop() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("session.cast");
    let partial_path = partial_recording_path(&final_path).unwrap();
    let mut writer = RecordingFileWriter::create(
        &final_path,
        metadata(false),
        initial_size(),
        RecordingFileConfig {
            flush_every_events: 1,
            ..RecordingFileConfig::default()
        },
    )
    .unwrap();

    assert!(!final_path.exists());
    assert!(partial_path.exists());
    writer
        .append(&event(1, RecordingEventKind::Output(b"hello".to_vec())))
        .unwrap();
    writer
        .append(&event(
            2,
            RecordingEventKind::Resize(TerminalSize {
                rows: 30,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            }),
        ))
        .unwrap();
    writer
        .append(&event(3, RecordingEventKind::Marker("deploy".to_string())))
        .unwrap();

    let partial = fs::read_to_string(&partial_path).unwrap();
    assert!(partial.contains("\"version\":2"));
    assert!(partial.contains("\"format_version\":1"));
    assert!(partial.contains("\"o\",\"hello\""));
    assert!(!final_path.exists());

    assert_eq!(RecordingFileTransition::Changed, writer.stop().unwrap());
    assert_eq!(&RecordingFileState::Published, writer.state());
    assert!(final_path.exists());
    assert!(!partial_path.exists());
    assert_eq!(RecordingFileTransition::Unchanged, writer.stop().unwrap());

    let recording = read_recording(&final_path, RecordingFileLimits::default()).unwrap();
    assert_eq!(ASCIICAST_VERSION, recording.header.version);
    assert_eq!(
        NAVOP_RECORDING_FORMAT_VERSION,
        recording.header.navop.format_version
    );
    assert_eq!("recording-1", recording.header.navop.recording_id);
    assert_eq!("session-1", recording.header.navop.session_id);
    assert_eq!(RecordingBackend::Local, recording.header.navop.backend);
    assert_eq!(RecordingCompleteness::Complete, recording.completeness);
    assert_eq!(
        vec![
            event(1, RecordingEventKind::Output(b"hello".to_vec())),
            event(
                2,
                RecordingEventKind::Resize(TerminalSize {
                    rows: 30,
                    cols: 100,
                    pixel_width: 0,
                    pixel_height: 0,
                })
            ),
            event(3, RecordingEventKind::Marker("deploy".to_string())),
        ],
        recording.events
    );
}

#[test]
fn binary_output_roundtrips_without_requiring_utf8() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("binary.cast");
    let bytes = vec![0xff, 0x00, 0xfe, b'\n'];
    let mut writer = RecordingFileWriter::create(
        &final_path,
        metadata(false),
        initial_size(),
        RecordingFileConfig::default(),
    )
    .unwrap();

    writer
        .append(&event(1, RecordingEventKind::Output(bytes.clone())))
        .unwrap();
    writer.stop().unwrap();

    let recording = read_recording(&final_path, RecordingFileLimits::default()).unwrap();
    assert_eq!(RecordingEventKind::Output(bytes), recording.events[0].kind);
}

#[test]
fn input_is_rejected_before_serialization_unless_header_opted_in() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("private.cast");
    let partial_path = partial_recording_path(&final_path).unwrap();
    let mut writer = RecordingFileWriter::create(
        &final_path,
        metadata(false),
        initial_size(),
        RecordingFileConfig::default(),
    )
    .unwrap();

    let error = writer
        .append(&event(
            1,
            RecordingEventKind::Input(b"secret-value".to_vec()),
        ))
        .unwrap_err();

    assert!(matches!(error, RecordingFileError::InputCaptureDisabled));
    assert_eq!(&RecordingFileState::Failed, writer.state());
    assert!(!final_path.exists());
    assert!(partial_path.exists());
    assert!(
        !fs::read_to_string(partial_path)
            .unwrap()
            .contains("secret-value")
    );

    let opted_in_path = temp.path().join("opted-in.cast");
    let mut opted_in = RecordingFileWriter::create(
        &opted_in_path,
        metadata(true),
        initial_size(),
        RecordingFileConfig::default(),
    )
    .unwrap();
    opted_in
        .append(&event(
            1,
            RecordingEventKind::Input(b"explicit-input".to_vec()),
        ))
        .unwrap();
    opted_in.stop().unwrap();
    let recording = read_recording(&opted_in_path, RecordingFileLimits::default()).unwrap();
    assert_eq!(
        RecordingEventKind::Input(b"explicit-input".to_vec()),
        recording.events[0].kind
    );
}

#[test]
fn serialized_event_and_file_limits_are_independent_of_payload_limits() {
    let temp = tempdir().unwrap();
    let event_limited_path = temp.path().join("event-limited.cast");
    let mut event_limited = RecordingFileWriter::create(
        &event_limited_path,
        metadata(false),
        initial_size(),
        RecordingFileConfig {
            limits: RecordingFileLimits {
                max_serialized_event_bytes: 32,
                ..RecordingFileLimits::default()
            },
            ..RecordingFileConfig::default()
        },
    )
    .unwrap();

    let error = event_limited
        .append(&event(1, RecordingEventKind::Output(vec![b'x'; 128])))
        .unwrap_err();
    assert!(matches!(
        error,
        RecordingFileError::LimitReached(RecordingFileLimit::EventBytes)
    ));
    assert_eq!(&RecordingFileState::Failed, event_limited.state());
    assert!(!event_limited_path.exists());

    let probe_path = temp.path().join("probe.cast");
    let probe = RecordingFileWriter::create(
        &probe_path,
        metadata(false),
        initial_size(),
        RecordingFileConfig::default(),
    )
    .unwrap();
    let header_bytes = probe.bytes_written();
    drop(probe);

    let file_limited_path = temp.path().join("file-limited.cast");
    let mut file_limited = RecordingFileWriter::create(
        &file_limited_path,
        metadata(false),
        initial_size(),
        RecordingFileConfig {
            limits: RecordingFileLimits {
                max_file_bytes: header_bytes + 16,
                ..RecordingFileLimits::default()
            },
            ..RecordingFileConfig::default()
        },
    )
    .unwrap();
    let error = file_limited
        .append(&event(
            1,
            RecordingEventKind::Output(b"this exceeds sixteen wire bytes".to_vec()),
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        RecordingFileError::LimitReached(RecordingFileLimit::FileBytes)
    ));
    assert_eq!(&RecordingFileState::Failed, file_limited.state());
    assert!(!file_limited_path.exists());
}

#[test]
fn stop_never_overwrites_a_final_path_created_after_recording_started() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("collision.cast");
    let partial_path = partial_recording_path(&final_path).unwrap();
    let mut writer = RecordingFileWriter::create(
        &final_path,
        metadata(false),
        initial_size(),
        RecordingFileConfig::default(),
    )
    .unwrap();
    writer
        .append(&event(1, RecordingEventKind::Output(b"safe".to_vec())))
        .unwrap();
    fs::write(&final_path, b"do-not-overwrite").unwrap();

    let error = writer.stop().unwrap_err();

    assert!(matches!(error, RecordingFileError::FinalPathExists(_)));
    assert_eq!(&RecordingFileState::Failed, writer.state());
    assert_eq!(
        b"do-not-overwrite",
        fs::read(&final_path).unwrap().as_slice()
    );
    assert!(partial_path.exists());
}

#[test]
fn partial_recovery_truncates_only_the_incomplete_tail() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("crashed.cast");
    let partial_path = partial_recording_path(&final_path).unwrap();
    let mut writer = RecordingFileWriter::create(
        &final_path,
        metadata(false),
        initial_size(),
        RecordingFileConfig {
            flush_every_events: 1,
            ..RecordingFileConfig::default()
        },
    )
    .unwrap();
    writer
        .append(&event(1, RecordingEventKind::Output(b"complete".to_vec())))
        .unwrap();
    writer.flush().unwrap();
    drop(writer);

    let valid_bytes = fs::metadata(&partial_path).unwrap().len();
    OpenOptions::new()
        .append(true)
        .open(&partial_path)
        .unwrap()
        .write_all(b"[2.0,\"o\",\"truncated")
        .unwrap();
    let crashed_bytes = fs::metadata(&partial_path).unwrap().len();

    let recovery =
        recover_partial_recording(&partial_path, RecordingFileLimits::default()).unwrap();

    assert_eq!(valid_bytes, recovery.valid_bytes);
    assert_eq!(crashed_bytes - valid_bytes, recovery.discarded_bytes);
    assert_eq!(fs::metadata(&partial_path).unwrap().len(), valid_bytes);
    assert_eq!(
        RecordingCompleteness::Partial {
            discarded_bytes: crashed_bytes - valid_bytes,
        },
        recovery.recording.completeness
    );
    assert_eq!(1, recovery.recording.events.len());
    assert!(!final_path.exists());
}

#[test]
fn recovery_rejects_unknown_versions_and_invalid_timestamps() {
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

    let error =
        recover_partial_recording(&unknown_path, RecordingFileLimits::default()).unwrap_err();
    assert!(matches!(
        error,
        RecordingFileError::UnknownAsciicastVersion(99)
    ));

    let unknown_navop_path = temp.path().join("unknown-navop.cast.partial");
    fs::write(
        &unknown_navop_path,
        concat!(
            "{\"version\":2,\"width\":80,\"height\":24,\"timestamp\":1700000000,",
            "\"navop\":{\"format_version\":99,\"recording_id\":\"r\",",
            "\"session_id\":\"s\",\"backend\":\"local\",",
            "\"application_version\":\"test\",\"started_at_unix_ms\":1700000000000,",
            "\"capture_input\":false,\"event_stream\":\"terminal_parser_input_v1\"}}\n"
        ),
    )
    .unwrap();
    let error =
        recover_partial_recording(&unknown_navop_path, RecordingFileLimits::default()).unwrap_err();
    assert!(matches!(error, RecordingFileError::UnknownNavopVersion(99)));

    let invalid_time_path = temp.path().join("invalid-time.cast");
    let mut writer = RecordingFileWriter::create(
        &invalid_time_path,
        metadata(false),
        initial_size(),
        RecordingFileConfig {
            flush_every_events: 1,
            ..RecordingFileConfig::default()
        },
    )
    .unwrap();
    writer
        .append(&event(1, RecordingEventKind::Output(b"valid".to_vec())))
        .unwrap();
    writer.flush().unwrap();
    let partial_path = writer.partial_path().to_path_buf();
    drop(writer);
    let header = fs::read_to_string(&partial_path)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    fs::write(&partial_path, format!("{header}\n[-1.0,\"o\",\"bad\"]\n")).unwrap();

    let error =
        recover_partial_recording(&partial_path, RecordingFileLimits::default()).unwrap_err();
    assert!(matches!(error, RecordingFileError::InvalidEvent { .. }));
}

#[test]
fn recovery_rejects_oversized_events_and_input_that_header_did_not_disclose() {
    let temp = tempdir().unwrap();
    let oversized_final = temp.path().join("oversized.cast");
    let oversized_partial = partial_recording_path(&oversized_final).unwrap();
    let writer = RecordingFileWriter::create(
        &oversized_final,
        metadata(false),
        initial_size(),
        RecordingFileConfig::default(),
    )
    .unwrap();
    drop(writer);
    let oversized_event = format!("[1.0,\"o\",\"{}\"]\n", "x".repeat(128));
    OpenOptions::new()
        .append(true)
        .open(&oversized_partial)
        .unwrap()
        .write_all(oversized_event.as_bytes())
        .unwrap();
    let error = recover_partial_recording(
        &oversized_partial,
        RecordingFileLimits {
            max_serialized_event_bytes: 32,
            ..RecordingFileLimits::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        error,
        RecordingFileError::LimitReached(RecordingFileLimit::EventBytes)
    ));

    let undisclosed_final = temp.path().join("undisclosed-input.cast");
    let undisclosed_partial = partial_recording_path(&undisclosed_final).unwrap();
    let writer = RecordingFileWriter::create(
        &undisclosed_final,
        metadata(false),
        initial_size(),
        RecordingFileConfig::default(),
    )
    .unwrap();
    drop(writer);
    OpenOptions::new()
        .append(true)
        .open(&undisclosed_partial)
        .unwrap()
        .write_all(b"[1.0,\"i\",\"not-disclosed\"]\n")
        .unwrap();
    let error = recover_partial_recording(&undisclosed_partial, RecordingFileLimits::default())
        .unwrap_err();
    assert!(matches!(error, RecordingFileError::InvalidEvent { .. }));
}
