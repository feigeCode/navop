use one_core::cloud_sync::personal::{SyncStoreError, SyncStoreHealth};
use std::sync::atomic::{AtomicI64, Ordering};

/// 本会话内最近一次同步（任意路由）成功完成的 Unix 秒时间戳，供账户菜单展示相对时间。
static LAST_SYNC_COMPLETED_AT: AtomicI64 = AtomicI64::new(0);

pub fn note_sync_completed() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    LAST_SYNC_COMPLETED_AT.store(now, Ordering::Relaxed);
}

pub fn last_sync_completed_at() -> Option<i64> {
    match LAST_SYNC_COMPLETED_AT.load(Ordering::Relaxed) {
        0 => None,
        timestamp => Some(timestamp),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonalSyncRuntimeStatus {
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

impl Default for PersonalSyncRuntimeStatus {
    fn default() -> Self {
        Self::Disabled
    }
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
