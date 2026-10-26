//! 主密钥变更后的云端重加密
//!
//! 本地轮换主密钥并不会让云端记录跟着变：业务 `updated_at` 没动，同步 planner
//! 会判定"已同步"而不重传，云端就一直留着旧密钥的密文，其他设备或重装恢复时
//! 必然解密失败。所以改完密钥必须主动遍历云端记录，逐条解密后用新密钥重新加密
//! 并回写（方案甲）。
//!
//! 两类同步共用同一套记录结构与同一套重加密逻辑：
//! - Navop Cloud Sync：`sync_data` 表，通过 `CloudApiClient` 读写；
//! - Personal Sync（Folder/Git/WebDAV）：`.onetcli-sync/records`，通过
//!   `PersonalSyncStore` 读写。
//!
//! 边界：团队记录（`team_id` 非空）由团队密钥加密，与主密钥无关，必须整条跳过
//! ——用主密钥去解团队记录只会得到 `DecryptionFailed`。

use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cloud_sync::client::CloudApiClient;
use crate::cloud_sync::models::{CloudSyncData, CloudUserConfig};
use crate::cloud_sync::personal::PersonalSyncStore;
use crate::cloud_sync::service::{CloudSyncService, SyncError};
use crate::crypto;

/// 云端重加密统计
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CloudReencryptStats {
    /// 拉取到的记录总数
    pub total: usize,
    /// 成功重加密并回写的条数
    pub re_encrypted: usize,
    /// 失败条数（解密失败或回写失败）
    pub failed: usize,
    /// 因属于团队数据而跳过的条数
    pub skipped_team: usize,
}

impl CloudReencryptStats {
    /// 是否存在未完成的部分（决定是否需要置 pending 标记）
    pub fn incomplete(&self) -> bool {
        self.failed > 0
    }
}

/// 只有个人记录受主密钥保护
fn is_personal_record(record: &CloudSyncData) -> bool {
    record.team_id.is_none()
}

/// 把一批云端记录从旧密钥重加密为新密钥（纯函数，不涉及 IO）
pub fn re_encrypt_records(
    service: &CloudSyncService,
    records: &[CloudSyncData],
    old_key: &str,
    new_key: &str,
    new_key_version: u32,
) -> (Vec<CloudSyncData>, CloudReencryptStats) {
    let mut re_encrypted_records = Vec::new();
    let mut stats = CloudReencryptStats {
        total: records.len(),
        ..CloudReencryptStats::default()
    };

    for record in records {
        if !is_personal_record(record) {
            stats.skipped_team += 1;
            continue;
        }
        match service.re_encrypt_sync_data(record, old_key, new_key, new_key_version) {
            Ok(re_encrypted) => {
                re_encrypted_records.push(re_encrypted);
                stats.re_encrypted += 1;
            }
            Err(error) => {
                stats.failed += 1;
                tracing::warn!("云端记录重加密失败（{}）: {}", record.id, error);
            }
        }
    }

    (re_encrypted_records, stats)
}

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// 生成新的云端用户配置（验证数据 + 递增后的版本号）
pub fn new_user_config(user_id: &str, new_key: &str, new_key_version: u32) -> CloudUserConfig {
    CloudUserConfig {
        user_id: user_id.to_string(),
        key_verification: crypto::generate_key_verification(new_key),
        key_version: new_key_version,
        updated_at: current_timestamp(),
    }
}

/// Navop Cloud Sync 云端重加密：拉取全量 → 重加密 → 逐条回写 → 上传新版本信息
///
/// 版本号与验证数据一并写入 `user_configs`，其他设备同步时先比对版本，就能区分
/// "云端主密钥被改过"和"自己输错了密钥"。
pub async fn reencrypt_navop_cloud(
    client: &Arc<dyn CloudApiClient>,
    service: &Arc<RwLock<CloudSyncService>>,
    old_key: &str,
    new_key: &str,
    new_key_version: u32,
    user_id: &str,
) -> Result<CloudReencryptStats, SyncError> {
    let records = client
        .list_sync_data(None, None, None)
        .await
        .map_err(|error| SyncError::NetworkError(format!("{error:?}")))?;

    let (re_encrypted, mut stats) = {
        let guard = service
            .read()
            .map_err(|_| SyncError::StorageError("云同步服务锁获取失败".to_string()))?;
        re_encrypt_records(&guard, &records, old_key, new_key, new_key_version)
    };

    for record in &re_encrypted {
        if let Err(error) = client.update_sync_data(record).await {
            stats.failed += 1;
            stats.re_encrypted = stats.re_encrypted.saturating_sub(1);
            tracing::warn!("云端记录重加密回写失败（{}）: {:?}", record.id, error);
        }
    }

    let config = new_user_config(user_id, new_key, new_key_version);
    client
        .save_user_config(&config)
        .await
        .map_err(|error| SyncError::NetworkError(format!("{error:?}")))?;

    // 用新配置重新解锁：既校验了新密钥，也把服务内的 key_version 抬到新版本
    if let Ok(mut guard) = service.write() {
        let _ = guard.unlock(new_key, &config);
    }

    Ok(stats)
}

/// Personal Sync 云端重加密：拉取全量 → 重加密 → 逐条写回
///
/// Folder / Git / WebDAV 三种后端都实现同一个 `PersonalSyncStore`，所以这里只
/// 写一遍；manifest 不携带验证数据，其他设备靠解密抽样判断是否需要 adopt 新密钥。
pub async fn reencrypt_personal<S: PersonalSyncStore>(
    store: &S,
    service: &CloudSyncService,
    old_key: &str,
    new_key: &str,
    new_key_version: u32,
) -> Result<CloudReencryptStats, crate::cloud_sync::personal::SyncStoreError> {
    let records = store.list_records(None, None).await?;
    let (re_encrypted, mut stats) =
        re_encrypt_records(service, &records, old_key, new_key, new_key_version);

    for record in &re_encrypted {
        if let Err(error) = store.upsert_record(record, Some(record.version)).await {
            stats.failed += 1;
            stats.re_encrypted = stats.re_encrypted.saturating_sub(1);
            tracing::warn!("个人同步记录重加密回写失败（{}）: {}", record.id, error);
        }
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::{CloudReencryptStats, is_personal_record, new_user_config, re_encrypt_records};
    use crate::cloud_sync::models::CloudSyncData;
    use crate::cloud_sync::service::CloudSyncService;
    use crate::crypto;

    fn record(id: &str, team_id: Option<&str>, plaintext: &str, key: &str) -> CloudSyncData {
        CloudSyncData {
            id: id.to_string(),
            owner_id: "user-1".to_string(),
            team_id: team_id.map(str::to_string),
            data_type: "connection".to_string(),
            encrypted_data: crypto::encrypt_with_key(plaintext, key),
            key_version: 1,
            checksum: "checksum".to_string(),
            version: 1,
            updated_at: 10,
            deleted_at: None,
        }
    }

    #[test]
    fn re_encryption_rewrites_personal_records_only() {
        let mut service = CloudSyncService::new();
        service.set_master_key_directly("new-key".to_string());
        let records = vec![
            record("personal-1", None, "secret-1", "old-key"),
            record("team-1", Some("team-1"), "team-secret", "team-data-key"),
        ];

        let (rewritten, stats) = re_encrypt_records(&service, &records, "old-key", "new-key", 2);

        assert_eq!(1, rewritten.len());
        assert_eq!("personal-1", rewritten[0].id);
        assert_eq!(2, rewritten[0].key_version);
        assert_eq!(
            "secret-1",
            crypto::decrypt_with_key(&rewritten[0].encrypted_data, "new-key")
                .expect("decrypt with new key")
        );
        assert!(crypto::decrypt_with_key(&rewritten[0].encrypted_data, "old-key").is_err());
        assert_eq!(
            CloudReencryptStats {
                total: 2,
                re_encrypted: 1,
                failed: 0,
                skipped_team: 1,
            },
            stats
        );
    }

    #[test]
    fn team_records_are_never_touched() {
        let team_record = record("team-1", Some("team-1"), "team-secret", "team-data-key");

        assert!(!is_personal_record(&team_record));
        assert!(is_personal_record(&record(
            "personal-1",
            None,
            "secret",
            "old-key"
        )));
    }

    #[test]
    fn a_record_encrypted_by_another_key_is_counted_as_failed() {
        let mut service = CloudSyncService::new();
        service.set_master_key_directly("new-key".to_string());
        let records = vec![record("personal-1", None, "secret", "unrelated-key")];

        let (rewritten, stats) = re_encrypt_records(&service, &records, "old-key", "new-key", 2);

        assert!(rewritten.is_empty());
        assert_eq!(1, stats.failed);
        assert!(stats.incomplete());
    }

    #[test]
    fn user_config_carries_the_new_verification_and_version() {
        let config = new_user_config("user-1", "brand-new-key", 5);

        assert_eq!("user-1", config.user_id);
        assert_eq!(5, config.key_version);
        assert!(crypto::verify_master_key(
            "brand-new-key",
            &config.key_verification
        ));
        assert!(!crypto::verify_master_key(
            "old-key",
            &config.key_verification
        ));
    }
}
