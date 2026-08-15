use super::{
    RecordingBackend, RecordingCompleteness, RecordingFileError, RecordingFileLimits,
    RecordingHeader, SessionLogFavorites, read_recording_for_playback,
};
use chrono::{TimeZone, Utc};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SESSION_LOGS_DIRECTORY: &str = "session-logs";
const MAX_RECORDING_ID_LENGTH: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionLogEntry {
    pub path: PathBuf,
    pub header: RecordingHeader,
    pub duration: Duration,
    pub completeness: RecordingCompleteness,
    pub favorite: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionLogScanIssue {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionLogCatalog {
    pub entries: Vec<SessionLogEntry>,
    pub skipped: Vec<SessionLogScanIssue>,
}

pub fn session_logs_directory(data_directory: impl AsRef<Path>) -> PathBuf {
    data_directory.as_ref().join(SESSION_LOGS_DIRECTORY)
}

pub fn session_log_path(
    session_logs_directory: impl AsRef<Path>,
    backend: RecordingBackend,
    started_at_unix_ms: u64,
    recording_id: &str,
) -> Option<PathBuf> {
    if !is_safe_recording_id(recording_id) {
        return None;
    }
    let timestamp = i64::try_from(started_at_unix_ms).ok()?;
    let started_at = Utc.timestamp_millis_opt(timestamp).single()?;
    let backend = match backend {
        RecordingBackend::Local => "local",
        RecordingBackend::Ssh => "ssh",
        RecordingBackend::Serial => "serial",
    };
    let directory = session_logs_directory
        .as_ref()
        .join(started_at.format("%Y").to_string())
        .join(started_at.format("%m").to_string());
    let file_name = format!(
        "{}-{backend}-{recording_id}.cast",
        started_at.format("%Y%m%d-%H%M%S-%3f")
    );
    Some(directory.join(file_name))
}

pub fn scan_session_logs(
    directory: impl AsRef<Path>,
    limits: RecordingFileLimits,
    favorites: &SessionLogFavorites,
) -> std::io::Result<SessionLogCatalog> {
    let directory = directory.as_ref();
    if !directory.exists() {
        return Ok(SessionLogCatalog::default());
    }
    let mut catalog = SessionLogCatalog::default();
    scan_directory(directory, &limits, favorites, &mut catalog)?;
    catalog.entries.sort_by(|left, right| {
        right
            .header
            .navop
            .started_at_unix_ms
            .cmp(&left.header.navop.started_at_unix_ms)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(catalog)
}

fn scan_directory(
    directory: &Path,
    limits: &RecordingFileLimits,
    favorites: &SessionLogFavorites,
    catalog: &mut SessionLogCatalog,
) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                push_scan_issue(catalog, directory.to_path_buf(), error);
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                push_scan_issue(catalog, path, error);
                continue;
            }
        };
        if file_type.is_dir() {
            if let Err(error) = scan_directory(&path, limits, favorites, catalog) {
                push_scan_issue(catalog, path, error);
            }
        } else if file_type.is_file() && is_recording_path(&path) {
            append_recording(path, limits, favorites, catalog);
        }
    }
    Ok(())
}

fn append_recording(
    path: PathBuf,
    limits: &RecordingFileLimits,
    favorites: &SessionLogFavorites,
    catalog: &mut SessionLogCatalog,
) {
    let recording = match read_recording_for_playback(&path, limits.clone()) {
        Err(RecordingFileError::FileChangedDuringRecovery) if is_partial_recording_path(&path) => {
            read_recording_for_playback(&path, limits.clone())
        }
        result => result,
    };
    match recording {
        Ok(recording) => {
            let duration = recording
                .events
                .last()
                .map_or(Duration::ZERO, |event| event.elapsed);
            let favorite = favorites.contains(&recording.header.navop.recording_id);
            catalog.entries.push(SessionLogEntry {
                path,
                header: recording.header,
                duration,
                completeness: recording.completeness,
                favorite,
            });
        }
        Err(error) => catalog.skipped.push(SessionLogScanIssue {
            path,
            message: error.to_string(),
        }),
    }
}

fn is_recording_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.ends_with(".cast") || name.ends_with(".cast.partial")
}

fn is_partial_recording_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".cast.partial"))
}

fn is_safe_recording_id(recording_id: &str) -> bool {
    let length = recording_id.len();
    (1..=MAX_RECORDING_ID_LENGTH).contains(&length)
        && recording_id != "."
        && recording_id != ".."
        && recording_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn push_scan_issue(catalog: &mut SessionLogCatalog, path: PathBuf, error: impl std::fmt::Display) {
    catalog.skipped.push(SessionLogScanIssue {
        path,
        message: error.to_string(),
    });
}
