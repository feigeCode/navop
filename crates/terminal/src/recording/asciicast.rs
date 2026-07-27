use super::model::DEFAULT_MAX_RECORDING_EVENTS;
use super::{RecordingEvent, RecordingEventKind};
use crate::TerminalSize;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

pub const ASCIICAST_VERSION: u32 = 2;
pub const NAVOP_RECORDING_FORMAT_VERSION: u32 = 1;
pub const NAVOP_EVENT_STREAM: &str = "terminal_parser_input_v1";

pub const DEFAULT_MAX_RECORDING_HEADER_BYTES: usize = 64 * 1024;
pub const DEFAULT_MAX_SERIALIZED_RECORDING_EVENT_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_MAX_RECORDING_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const DEFAULT_MAX_DECODED_RECORDING_BYTES: u64 = 1024 * 1024 * 1024;

const OUTPUT_EVENT: &str = "o";
const INPUT_EVENT: &str = "i";
const RESIZE_EVENT: &str = "r";
const MARKER_EVENT: &str = "m";
const BASE64_OUTPUT_EVENT: &str = "x-navop-o64";
const BASE64_INPUT_EVENT: &str = "x-navop-i64";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingBackend {
    Local,
    Ssh,
    Serial,
}

/// Metadata accepted from the active session when a recording starts.
///
/// Authentication material, environment values, connection strings, and
/// command text are intentionally absent from this type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingMetadata {
    pub recording_id: String,
    pub session_id: String,
    pub backend: RecordingBackend,
    pub application_version: String,
    pub started_at_unix_ms: u64,
    pub capture_input: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingHeaderMetadata {
    pub format_version: u32,
    pub recording_id: String,
    pub session_id: String,
    pub backend: RecordingBackend,
    pub application_version: String,
    pub started_at_unix_ms: u64,
    pub capture_input: bool,
    pub event_stream: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingHeader {
    pub version: u32,
    pub width: u16,
    pub height: u16,
    pub timestamp: u64,
    pub navop: RecordingHeaderMetadata,
}

impl RecordingHeader {
    pub(crate) fn new(metadata: RecordingMetadata, initial_size: TerminalSize) -> Self {
        Self {
            version: ASCIICAST_VERSION,
            width: initial_size.cols,
            height: initial_size.rows,
            timestamp: metadata.started_at_unix_ms / 1_000,
            navop: RecordingHeaderMetadata {
                format_version: NAVOP_RECORDING_FORMAT_VERSION,
                recording_id: metadata.recording_id,
                session_id: metadata.session_id,
                backend: metadata.backend,
                application_version: metadata.application_version,
                started_at_unix_ms: metadata.started_at_unix_ms,
                capture_input: metadata.capture_input,
                event_stream: NAVOP_EVENT_STREAM.to_string(),
            },
        }
    }

    pub(crate) fn validate(&self) -> Result<(), RecordingFileError> {
        if self.version != ASCIICAST_VERSION {
            return Err(RecordingFileError::UnknownAsciicastVersion(self.version));
        }
        if self.navop.format_version != NAVOP_RECORDING_FORMAT_VERSION {
            return Err(RecordingFileError::UnknownNavopVersion(
                self.navop.format_version,
            ));
        }
        if self.width == 0 || self.height == 0 {
            return Err(RecordingFileError::InvalidHeader(
                "terminal dimensions must be non-zero".to_string(),
            ));
        }
        if self.navop.recording_id.is_empty() || self.navop.session_id.is_empty() {
            return Err(RecordingFileError::InvalidHeader(
                "recording_id and session_id must be non-empty".to_string(),
            ));
        }
        if self.navop.application_version.is_empty() {
            return Err(RecordingFileError::InvalidHeader(
                "application_version must be non-empty".to_string(),
            ));
        }
        if self.timestamp != self.navop.started_at_unix_ms / 1_000 {
            return Err(RecordingFileError::InvalidHeader(
                "timestamp does not match started_at_unix_ms".to_string(),
            ));
        }
        if self.navop.event_stream != NAVOP_EVENT_STREAM {
            return Err(RecordingFileError::InvalidHeader(format!(
                "unsupported event stream: {}",
                self.navop.event_stream
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingFileLimit {
    HeaderBytes,
    EventBytes,
    FileBytes,
    EventCount,
    DecodedPayloadBytes,
}

impl fmt::Display for RecordingFileLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::HeaderBytes => "header_bytes",
            Self::EventBytes => "event_bytes",
            Self::FileBytes => "file_bytes",
            Self::EventCount => "event_count",
            Self::DecodedPayloadBytes => "decoded_payload_bytes",
        })
    }
}

#[derive(Clone, Debug)]
pub struct RecordingFileLimits {
    pub max_header_bytes: usize,
    pub max_serialized_event_bytes: usize,
    pub max_file_bytes: u64,
    pub max_events: u64,
    pub max_decoded_payload_bytes: u64,
}

impl Default for RecordingFileLimits {
    fn default() -> Self {
        Self {
            max_header_bytes: DEFAULT_MAX_RECORDING_HEADER_BYTES,
            max_serialized_event_bytes: DEFAULT_MAX_SERIALIZED_RECORDING_EVENT_BYTES,
            max_file_bytes: DEFAULT_MAX_RECORDING_FILE_BYTES,
            max_events: DEFAULT_MAX_RECORDING_EVENTS,
            max_decoded_payload_bytes: DEFAULT_MAX_DECODED_RECORDING_BYTES,
        }
    }
}

#[derive(Debug)]
pub enum RecordingFileError {
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Json(serde_json::Error),
    InvalidConfig(String),
    InvalidHeader(String),
    InvalidEvent {
        line: u64,
        reason: String,
    },
    UnknownAsciicastVersion(u32),
    UnknownNavopVersion(u32),
    LimitReached(RecordingFileLimit),
    InputCaptureDisabled,
    FinalPathExists(PathBuf),
    InvalidFinalPath(PathBuf),
    InvalidPartialPath(PathBuf),
    FileChangedDuringRecovery,
    NotOpen,
}

impl RecordingFileError {
    pub(crate) fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    pub(crate) fn invalid_event(line: u64, reason: impl Into<String>) -> Self {
        Self::InvalidEvent {
            line,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for RecordingFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, source } => {
                write!(formatter, "recording {operation} failed: {source}")
            }
            Self::Json(source) => write!(formatter, "recording JSON failed: {source}"),
            Self::InvalidConfig(reason) => {
                write!(formatter, "invalid recording file config: {reason}")
            }
            Self::InvalidHeader(reason) => write!(formatter, "invalid recording header: {reason}"),
            Self::InvalidEvent { line, reason } => {
                write!(
                    formatter,
                    "invalid recording event at line {line}: {reason}"
                )
            }
            Self::UnknownAsciicastVersion(version) => {
                write!(formatter, "unsupported asciicast version: {version}")
            }
            Self::UnknownNavopVersion(version) => {
                write!(formatter, "unsupported Navop recording version: {version}")
            }
            Self::LimitReached(limit) => {
                write!(formatter, "recording {limit} limit reached")
            }
            Self::InputCaptureDisabled => {
                formatter.write_str("recording input capture is disabled")
            }
            Self::FinalPathExists(path) => {
                write!(
                    formatter,
                    "recording final path already exists: {}",
                    path.display()
                )
            }
            Self::InvalidFinalPath(path) => {
                write!(
                    formatter,
                    "invalid recording final path: {}",
                    path.display()
                )
            }
            Self::InvalidPartialPath(path) => {
                write!(
                    formatter,
                    "invalid recording partial path: {}",
                    path.display()
                )
            }
            Self::FileChangedDuringRecovery => {
                formatter.write_str("recording partial changed during recovery")
            }
            Self::NotOpen => formatter.write_str("recording file is not open"),
        }
    }
}

impl std::error::Error for RecordingFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(source) => Some(source),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for RecordingFileError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub(crate) fn encode_header(header: &RecordingHeader) -> Result<Vec<u8>, RecordingFileError> {
    header.validate()?;
    serde_json::to_vec(header).map_err(RecordingFileError::from)
}

pub(crate) fn decode_header(bytes: &[u8]) -> Result<RecordingHeader, RecordingFileError> {
    let header: RecordingHeader =
        serde_json::from_slice(bytes).map_err(RecordingFileError::from)?;
    header.validate()?;
    Ok(header)
}

pub(crate) fn encode_event(
    event: &RecordingEvent,
    capture_input: bool,
) -> Result<Vec<u8>, RecordingFileError> {
    let elapsed = event.elapsed.as_secs_f64();
    let (event_type, data) = match &event.kind {
        RecordingEventKind::Output(bytes) => encode_bytes(bytes, OUTPUT_EVENT, BASE64_OUTPUT_EVENT),
        RecordingEventKind::Input(bytes) => {
            if !capture_input {
                return Err(RecordingFileError::InputCaptureDisabled);
            }
            encode_bytes(bytes, INPUT_EVENT, BASE64_INPUT_EVENT)
        }
        RecordingEventKind::Resize(size) => {
            if size.cols == 0 || size.rows == 0 {
                return Err(RecordingFileError::invalid_event(
                    0,
                    "resize dimensions must be non-zero",
                ));
            }
            (
                RESIZE_EVENT,
                format!("{}x{}", u32::from(size.cols), u32::from(size.rows)),
            )
        }
        RecordingEventKind::Marker(marker) => (MARKER_EVENT, marker.clone()),
    };
    serde_json::to_vec(&(elapsed, event_type, data)).map_err(RecordingFileError::from)
}

pub(crate) fn decode_event(bytes: &[u8], line: u64) -> Result<RecordingEvent, RecordingFileError> {
    let (elapsed, event_type, data): (f64, String, String) = serde_json::from_slice(bytes)
        .map_err(|error| RecordingFileError::invalid_event(line, error.to_string()))?;
    let elapsed = Duration::try_from_secs_f64(elapsed).map_err(|error| {
        RecordingFileError::invalid_event(line, format!("invalid timestamp: {error}"))
    })?;
    let kind = match event_type.as_str() {
        OUTPUT_EVENT => RecordingEventKind::Output(data.into_bytes()),
        INPUT_EVENT => RecordingEventKind::Input(data.into_bytes()),
        BASE64_OUTPUT_EVENT => RecordingEventKind::Output(
            BASE64
                .decode(data)
                .map_err(|error| RecordingFileError::invalid_event(line, error.to_string()))?,
        ),
        BASE64_INPUT_EVENT => RecordingEventKind::Input(
            BASE64
                .decode(data)
                .map_err(|error| RecordingFileError::invalid_event(line, error.to_string()))?,
        ),
        RESIZE_EVENT => RecordingEventKind::Resize(decode_resize(&data, line)?),
        MARKER_EVENT => RecordingEventKind::Marker(data),
        _ => {
            return Err(RecordingFileError::invalid_event(
                line,
                format!("unsupported event type: {event_type}"),
            ));
        }
    };
    Ok(RecordingEvent { elapsed, kind })
}

fn encode_bytes<'a>(
    bytes: &'a [u8],
    utf8_event: &'static str,
    base64_event: &'static str,
) -> (&'static str, String) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (utf8_event, text.to_string()),
        Err(_) => (base64_event, BASE64.encode(bytes)),
    }
}

fn decode_resize(data: &str, line: u64) -> Result<TerminalSize, RecordingFileError> {
    let Some((cols, rows)) = data.split_once('x') else {
        return Err(RecordingFileError::invalid_event(
            line,
            "resize must use COLSxROWS",
        ));
    };
    let cols = cols
        .parse::<u16>()
        .map_err(|error| RecordingFileError::invalid_event(line, error.to_string()))?;
    let rows = rows
        .parse::<u16>()
        .map_err(|error| RecordingFileError::invalid_event(line, error.to_string()))?;
    if cols == 0 || rows == 0 {
        return Err(RecordingFileError::invalid_event(
            line,
            "resize dimensions must be non-zero",
        ));
    }
    Ok(TerminalSize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })
}
