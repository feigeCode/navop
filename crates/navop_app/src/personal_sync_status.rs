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
        SyncStoreError::WebdavAuthFailed => SyncStoreHealth::WebdavAuthFailed,
        SyncStoreError::WebdavUnreachable(_) => SyncStoreHealth::WebdavUnreachable,
        // 锁被别的实例（另一台机器 / 另一个 navop 进程）占着**不是**存储故障：
        // 这一轮只是让路，下一轮会自动重试。落到 `_` 分支会被标成「多次失败后已暂停」，
        // 让用户以为同步坏了；标成 Ready、把原因放在详情文案里更准确。
        SyncStoreError::LockTimeout => SyncStoreHealth::Ready,
        _ => SyncStoreHealth::PausedAfterRepeatedFailures,
    }
}

#[cfg(test)]
mod tests {
    use one_core::cloud_sync::personal::{SyncStoreError, SyncStoreHealth};

    use super::{PersonalSyncRuntimeStatus, health_from_error};

    /// 锁被别的实例占着不是存储故障：落到「多次失败后已暂停」会让用户以为同步坏了。
    #[test]
    fn lock_timeout_is_not_reported_as_a_storage_failure() {
        assert_eq!(
            SyncStoreHealth::Ready,
            health_from_error(&SyncStoreError::LockTimeout)
        );

        match PersonalSyncRuntimeStatus::from_error(SyncStoreError::LockTimeout) {
            PersonalSyncRuntimeStatus::Failed { health, message } => {
                assert_eq!(SyncStoreHealth::Ready, health);
                assert!(message.contains("retry"), "文案要说清会自动重试：{message}");
            }
            other => panic!("期望 Failed 但 health 为 Ready，实际为 {other:?}"),
        }
    }

    #[test]
    fn other_errors_still_map_to_paused_after_repeated_failures() {
        assert_eq!(
            SyncStoreHealth::PausedAfterRepeatedFailures,
            health_from_error(&SyncStoreError::Io("boom".to_string()))
        );
    }
}
