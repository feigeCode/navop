use std::fmt;
use std::sync::{Arc, RwLock};

use crate::cloud_sync::{CloudSyncData, CloudSyncService, Team};
use crate::crypto;
use crate::storage::{TeamKeyCache, TeamKeyCacheRepository, now};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamKeyStatus {
    Missing,
    Cached,
    Unlocked,
    VersionMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamKeyError {
    MissingVerification,
    MissingPersonalKey,
    MissingTeamKey,
    VersionMismatch,
    InvalidTeamKey,
    InvalidCachedKey,
    Storage(String),
    ServiceLock,
}

impl fmt::Display for TeamKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVerification => write!(f, "团队尚未初始化密钥验证数据"),
            Self::MissingPersonalKey => write!(f, "请先解锁个人主密钥"),
            Self::MissingTeamKey => write!(f, "请先设置团队密钥"),
            Self::VersionMismatch => write!(f, "团队密钥版本已变更，请重新输入"),
            Self::InvalidTeamKey => write!(f, "团队密钥错误"),
            Self::InvalidCachedKey => write!(f, "本地缓存的团队密钥无法解密或验证"),
            Self::Storage(error) => write!(f, "团队密钥缓存读写失败: {error}"),
            Self::ServiceLock => write!(f, "同步服务锁获取失败"),
        }
    }
}

impl std::error::Error for TeamKeyError {}

pub struct TeamKeyManager {
    repo: TeamKeyCacheRepository,
    service: Arc<RwLock<CloudSyncService>>,
}

pub struct TeamKeyRotation {
    pub team: Team,
    pub records: Vec<CloudSyncData>,
}

impl TeamKeyManager {
    pub fn new(repo: TeamKeyCacheRepository, service: Arc<RwLock<CloudSyncService>>) -> Self {
        Self { repo, service }
    }

    pub fn save_verified_team_key(
        &self,
        team: &Team,
        team_key: &str,
        personal_key: &str,
    ) -> Result<TeamKeyStatus, TeamKeyError> {
        self.verify_team_key(team, team_key)?;
        let encrypted = crypto::encrypt_with_key(team_key, personal_key);
        let cache = self.cache_for_team(team, Some(encrypted), Some(now()))?;
        self.repo
            .upsert(&cache)
            .map_err(|e| TeamKeyError::Storage(e.to_string()))?;
        self.service
            .write()
            .map_err(|_| TeamKeyError::ServiceLock)?
            .set_team_key(&team.id, team_key.to_string());
        Ok(TeamKeyStatus::Unlocked)
    }

    pub fn save_key_for_cached_team(
        &self,
        team_id: &str,
        team_key: &str,
        personal_key: &str,
    ) -> Result<TeamKeyStatus, TeamKeyError> {
        let cache = self
            .repo
            .get(team_id)
            .map_err(|e| TeamKeyError::Storage(e.to_string()))?
            .ok_or_else(|| TeamKeyError::Storage("团队缓存不存在".to_string()))?;
        let team = Team {
            id: cache.team_id,
            name: cache.team_name,
            owner_id: String::new(),
            description: None,
            key_verification: cache.key_verification,
            key_version: cache.key_version,
            created_at: 0,
            updated_at: cache.updated_at,
        };
        self.save_verified_team_key(&team, team_key, personal_key)
    }

    pub fn load_cached_team_key(
        &self,
        team: &Team,
        personal_key: &str,
    ) -> Result<TeamKeyStatus, TeamKeyError> {
        let Some(cache) = self
            .repo
            .get(&team.id)
            .map_err(|e| TeamKeyError::Storage(e.to_string()))?
        else {
            return Ok(TeamKeyStatus::Missing);
        };
        if cache.key_version != team.key_version {
            self.service
                .write()
                .map_err(|_| TeamKeyError::ServiceLock)?
                .remove_team_key(&team.id);
            return Ok(TeamKeyStatus::VersionMismatch);
        }
        let Some(encrypted) = cache.encrypted_team_key else {
            return Ok(TeamKeyStatus::Missing);
        };
        let team_key = crypto::decrypt_with_key(&encrypted, personal_key)
            .map_err(|_| TeamKeyError::InvalidCachedKey)?;
        self.verify_team_key(team, &team_key)
            .map_err(|_| TeamKeyError::InvalidCachedKey)?;
        self.service
            .write()
            .map_err(|_| TeamKeyError::ServiceLock)?
            .set_team_key(&team.id, team_key);
        Ok(TeamKeyStatus::Unlocked)
    }

    pub fn load_cached_team_key_by_id(
        &self,
        team_id: &str,
        personal_key: &str,
    ) -> Result<TeamKeyStatus, TeamKeyError> {
        let Some(cache) = self
            .repo
            .get(team_id)
            .map_err(|e| TeamKeyError::Storage(e.to_string()))?
        else {
            return Ok(TeamKeyStatus::Missing);
        };
        let team = Team {
            id: cache.team_id,
            name: cache.team_name,
            owner_id: String::new(),
            description: None,
            key_verification: cache.key_verification,
            key_version: cache.key_version,
            created_at: 0,
            updated_at: cache.updated_at,
        };
        self.load_cached_team_key(&team, personal_key)
    }

    pub fn load_cached_team_keys(
        &self,
        teams: &[Team],
        personal_key: &str,
    ) -> Vec<(String, Result<TeamKeyStatus, TeamKeyError>)> {
        teams
            .iter()
            .map(|team| {
                (
                    team.id.clone(),
                    self.load_cached_team_key(team, personal_key),
                )
            })
            .collect()
    }

    pub fn rotate_team_key_records(
        team: &Team,
        old_key: &str,
        new_key: &str,
        records: &[CloudSyncData],
    ) -> Result<TeamKeyRotation, TeamKeyError> {
        let verification = team
            .key_verification
            .as_deref()
            .ok_or(TeamKeyError::MissingVerification)?;
        if !crypto::verify_master_key(old_key, verification) {
            return Err(TeamKeyError::InvalidTeamKey);
        }

        let new_version = team.key_version.saturating_add(1);
        let service = CloudSyncService::new();
        let records = records
            .iter()
            .map(|record| {
                service
                    .re_encrypt_sync_data(record, old_key, new_key, new_version)
                    .map_err(|e| TeamKeyError::Storage(e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut team = team.clone();
        team.key_verification = Some(crypto::generate_key_verification(new_key));
        team.key_version = new_version;

        Ok(TeamKeyRotation { team, records })
    }

    pub fn forget_team_key(&self, team_id: &str) -> Result<(), TeamKeyError> {
        if let Some(cache) = self
            .repo
            .get(team_id)
            .map_err(|e| TeamKeyError::Storage(e.to_string()))?
        {
            self.repo
                .upsert(&TeamKeyCache {
                    encrypted_team_key: None,
                    last_verified_at: None,
                    ..cache
                })
                .map_err(|e| TeamKeyError::Storage(e.to_string()))?;
        }
        self.service
            .write()
            .map_err(|_| TeamKeyError::ServiceLock)?
            .remove_team_key(team_id);
        Ok(())
    }

    fn verify_team_key(&self, team: &Team, team_key: &str) -> Result<(), TeamKeyError> {
        let verification = team
            .key_verification
            .as_deref()
            .ok_or(TeamKeyError::MissingVerification)?;
        if crypto::verify_master_key(team_key, verification) {
            Ok(())
        } else {
            Err(TeamKeyError::InvalidTeamKey)
        }
    }

    fn cache_for_team(
        &self,
        team: &Team,
        encrypted_team_key: Option<String>,
        last_verified_at: Option<i64>,
    ) -> Result<TeamKeyCache, TeamKeyError> {
        let existing = self
            .repo
            .get(&team.id)
            .map_err(|e| TeamKeyError::Storage(e.to_string()))?;
        Ok(TeamKeyCache {
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            key_version: team.key_version,
            key_verification: team.key_verification.clone(),
            encrypted_team_key,
            last_verified_at,
            updated_at: team.updated_at,
            role: existing.and_then(|cache| cache.role),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, RwLock};

    use crate::cloud_sync::{
        CloudSyncData, CloudSyncService, Team, TeamKeyManager, TeamKeyStatus, data_type,
    };
    use crate::crypto;
    use crate::storage::connection::SqliteConnection;
    use crate::storage::migration::run_migrations;
    use crate::storage::{TeamKeyCache, TeamKeyCacheRepository};

    static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_repo() -> TeamKeyCacheRepository {
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let db_path = std::env::temp_dir().join(format!(
            "onetcli-team-key-manager-{}-{}-{}.db",
            std::process::id(),
            unique_suffix(),
            counter
        ));
        let _ = std::fs::remove_file(&db_path);
        let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
        conn.with_connection(|conn| run_migrations(conn))
            .expect("run migrations");
        TeamKeyCacheRepository::new(conn)
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    }

    fn team(service: &CloudSyncService, version: u32) -> Team {
        Team {
            id: "team-1".to_string(),
            name: "Platform".to_string(),
            owner_id: "owner-1".to_string(),
            description: None,
            key_verification: Some(service.generate_team_key_verification("team-secret")),
            key_version: version,
            created_at: 100,
            updated_at: 200,
        }
    }

    fn manager(repo: TeamKeyCacheRepository) -> (TeamKeyManager, Arc<RwLock<CloudSyncService>>) {
        let service = Arc::new(RwLock::new(CloudSyncService::new()));
        (TeamKeyManager::new(repo, service.clone()), service)
    }

    #[test]
    fn save_verified_key_persists_encrypted_cache_and_unlocks_service() {
        let repo = test_repo();
        let verification_service = CloudSyncService::new();
        let team = team(&verification_service, 1);
        let (manager, service) = manager(repo.clone());

        let status = manager
            .save_verified_team_key(&team, "team-secret", "personal-secret")
            .expect("team key saves");

        assert_eq!(TeamKeyStatus::Unlocked, status);
        assert_eq!(
            Some("team-secret".to_string()),
            service
                .read()
                .expect("service lock")
                .get_team_key("team-1")
                .cloned()
        );

        let cache = repo
            .get("team-1")
            .expect("cache read")
            .expect("cache exists");
        assert_ne!(Some("team-secret".to_string()), cache.encrypted_team_key);
        assert_eq!(1, cache.key_version);
        assert!(cache.last_verified_at.is_some());
    }

    #[test]
    fn load_cached_key_unlocks_new_service() {
        let repo = test_repo();
        let verification_service = CloudSyncService::new();
        let team = team(&verification_service, 1);
        let (first_manager, _) = manager(repo.clone());
        first_manager
            .save_verified_team_key(&team, "team-secret", "personal-secret")
            .expect("team key saves");

        let (second_manager, second_service) = manager(repo);
        let status = second_manager
            .load_cached_team_key(&team, "personal-secret")
            .expect("cached key loads");

        assert_eq!(TeamKeyStatus::Unlocked, status);
        assert_eq!(
            Some("team-secret".to_string()),
            second_service
                .read()
                .expect("service lock")
                .get_team_key("team-1")
                .cloned()
        );
    }

    #[test]
    fn save_key_for_cached_team_uses_cached_verification() {
        let repo = test_repo();
        let verification_service = CloudSyncService::new();
        let team = team(&verification_service, 4);
        repo.upsert(&TeamKeyCache {
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            key_version: team.key_version,
            key_verification: team.key_verification.clone(),
            encrypted_team_key: None,
            last_verified_at: None,
            updated_at: team.updated_at,
            role: Some("member".to_string()),
        })
        .expect("cache seed");
        let (manager, service) = manager(repo.clone());

        let status = manager
            .save_key_for_cached_team("team-1", "team-secret", "personal-secret")
            .expect("team key saves");

        assert_eq!(TeamKeyStatus::Unlocked, status);
        assert!(
            repo.get("team-1")
                .expect("cache read")
                .expect("cache exists")
                .encrypted_team_key
                .is_some()
        );
        assert!(
            service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-1")
        );
    }

    #[test]
    fn load_cached_team_keys_restores_each_valid_team() {
        let repo = test_repo();
        let verification_service = CloudSyncService::new();
        let valid_team = team(&verification_service, 1);
        let missing_team = Team {
            id: "team-2".to_string(),
            name: "Empty".to_string(),
            owner_id: "owner-1".to_string(),
            description: None,
            key_verification: Some(verification_service.generate_team_key_verification("other")),
            key_version: 1,
            created_at: 100,
            updated_at: 200,
        };
        let (first_manager, _) = manager(repo.clone());
        first_manager
            .save_verified_team_key(&valid_team, "team-secret", "personal-secret")
            .expect("team key saves");
        let (second_manager, service) = manager(repo);

        let statuses = second_manager.load_cached_team_keys(
            &[valid_team.clone(), missing_team.clone()],
            "personal-secret",
        );

        assert_eq!(
            vec![
                ("team-1".to_string(), Ok(TeamKeyStatus::Unlocked)),
                ("team-2".to_string(), Ok(TeamKeyStatus::Missing)),
            ],
            statuses
        );
        assert!(
            service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-1")
        );
        assert!(
            !service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-2")
        );
    }

    #[test]
    fn load_cached_team_key_by_id_validates_cached_key() {
        let repo = test_repo();
        let verification_service = CloudSyncService::new();
        let team = team(&verification_service, 1);
        let (first_manager, _) = manager(repo.clone());
        first_manager
            .save_verified_team_key(&team, "team-secret", "personal-secret")
            .expect("team key saves");
        let (second_manager, service) = manager(repo);

        let status = second_manager
            .load_cached_team_key_by_id("team-1", "personal-secret")
            .expect("cached key loads");

        assert_eq!(TeamKeyStatus::Unlocked, status);
        assert!(
            service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-1")
        );
    }

    #[test]
    fn rotate_team_key_reencrypts_team_records_with_next_version() {
        let service = CloudSyncService::new();
        let team = team(&service, 3);
        let old_key = "team-secret";
        let new_key = "new-team-secret";
        let record = CloudSyncData {
            id: "record-1".to_string(),
            owner_id: "owner-1".to_string(),
            team_id: Some(team.id.clone()),
            data_type: data_type::CONNECTION.to_string(),
            encrypted_data: crypto::encrypt_with_key(r#"{"name":"db"}"#, old_key),
            key_version: team.key_version,
            checksum: "checksum".to_string(),
            version: 7,
            updated_at: 100,
            deleted_at: None,
        };

        let rotation = TeamKeyManager::rotate_team_key_records(&team, old_key, new_key, &[record])
            .expect("rotation succeeds");

        assert_eq!(4, rotation.team.key_version);
        assert_ne!(team.key_verification, rotation.team.key_verification);
        assert_eq!(1, rotation.records.len());
        let rotated = &rotation.records[0];
        assert_eq!(4, rotated.key_version);
        assert_eq!(7, rotated.version);
        let decrypted = crypto::decrypt_with_key(&rotated.encrypted_data, new_key)
            .expect("rotated data decrypts with new key");
        assert_eq!(r#"{"name":"db"}"#, decrypted);
        assert!(crypto::decrypt_with_key(&rotated.encrypted_data, old_key).is_err());
    }

    #[test]
    fn rotate_team_key_rejects_invalid_old_key() {
        let service = CloudSyncService::new();
        let team = team(&service, 3);
        let record = CloudSyncData {
            id: "record-1".to_string(),
            owner_id: "owner-1".to_string(),
            team_id: Some(team.id.clone()),
            data_type: data_type::CONNECTION.to_string(),
            encrypted_data: crypto::encrypt_with_key("{}", "team-secret"),
            key_version: team.key_version,
            checksum: "checksum".to_string(),
            version: 1,
            updated_at: 100,
            deleted_at: None,
        };

        let result =
            TeamKeyManager::rotate_team_key_records(&team, "wrong-key", "new-key", &[record]);

        assert!(result.is_err());
    }

    #[test]
    fn wrong_team_key_is_rejected_and_not_cached() {
        let repo = test_repo();
        let verification_service = CloudSyncService::new();
        let team = team(&verification_service, 1);
        let (manager, service) = manager(repo.clone());

        let result = manager.save_verified_team_key(&team, "wrong-key", "personal-secret");

        assert!(result.is_err());
        assert!(repo.get("team-1").expect("cache read").is_none());
        assert!(
            !service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-1")
        );
    }

    #[test]
    fn cached_key_with_old_version_reports_version_mismatch() {
        let repo = test_repo();
        let verification_service = CloudSyncService::new();
        let old_team = team(&verification_service, 1);
        let new_team = team(&verification_service, 2);
        let (manager, service) = manager(repo);
        manager
            .save_verified_team_key(&old_team, "team-secret", "personal-secret")
            .expect("team key saves");

        let status = manager
            .load_cached_team_key(&new_team, "personal-secret")
            .expect("cached key status resolves");

        assert_eq!(TeamKeyStatus::VersionMismatch, status);
        assert!(
            !service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-1")
        );
    }

    #[test]
    fn forget_team_key_preserves_team_metadata() {
        let repo = test_repo();
        repo.upsert(&TeamKeyCache {
            team_id: "team-1".to_string(),
            team_name: "Platform".to_string(),
            key_version: 1,
            key_verification: Some("verification".to_string()),
            encrypted_team_key: Some("encrypted".to_string()),
            last_verified_at: Some(123),
            updated_at: 200,
            role: Some("admin".to_string()),
        })
        .expect("cache seed");
        let (manager, service) = manager(repo.clone());
        service
            .write()
            .expect("service lock")
            .set_team_key("team-1", "team-secret".to_string());

        manager.forget_team_key("team-1").expect("forget key");

        let cache = repo
            .get("team-1")
            .expect("cache read")
            .expect("cache exists");
        assert_eq!("Platform", cache.team_name);
        assert_eq!(Some("admin".to_string()), cache.role);
        assert_eq!(None, cache.encrypted_team_key);
        assert_eq!(None, cache.last_verified_at);
        assert!(
            !service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-1")
        );
    }
}
