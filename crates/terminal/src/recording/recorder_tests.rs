use super::RecordingFileError;
use super::recorder::create_partial_file;
use std::fs;
use std::io::{self, Write};
use tempfile::tempdir;

#[test]
fn failed_header_initialization_removes_the_new_partial_file() {
    let temp = tempdir().unwrap();
    let partial_path = temp.path().join("failed.cast.partial");

    let error = create_partial_file(&partial_path, |_| {
        Err(RecordingFileError::io(
            "initialize test recording",
            io::Error::other("injected failure"),
        ))
    })
    .unwrap_err();

    assert!(matches!(error, RecordingFileError::Io { .. }));
    assert!(!partial_path.exists());
}

#[test]
fn existing_partial_file_is_never_replaced() {
    let temp = tempdir().unwrap();
    let partial_path = temp.path().join("existing.cast.partial");
    fs::write(&partial_path, b"existing recording").unwrap();

    let error = create_partial_file(&partial_path, |file| {
        file.write_all(b"replacement")
            .map_err(|error| RecordingFileError::io("write test recording", error))
    })
    .unwrap_err();

    assert!(matches!(error, RecordingFileError::Io { .. }));
    assert_eq!(
        b"existing recording",
        fs::read(&partial_path).unwrap().as_slice()
    );
}
