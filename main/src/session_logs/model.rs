use chrono::{Local, TimeZone};
use std::path::Path;
use std::time::Duration;
use terminal::recording::{RecordingBackend, SessionLogEntry};

pub(super) fn session_log_matches(entry: &SessionLogEntry, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }

    searchable_fields(entry)
        .into_iter()
        .any(|field| field.to_lowercase().contains(&query))
}

pub(super) fn format_started_at(started_at_unix_ms: u64) -> String {
    let Ok(timestamp) = i64::try_from(started_at_unix_ms) else {
        return started_at_unix_ms.to_string();
    };
    Local
        .timestamp_millis_opt(timestamp)
        .single()
        .map(|started_at| started_at.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| started_at_unix_ms.to_string())
}

pub(super) fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

pub(super) fn backend_label(backend: RecordingBackend) -> &'static str {
    match backend {
        RecordingBackend::Local => "local",
        RecordingBackend::Ssh => "ssh",
        RecordingBackend::Serial => "serial",
    }
}

pub(super) fn local_identity(entry: &SessionLogEntry) -> Option<String> {
    let session = entry.header.navop.session.as_ref()?;
    user_host(session.local_user.as_deref(), session.local_host.as_deref())
}

pub(super) fn remote_identity(entry: &SessionLogEntry) -> Option<String> {
    let session = entry.header.navop.session.as_ref()?;
    let identity = user_host(
        session.remote_user.as_deref(),
        session.remote_host.as_deref(),
    );
    match (identity, session.remote_port) {
        (Some(identity), Some(port)) => Some(format!("{identity}:{port}")),
        (Some(identity), None) => Some(identity),
        (None, Some(port)) => Some(port.to_string()),
        (None, None) => None,
    }
}

pub(super) fn exported_text_base_name(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recording");
    let base = name
        .strip_suffix(".cast.partial")
        .or_else(|| name.strip_suffix(".cast"))
        .unwrap_or(name);
    format!("{}.txt", if base.is_empty() { "recording" } else { base })
}

fn searchable_fields(entry: &SessionLogEntry) -> Vec<String> {
    let mut fields = vec![
        backend_label(entry.header.navop.backend).to_string(),
        entry.header.navop.recording_id.clone(),
        format_started_at(entry.header.navop.started_at_unix_ms),
        entry
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
    ];
    if let Some(session) = entry.header.navop.session.as_ref() {
        extend_session_fields(&mut fields, session);
    }
    fields
}

fn extend_session_fields(
    fields: &mut Vec<String>,
    session: &terminal::recording::RecordingSessionMetadata,
) {
    fields.extend(
        [
            session.connection_name.clone(),
            session.local_user.clone(),
            session.local_host.clone(),
            session.remote_user.clone(),
            session.remote_host.clone(),
            session.remote_port.map(|port| port.to_string()),
            session.serial_port.clone(),
        ]
        .into_iter()
        .flatten(),
    );
    fields.extend(user_host(
        session.local_user.as_deref(),
        session.local_host.as_deref(),
    ));
    fields.extend(remote_identity_parts(
        session.remote_user.as_deref(),
        session.remote_host.as_deref(),
        session.remote_port,
    ));
}

fn remote_identity_parts(
    user: Option<&str>,
    host: Option<&str>,
    port: Option<u16>,
) -> Option<String> {
    match (user_host(user, host), port) {
        (Some(identity), Some(port)) => Some(format!("{identity}:{port}")),
        (Some(identity), None) => Some(identity),
        (None, Some(port)) => Some(port.to_string()),
        (None, None) => None,
    }
}

fn user_host(user: Option<&str>, host: Option<&str>) -> Option<String> {
    match (user, host) {
        (Some(user), Some(host)) => Some(format!("{user}@{host}")),
        (Some(user), None) => Some(user.to_string()),
        (None, Some(host)) => Some(host.to_string()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use terminal::recording::{
        ASCIICAST_VERSION, NAVOP_EVENT_STREAM, NAVOP_RECORDING_FORMAT_VERSION,
        RecordingArtifactKind, RecordingCompleteness, RecordingHeader, RecordingHeaderMetadata,
        RecordingSessionMetadata,
    };

    fn entry(backend: RecordingBackend) -> SessionLogEntry {
        SessionLogEntry {
            path: PathBuf::from("20260814-160400-000-ssh-recording-1.cast"),
            header: RecordingHeader {
                version: ASCIICAST_VERSION,
                width: 120,
                height: 40,
                timestamp: 1_765_000_000,
                navop: RecordingHeaderMetadata {
                    format_version: NAVOP_RECORDING_FORMAT_VERSION,
                    recording_id: "recording-1".to_string(),
                    session_id: "session-1".to_string(),
                    backend,
                    artifact_kind: RecordingArtifactKind::SessionLog,
                    application_version: "0.10.7".to_string(),
                    started_at_unix_ms: 1_765_000_000_000,
                    capture_input: false,
                    event_stream: NAVOP_EVENT_STREAM.to_string(),
                    session: Some(RecordingSessionMetadata {
                        connection_name: Some("Production Shell".to_string()),
                        local_user: Some("hufei".to_string()),
                        local_host: Some("macbook.local".to_string()),
                        remote_user: Some("root".to_string()),
                        remote_host: Some("10.2.4.53".to_string()),
                        remote_port: Some(22),
                        serial_port: Some("/dev/ttyUSB0".to_string()),
                        ..RecordingSessionMetadata::default()
                    }),
                },
            },
            duration: Duration::from_secs(65),
            completeness: RecordingCompleteness::Complete,
            favorite: false,
        }
    }

    #[test]
    fn matching_is_case_insensitive_across_session_identity_fields() {
        let ssh = entry(RecordingBackend::Ssh);

        assert!(session_log_matches(&ssh, "production"));
        assert!(session_log_matches(&ssh, "ROOT@10.2.4.53"));
        assert!(session_log_matches(&ssh, "hufei@macbook.local"));
        assert!(session_log_matches(&ssh, "ttyusb0"));
        assert!(session_log_matches(&ssh, "RECORDING-1"));
        assert!(session_log_matches(&ssh, "20260814-160400"));
        assert!(!session_log_matches(&ssh, "unrelated"));
    }

    #[test]
    fn duration_uses_compact_hour_minute_second_format() {
        assert_eq!("0s", format_duration(Duration::ZERO));
        assert_eq!("59s", format_duration(Duration::from_secs(59)));
        assert_eq!("1m 05s", format_duration(Duration::from_secs(65)));
        assert_eq!("2h 03m 04s", format_duration(Duration::from_secs(7_384)));
    }

    #[test]
    fn identities_include_only_available_non_secret_metadata() {
        let ssh = entry(RecordingBackend::Ssh);
        assert_eq!(
            Some("hufei@macbook.local".to_string()),
            local_identity(&ssh)
        );
        assert_eq!(Some("root@10.2.4.53:22".to_string()), remote_identity(&ssh));
    }

    #[test]
    fn txt_export_name_removes_cast_and_partial_suffixes() {
        assert_eq!(
            "session.txt",
            exported_text_base_name(Path::new("session.cast"))
        );
        assert_eq!(
            "session.txt",
            exported_text_base_name(Path::new("session.cast.partial"))
        );
        assert_eq!(
            "recording.txt",
            exported_text_base_name(Path::new(".cast.partial"))
        );
    }
}
