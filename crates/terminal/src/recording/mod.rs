mod asciicast;
mod controller;
mod model;
mod playback;
mod recorder;
mod recovery;
mod runtime;

pub use asciicast::{
    ASCIICAST_VERSION, DEFAULT_MAX_DECODED_RECORDING_BYTES, DEFAULT_MAX_RECORDING_FILE_BYTES,
    DEFAULT_MAX_RECORDING_HEADER_BYTES, DEFAULT_MAX_SERIALIZED_RECORDING_EVENT_BYTES,
    NAVOP_EVENT_STREAM, NAVOP_RECORDING_FORMAT_VERSION, RecordingBackend, RecordingFileError,
    RecordingFileLimit, RecordingFileLimits, RecordingHeader, RecordingHeaderMetadata,
    RecordingMetadata,
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
pub use recorder::{
    RecordingFileConfig, RecordingFileState, RecordingFileTransition, RecordingFileWriter,
    partial_recording_path,
};
pub use recovery::{
    ParsedRecording, RecordingCompleteness, RecordingRecovery, read_recording,
    recover_partial_recording,
};
#[cfg(test)]
use runtime::RecordingWorkerTestGate;
pub use runtime::{
    RecordingQueueLimits, RecordingQueueSnapshot, RecordingRuntime, RecordingRuntimeConfig,
    RecordingRuntimeError, RecordingSnapshot, RecordingStartRequest, RecordingTap,
    RecordingTapOutcome,
};

#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod playback_tests;
#[cfg(test)]
mod runtime_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
