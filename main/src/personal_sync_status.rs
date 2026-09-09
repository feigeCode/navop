use one_core::cloud_sync::personal::{SyncStoreError, SyncStoreHealth};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PersonalSyncRuntimeStatus {
    #[default]
    Disabled,
    Ready {
        health: SyncStoreHealth,
        message: Option<String>,
    },
    Syncing,
    Failed {
        health: SyncStoreHealth,
        message: String,
    },
}

impl PersonalSyncRuntimeStatus {
    pub fn from_error(error: SyncStoreError) -> Self {
        let health = health_from_error(&error);
        Self::Failed {
            health,
            message: error.to_string(),
        }
    }

    pub fn failed(message: &str) -> Self {
        Self::Failed {
            health: SyncStoreHealth::NotConfigured,
            message: message.to_string(),
        }
    }
}

fn health_from_error(error: &SyncStoreError) -> SyncStoreHealth {
    match error {
        SyncStoreError::NotConfigured => SyncStoreHealth::NotConfigured,
        SyncStoreError::DirectoryUnavailable(_) => SyncStoreHealth::DirectoryUnavailable,
        SyncStoreError::SchemaUnsupported { .. } => SyncStoreHealth::SchemaUnsupported,
        SyncStoreError::GitAuthRequired => SyncStoreHealth::GitAuthRequired,
        SyncStoreError::GitMergeConflict => SyncStoreHealth::GitMergeConflict,
        _ => SyncStoreHealth::PausedAfterRepeatedFailures,
    }
}
