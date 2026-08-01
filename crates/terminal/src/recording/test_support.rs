use super::{
    ParsedRecording, RecordingBackend, RecordingConfig, RecordingFileLimits, RecordingMetadata,
    RecordingRuntime, RecordingRuntimeConfig, RecordingStartRequest, RecordingTap, read_recording,
};
use crate::TerminalSize;
use std::path::PathBuf;
use tempfile::TempDir;

pub(crate) struct TestRecording {
    _directory: TempDir,
    final_path: PathBuf,
    runtime: RecordingRuntime,
}

impl TestRecording {
    pub(crate) fn start(backend: RecordingBackend, capture_input: bool) -> Self {
        let directory = tempfile::tempdir().expect("create recording test directory");
        let final_path = directory.path().join("session.cast");
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("start recording test runtime");
        runtime
            .start(RecordingStartRequest {
                final_path: final_path.clone(),
                metadata: RecordingMetadata {
                    recording_id: "test-recording".to_string(),
                    session_id: "test-session".to_string(),
                    backend,
                    application_version: "0.1.0-test".to_string(),
                    started_at_unix_ms: 1_700_000_000_123,
                    capture_input,
                },
                initial_size: TerminalSize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 640,
                    pixel_height: 480,
                },
                recording: RecordingConfig {
                    capture_input,
                    ..RecordingConfig::default()
                },
            })
            .expect("start recording test session");
        Self {
            _directory: directory,
            final_path,
            runtime,
        }
    }

    pub(crate) fn tap(&self) -> RecordingTap {
        self.runtime.tap()
    }

    pub(crate) fn finish(self) -> ParsedRecording {
        self.runtime.stop().expect("stop recording test session");
        let parsed = read_recording(&self.final_path, RecordingFileLimits::default())
            .expect("read recording test session");
        self.runtime
            .shutdown()
            .expect("shutdown recording test runtime");
        parsed
    }
}
