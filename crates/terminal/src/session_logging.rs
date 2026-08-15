use crate::TerminalSize;
use crate::recording::{
    RecordingBackend, RecordingConfig, RecordingMetadata, RecordingRuntimeError,
    RecordingSessionMetadata, RecordingStartRequest, session_log_path, session_logs_directory,
};
use std::env;
use std::path::PathBuf;

pub(crate) struct AutomaticSessionLogRequestInput {
    pub data_directory: PathBuf,
    pub backend: RecordingBackend,
    pub session_id: String,
    pub initial_size: TerminalSize,
    pub session: RecordingSessionMetadata,
    pub started_at_unix_ms: u64,
    pub recording_id: String,
}

pub(crate) fn build_automatic_session_log_request(
    input: AutomaticSessionLogRequestInput,
) -> Result<RecordingStartRequest, RecordingRuntimeError> {
    let final_path = session_log_path(
        session_logs_directory(input.data_directory),
        input.backend,
        input.started_at_unix_ms,
        &input.recording_id,
    )
    .ok_or_else(|| {
        RecordingRuntimeError::InvalidConfig(
            "automatic session log timestamp or recording ID is invalid".to_string(),
        )
    })?;

    Ok(RecordingStartRequest {
        final_path,
        metadata: RecordingMetadata {
            recording_id: input.recording_id,
            session_id: input.session_id,
            backend: input.backend,
            application_version: application_version(),
            started_at_unix_ms: input.started_at_unix_ms,
            capture_input: false,
            session: Some(input.session),
        },
        initial_size: input.initial_size,
        recording: output_only_recording_config(),
    })
}

pub(crate) fn application_version() -> String {
    option_env!("NAVOP_APPLICATION_VERSION")
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string()
}

pub(crate) fn output_only_recording_config() -> RecordingConfig {
    RecordingConfig {
        capture_input: false,
        ..RecordingConfig::default()
    }
}

pub(crate) fn local_recording_session_metadata() -> RecordingSessionMetadata {
    RecordingSessionMetadata {
        local_user: environment_identity(&["USER", "USERNAME"]),
        local_host: environment_identity(&["HOSTNAME", "COMPUTERNAME"]),
        ..RecordingSessionMetadata::default()
    }
}

pub(crate) fn ssh_recording_session_metadata(
    connection_id: Option<i64>,
    connection_name: String,
    remote_user: String,
    remote_host: String,
    remote_port: u16,
) -> RecordingSessionMetadata {
    RecordingSessionMetadata {
        connection_id,
        connection_name: non_empty(connection_name),
        remote_user: non_empty(remote_user),
        remote_host: non_empty(remote_host),
        remote_port: Some(remote_port),
        ..RecordingSessionMetadata::default()
    }
}

pub(crate) fn serial_recording_session_metadata(
    connection_id: Option<i64>,
    connection_name: String,
    serial_port: String,
) -> RecordingSessionMetadata {
    RecordingSessionMetadata {
        connection_id,
        connection_name: non_empty(connection_name),
        serial_port: non_empty(serial_port),
        ..RecordingSessionMetadata::default()
    }
}

fn environment_identity(names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| env::var(name).ok())
        .and_then(non_empty)
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}
