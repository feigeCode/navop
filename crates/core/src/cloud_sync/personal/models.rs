use serde::{Deserialize, Serialize};

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
