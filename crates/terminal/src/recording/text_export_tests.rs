use super::text_export::retained_line_bounds;
use super::{
    RecordingBackend, RecordingCompleteness, RecordingEvent, RecordingEventKind,
    RecordingFileConfig, RecordingFileLimits, RecordingFileWriter, RecordingMetadata,
    RecordingTextExport, export_recording_text, partial_recording_path,
};
use crate::TerminalSize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::tempdir;

fn metadata(capture_input: bool) -> RecordingMetadata {
    RecordingMetadata {
        recording_id: "text-export-recording".to_string(),
        session_id: "text-export-session".to_string(),
        backend: RecordingBackend::Ssh,
        application_version: "0.1.0-test".to_string(),
        started_at_unix_ms: 1_700_000_000_123,
        capture_input,
        session: None,
    }
}

fn size(rows: u16, cols: u16) -> TerminalSize {
    TerminalSize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn event(elapsed_ms: u64, kind: RecordingEventKind) -> RecordingEvent {
    RecordingEvent {
        elapsed: Duration::from_millis(elapsed_ms),
        kind,
    }
}

fn write_recording(
    path: &Path,
    capture_input: bool,
    events: &[RecordingEvent],
    publish: bool,
) -> PathBuf {
    let mut writer = RecordingFileWriter::create(
        path,
        metadata(capture_input),
        size(4, 40),
        RecordingFileConfig::default(),
    )
    .unwrap();
    for event in events {
        writer.append(event).unwrap();
    }
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

fn export(path: &Path) -> RecordingTextExport {
    export_recording_text(path, RecordingFileLimits::default(), 1_000).unwrap()
}

#[test]
fn text_export_applies_terminal_control_semantics_instead_of_stripping_bytes() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("controls.cast");
    write_recording(
        &path,
        false,
        &[
            event(
                0,
                RecordingEventKind::Output(b"progress 10%\rprogress 100%\r\n".to_vec()),
            ),
            event(
                1,
                RecordingEventKind::Output(b"\x1b[31mred\x1b[0m\r\n".to_vec()),
            ),
            event(2, RecordingEventKind::Output(b"abc\x08D".to_vec())),
        ],
        true,
    );

    let exported = export(&path);
    assert_eq!("progress 100%\nred\nabD", exported.text);
    assert_eq!(4, exported.screen_lines);
    assert_eq!(40, exported.columns);
}

#[test]
fn text_export_preserves_parser_state_across_events_and_ignores_private_metadata() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("chunks.cast");
    let utf8 = "界".as_bytes();
    write_recording(
        &path,
        true,
        &[
            event(0, RecordingEventKind::Output(utf8[..1].to_vec())),
            event(1, RecordingEventKind::Output(utf8[1..].to_vec())),
            event(2, RecordingEventKind::Output(b"\x1b[3".to_vec())),
            event(3, RecordingEventKind::Output(b"1mR\x1b[0m".to_vec())),
            event(4, RecordingEventKind::Input(b"SECRET".to_vec())),
            event(5, RecordingEventKind::Marker("INTERNAL".to_string())),
        ],
        true,
    );

    let exported = export(&path);
    assert_eq!("界 R", exported.text);
    assert!(!exported.text.contains("SECRET"));
    assert!(!exported.text.contains("INTERNAL"));
}

#[test]
fn text_export_reads_partial_recordings_without_modifying_them() {
    let temp = tempdir().unwrap();
    let final_path = temp.path().join("partial.cast");
    let partial_path = write_recording(
        &final_path,
        false,
        &[event(
            0,
            RecordingEventKind::Output(b"recoverable".to_vec()),
        )],
        false,
    );
    let before = std::fs::read(&partial_path).unwrap();

    let exported = export(&partial_path);

    assert_eq!("recoverable", exported.text);
    assert!(matches!(
        exported.completeness,
        RecordingCompleteness::Partial { .. }
    ));
    assert_eq!(before, std::fs::read(&partial_path).unwrap());
    assert!(!final_path.exists());
}

#[test]
fn text_export_applies_resize_and_retains_scrollback() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("resize.cast");
    write_recording(
        &path,
        false,
        &[
            event(0, RecordingEventKind::Output(b"one\r\ntwo\r\n".to_vec())),
            event(1, RecordingEventKind::Resize(size(2, 20))),
            event(2, RecordingEventKind::Output(b"three\r\nfour".to_vec())),
        ],
        true,
    );

    let exported = export(&path);
    assert_eq!("one\ntwo\nthree\nfour", exported.text);
    assert!(exported.history_size >= 2);
    assert_eq!(2, exported.screen_lines);
    assert_eq!(20, exported.columns);
}

#[test]
fn text_export_of_empty_recording_returns_empty_text() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("empty.cast");
    write_recording(&path, false, &[], true);

    let exported = export(&path);

    assert!(exported.text.is_empty());
    assert_eq!(0, exported.history_size);
}

#[test]
fn text_export_with_only_non_output_events_returns_empty_text() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("metadata-only.cast");
    write_recording(
        &path,
        true,
        &[
            event(0, RecordingEventKind::Input(b"SECRET".to_vec())),
            event(1, RecordingEventKind::Marker("internal".to_string())),
            event(2, RecordingEventKind::Resize(size(2, 20))),
        ],
        true,
    );

    let exported = export(&path);

    assert!(exported.text.is_empty());
}

#[test]
fn retained_line_bounds_saturate_before_converting_to_terminal_indices() {
    assert_eq!(
        (-i32::MAX, i32::MAX - 1),
        retained_line_bounds(usize::MAX, usize::MAX)
    );
    assert_eq!((0, -1), retained_line_bounds(0, 0));
}
