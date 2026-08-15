mod asciicast;
mod controller;
mod model;
mod playback;
mod playback_surface;
mod recorder;
mod recovery;
mod runtime;
mod session_favorites;
mod session_log;
mod text_export;

pub use asciicast::{
    ASCIICAST_VERSION, DEFAULT_MAX_DECODED_RECORDING_BYTES, DEFAULT_MAX_RECORDING_FILE_BYTES,
    DEFAULT_MAX_RECORDING_HEADER_BYTES, DEFAULT_MAX_SERIALIZED_RECORDING_EVENT_BYTES,
    NAVOP_EVENT_STREAM, NAVOP_RECORDING_FORMAT_VERSION, RecordingBackend, RecordingFileError,
    RecordingFileLimit, RecordingFileLimits, RecordingHeader, RecordingHeaderMetadata,
    RecordingMetadata, RecordingSessionMetadata,
};
pub use controller::RecordingController;
pub use model::{
    RecordingConfig, RecordingEvent, RecordingEventKind, RecordingFailure, RecordingLimit,
    RecordingLimits, RecordingState, RecordingTransition,
};
pub use playback::{
    DEFAULT_MAX_PLAYBACK_INDEXED_EVENTS, DEFAULT_MAX_PLAYBACK_INDEXED_TEXT_BYTES,
    DEFAULT_MAX_PLAYBACK_SEARCH_QUERY_BYTES, DEFAULT_MAX_PLAYBACK_SEARCH_RESULTS,
    DEFAULT_MAX_PLAYBACK_SEARCH_SNIPPET_BYTES, MAX_PLAYBACK_SPEED, MIN_PLAYBACK_SPEED,
    RecordingPlayback, RecordingPlaybackError, RecordingPlaybackLimits,
    RecordingPlaybackSearchIndexStatus, RecordingPlaybackSearchKind, RecordingPlaybackSearchMatch,
    RecordingPlaybackSearchResults, RecordingPlaybackState, RecordingPlaybackTransition,
};
pub(crate) use playback_surface::TerminalPlaybackRuntime;
pub use recorder::{
    RecordingFileConfig, RecordingFileState, RecordingFileTransition, RecordingFileWriter,
    partial_recording_path,
};
pub use recovery::{
    ParsedRecording, RecordingCompleteness, RecordingRecovery, read_recording,
    read_recording_for_playback, recover_partial_recording,
};
#[cfg(test)]
use runtime::RecordingWorkerTestGate;
pub use runtime::{
    RecordingQueueLimits, RecordingQueueSnapshot, RecordingRuntime, RecordingRuntimeConfig,
    RecordingRuntimeError, RecordingSnapshot, RecordingStartRequest, RecordingTap,
    RecordingTapOutcome,
};
pub use session_favorites::{
    SessionLogFavorites, load_session_log_favorites, save_session_log_favorites,
};
pub use session_log::{
    SESSION_LOGS_DIRECTORY, SessionLogCatalog, SessionLogEntry, SessionLogScanIssue,
    scan_session_logs, session_log_path, session_logs_directory,
};
pub use text_export::{RecordingTextExport, export_recording_text};

#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod playback_surface_tests;
#[cfg(test)]
mod playback_tests;
#[cfg(test)]
mod recorder_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod runtime_tests;
#[cfg(test)]
mod session_log_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_export_tests;
