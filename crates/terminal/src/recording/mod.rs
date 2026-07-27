mod asciicast;
mod controller;
mod model;
mod recorder;
mod recovery;

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
pub use recorder::{
    RecordingFileConfig, RecordingFileState, RecordingFileTransition, RecordingFileWriter,
    partial_recording_path,
};
pub use recovery::{
    ParsedRecording, RecordingCompleteness, RecordingRecovery, read_recording,
    recover_partial_recording,
};

#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod tests;
