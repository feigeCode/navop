use serde::{Deserialize, Serialize};
use std::fmt;

pub const APP_ID: &str = "onetcli";
pub const PERSONAL_PROFILE_ID: &str = "personal";
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalSyncManifest {
    pub schema_version: u32,
    pub app: String,
    pub profile_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl PersonalSyncManifest {
    pub fn validate(&self) -> Result<(), SyncStoreError> {
        if self.schema_version > SUPPORTED_SCHEMA_VERSION {
            return Err(SyncStoreError::SchemaUnsupported {
                found: self.schema_version,
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncTombstone {
    pub id: String,
    pub data_type: String,
    pub deleted_at: i64,
    pub version: u32,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStoreError {
    NotConfigured,
    DirectoryUnavailable(String),
    SchemaUnsupported { found: u32 },
    Conflict(String),
    LockTimeout,
    GitAuthRequired,
    GitMergeConflict,
    Io(String),
    Parse(String),
}

impl fmt::Display for SyncStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "personal sync is not configured"),
            Self::DirectoryUnavailable(message) => write!(f, "directory unavailable: {message}"),
            Self::SchemaUnsupported { found } => {
                write!(f, "unsupported personal sync schema: {found}")
            }
            Self::Conflict(message) => write!(f, "personal sync conflict: {message}"),
            Self::LockTimeout => write!(f, "personal sync lock timed out"),
            Self::GitAuthRequired => write!(f, "git authentication required"),
            Self::GitMergeConflict => write!(f, "git merge conflict"),
            Self::Io(message) => write!(f, "personal sync io error: {message}"),
            Self::Parse(message) => write!(f, "personal sync parse error: {message}"),
        }
    }
}

impl std::error::Error for SyncStoreError {}

impl From<std::io::Error> for SyncStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<serde_json::Error> for SyncStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Parse(error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStoreHealth {
    Ready,
    NotConfigured,
    DirectoryUnavailable,
    SchemaUnsupported,
    GitAuthRequired,
    GitMergeConflict,
    PausedAfterRepeatedFailures,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStoreStatus {
    pub health: SyncStoreHealth,
    pub message: Option<String>,
}

impl SyncStoreStatus {
    pub fn ready() -> Self {
        Self {
            health: SyncStoreHealth::Ready,
            message: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncDeviceId(pub String);

#[derive(Debug)]
pub struct SyncStoreLock {
    pub owner: SyncDeviceId,
}

impl SyncStoreHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotConfigured => "not_configured",
            Self::DirectoryUnavailable => "directory_unavailable",
            Self::SchemaUnsupported => "schema_unsupported",
            Self::GitAuthRequired => "git_auth_required",
            Self::GitMergeConflict => "git_merge_conflict",
            Self::PausedAfterRepeatedFailures => "paused_after_repeated_failures",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "ready" => Self::Ready,
            "directory_unavailable" => Self::DirectoryUnavailable,
            "schema_unsupported" => Self::SchemaUnsupported,
            "git_auth_required" => Self::GitAuthRequired,
            "git_merge_conflict" => Self::GitMergeConflict,
            "paused_after_repeated_failures" => Self::PausedAfterRepeatedFailures,
            _ => Self::NotConfigured,
        }
    }
}
