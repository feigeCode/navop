use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use sftp::PathMetadata;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteFileSnapshot {
    pub size: u64,
    pub modified_secs: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadDecision {
    Upload,
    Conflict,
    RemoteMissing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalEditSession {
    pub remote_path: String,
    pub local_path: PathBuf,
    pub opened_snapshot: RemoteFileSnapshot,
    pub last_uploaded_snapshot: RemoteFileSnapshot,
}

pub fn decide_upload(
    opened: RemoteFileSnapshot,
    current: Option<RemoteFileSnapshot>,
) -> UploadDecision {
    match current {
        Some(current) if current == opened => UploadDecision::Upload,
        Some(_) => UploadDecision::Conflict,
        None => UploadDecision::RemoteMissing,
    }
}

pub fn sanitized_file_name(name: &str) -> String {
    let base_name = Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("remote-file");
    let sanitized: String = base_name
        .chars()
        .map(|value| match value {
            '/' | '\\' | ':' | '\0' => '_',
            _ => value,
        })
        .collect();
    if sanitized.is_empty() {
        "remote-file".to_string()
    } else {
        sanitized
    }
}

pub fn session_temp_file(cache_root: &Path, session_id: &str, remote_path: &str) -> PathBuf {
    let safe_session_id = sanitized_file_name(session_id);
    cache_root
        .join(safe_session_id)
        .join(sanitized_file_name(remote_path))
}

pub(crate) fn snapshot_from_metadata(metadata: &PathMetadata) -> RemoteFileSnapshot {
    RemoteFileSnapshot {
        size: metadata.size,
        modified_secs: metadata
            .modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        RemoteFileSnapshot, UploadDecision, decide_upload, sanitized_file_name, session_temp_file,
    };

    #[test]
    fn unchanged_remote_snapshot_allows_upload() {
        let opened = RemoteFileSnapshot {
            size: 12,
            modified_secs: 100,
        };

        assert_eq!(UploadDecision::Upload, decide_upload(opened, Some(opened)));
    }

    #[test]
    fn changed_remote_snapshot_requires_conflict_prompt() {
        let opened = RemoteFileSnapshot {
            size: 12,
            modified_secs: 100,
        };
        let current = RemoteFileSnapshot {
            size: 13,
            modified_secs: 101,
        };

        assert_eq!(
            UploadDecision::Conflict,
            decide_upload(opened, Some(current))
        );
    }

    #[test]
    fn missing_remote_snapshot_requires_missing_remote_prompt() {
        let opened = RemoteFileSnapshot {
            size: 12,
            modified_secs: 100,
        };

        assert_eq!(UploadDecision::RemoteMissing, decide_upload(opened, None));
    }

    #[test]
    fn temp_file_keeps_extension_and_stays_inside_session_directory() {
        let path = session_temp_file(Path::new("/tmp/cache"), "session-1", "/etc/app.toml");

        assert_eq!(Path::new("/tmp/cache/session-1/app.toml"), path);
        assert_eq!("name.rs", sanitized_file_name("../unsafe/name.rs"));
    }
}
