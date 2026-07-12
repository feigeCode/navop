use std::fmt;
use std::sync::{Arc, RwLock};

use crate::cloud_sync::team_key_envelope::{
    TeamKeyEnvelopeError, TeamKeyKdfParams, create_team_key_envelope, is_team_key_envelope,
    unlock_team_key,
};
use crate::cloud_sync::{CloudAccountScope, CloudSyncData, CloudSyncService, Team};
use crate::crypto;
use crate::storage::{TeamKeyCache, TeamKeyCacheRepository, now};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamKeyCacheStatus {
    Missing,
    Cached,
    VersionMismatch,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamKeyLoadStatus {
    Unlocked,
    Missing,
    VersionMismatch,
    Invalid,
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
    KeyTooShort { minimum: usize },
}

impl fmt::Display for TeamKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVerification => write!(f, "团队尚未初始化密钥验证数据"),
            Self::MissingPersonalKey => write!(f, "请先解锁个人主密钥"),
            Self::MissingTeamKey => write!(f, "请先在 设置 > 团队密钥 中录入该团队密钥"),
            Self::VersionMismatch => {
                write!(f, "团队密钥版本已变更，请在 设置 > 团队密钥 中重新录入")
            }
            Self::InvalidTeamKey => write!(f, "团队密钥错误"),
            Self::InvalidCachedKey => write!(f, "本地缓存的团队密钥无法解密或验证"),
            Self::Storage(error) => write!(f, "团队密钥缓存读写失败: {error}"),
            Self::ServiceLock => write!(f, "同步服务锁获取失败"),
            Self::KeyTooShort { minimum } => write!(f, "团队密钥至少需要 {minimum} 个字符"),
        }
    }
}

impl std::error::Error for TeamKeyError {}

pub struct TeamKeyManager {
    repo: TeamKeyCacheRepository,
    service: Arc<RwLock<CloudSyncService>>,
    scope: CloudAccountScope,
}

pub struct TeamKeyRotation {
    pub team: Team,
    pub records: Vec<CloudSyncData>,
    pub data_key: String,
}

pub struct PreparedTeamKey {
    pub team: Team,
    pub data_key: String,
}

impl TeamKeyManager {
    pub fn new(
        repo: TeamKeyCacheRepository,
        service: Arc<RwLock<CloudSyncService>>,
        scope: CloudAccountScope,
    ) -> Self {
        Self {
            repo,
            service,
            scope,
        }
    }

    pub fn save_verified_team_key(
        &self,
        team: &Team,
        team_key: &str,
        personal_key: &str,
    ) -> Result<TeamKeyLoadStatus, TeamKeyError> {
        let unlocked = self.unlock_team_key(team, team_key)?;
        let encrypted = crypto::encrypt_with_key(team_key, personal_key);
        let cache = self.cache_for_team(team, Some(encrypted), Some(now()))?;
        self.repo
            .upsert(&cache)
            .map_err(|e| TeamKeyError::Storage(e.to_string()))?;
        self.service
            .write()
            .map_err(|_| TeamKeyError::ServiceLock)?
            .set_team_key(&team.id, unlocked.data_key);
        Ok(TeamKeyLoadStatus::Unlocked)
    }

    pub fn save_key_for_cached_team(
        &self,
        team_id: &str,
        team_key: &str,
        personal_key: &str,
    ) -> Result<TeamKeyLoadStatus, TeamKeyError> {
        let cache = self
            .repo
            .get(&self.scope, team_id)
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
    ) -> Result<TeamKeyLoadStatus, TeamKeyError> {
        let Some(cache) = self
            .repo
            .get(&self.scope, &team.id)
            .map_err(|e| TeamKeyError::Storage(e.to_string()))?
        else {
            self.clear_runtime_team_key(&team.id)?;
            return Ok(TeamKeyLoadStatus::Missing);
        };
        let (Some(encrypted), Some(cached_key_version)) = (
            cache.encrypted_team_key.as_deref(),
            cache.cached_key_version,
        ) else {
            self.clear_runtime_team_key(&team.id)?;
            return Ok(TeamKeyLoadStatus::Missing);
        };
        if cached_key_version != team.key_version {
            self.clear_runtime_team_key(&team.id)?;
            return Ok(TeamKeyLoadStatus::VersionMismatch);
        }
        let team_key = match crypto::decrypt_with_key(encrypted, personal_key) {
            Ok(team_key) => team_key,
            Err(_) => {
                self.clear_runtime_team_key(&team.id)?;
                return Err(TeamKeyError::InvalidCachedKey);
            }
        };
        let unlocked = match self.unlock_team_key(team, &team_key) {
            Ok(unlocked) => unlocked,
            Err(_) => {
                self.clear_runtime_team_key(&team.id)?;
                return Err(TeamKeyError::InvalidCachedKey);
            }
        };
        self.service
            .write()
            .map_err(|_| TeamKeyError::ServiceLock)?
            .set_team_key(&team.id, unlocked.data_key);
        Ok(TeamKeyLoadStatus::Unlocked)
    }

    pub fn load_cached_team_key_by_id(
        &self,
        team_id: &str,
        personal_key: &str,
    ) -> Result<TeamKeyLoadStatus, TeamKeyError> {
        let Some(cache) = self
            .repo
            .get(&self.scope, team_id)
            .map_err(|e| TeamKeyError::Storage(e.to_string()))?
        else {
            return Ok(TeamKeyLoadStatus::Missing);
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
    ) -> Vec<(String, Result<TeamKeyLoadStatus, TeamKeyError>)> {
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
        params: TeamKeyKdfParams,
    ) -> Result<TeamKeyRotation, TeamKeyError> {
        let verification = team
            .key_verification
            .as_deref()
            .ok_or(TeamKeyError::MissingVerification)?;
        let old_data_key = unlock_team_key(verification, old_key)
            .map_err(team_key_envelope_error)?
            .data_key;
        let created = create_team_key_envelope(new_key, params).map_err(team_key_envelope_error)?;

        let new_version = team.key_version.saturating_add(1);
        let service = CloudSyncService::new();
        let records = records
            .iter()
            .map(|record| {
                service
                    .re_encrypt_sync_data(record, &old_data_key, &created.data_key, new_version)
                    .map_err(|e| TeamKeyError::Storage(e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut team = team.clone();
        team.key_verification = Some(created.verification);
        team.key_version = new_version;

        Ok(TeamKeyRotation {
            team,
            records,
            data_key: created.data_key,
        })
    }

    pub fn prepare_initial_team_key(
        team: &Team,
        passphrase: &str,
        params: TeamKeyKdfParams,
    ) -> Result<PreparedTeamKey, TeamKeyError> {
        let created =
            create_team_key_envelope(passphrase, params).map_err(team_key_envelope_error)?;
        let mut team = team.clone();
        team.key_verification = Some(created.verification);
        team.key_version = team.key_version.max(1);
        Ok(PreparedTeamKey {
            team,
            data_key: created.data_key,
        })
    }

    pub fn forget_team_key(&self, team_id: &str) -> Result<(), TeamKeyError> {
        if let Some(cache) = self
            .repo
            .get(&self.scope, team_id)
            .map_err(|e| TeamKeyError::Storage(e.to_string()))?
        {
            self.repo
                .upsert(&TeamKeyCache {
                    cached_key_version: None,
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

    fn clear_runtime_team_key(&self, team_id: &str) -> Result<(), TeamKeyError> {
        self.service
            .write()
            .map_err(|_| TeamKeyError::ServiceLock)?
            .remove_team_key(team_id);
        Ok(())
    }

    fn unlock_team_key(
        &self,
        team: &Team,
        team_key: &str,
    ) -> Result<crate::cloud_sync::team_key_envelope::UnlockedTeamKey, TeamKeyError> {
        let verification = team
            .key_verification
            .as_deref()
            .ok_or(TeamKeyError::MissingVerification)?;
        unlock_team_key(verification, team_key).map_err(team_key_envelope_error)
    }

    fn cache_for_team(
        &self,
        team: &Team,
        encrypted_team_key: Option<String>,
        last_verified_at: Option<i64>,
    ) -> Result<TeamKeyCache, TeamKeyError> {
        let existing = self
            .repo
            .get(&self.scope, &team.id)
            .map_err(|e| TeamKeyError::Storage(e.to_string()))?;
        Ok(TeamKeyCache {
            scope: self.scope.clone(),
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            key_version: team.key_version,
            cached_key_version: encrypted_team_key.as_ref().map(|_| team.key_version),
            key_verification: team.key_verification.clone(),
            encrypted_team_key,
            last_verified_at,
            updated_at: team.updated_at,
            role: existing.and_then(|cache| cache.role),
        })
    }
}

pub fn team_key_cache_status(cache: &TeamKeyCache) -> TeamKeyCacheStatus {
    if cache.cached_key_version.is_some() && cache.cached_key_version != Some(cache.key_version) {
        return TeamKeyCacheStatus::VersionMismatch;
    }
    if cache.encrypted_team_key.is_none() || cache.cached_key_version.is_none() {
        return TeamKeyCacheStatus::Missing;
    }
    match cache.key_verification.as_deref() {
        Some(verification) if is_team_key_envelope(verification) => TeamKeyCacheStatus::Cached,
        Some(_) => TeamKeyCacheStatus::Invalid,
        None => TeamKeyCacheStatus::Missing,
    }
}

fn team_key_envelope_error(error: TeamKeyEnvelopeError) -> TeamKeyError {
    match error {
        TeamKeyEnvelopeError::KeyTooShort { minimum } => TeamKeyError::KeyTooShort { minimum },
        TeamKeyEnvelopeError::InvalidKeyOrEnvelope | TeamKeyEnvelopeError::CreationFailed => {
            TeamKeyError::InvalidTeamKey
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, RwLock};

    use crate::cloud_sync::team_key_envelope::{
        TeamKeyKdfParams, create_team_key_envelope, unlock_team_key,
    };
    use crate::cloud_sync::{
        CloudAccountScope, CloudSyncData, CloudSyncService, Team, TeamKeyError, TeamKeyLoadStatus,
        TeamKeyManager, data_type,
    };
    use crate::crypto;
    use crate::storage::connection::SqliteConnection;
    use crate::storage::migration::run_migrations;
    use crate::storage::{TeamKeyCache, TeamKeyCacheRepository};

    static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_scope() -> CloudAccountScope {
        CloudAccountScope::new("default", "user-1")
    }

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

    const TEAM_PASSPHRASE: &str = "team-secret-key";
    const NEW_PASSPHRASE: &str = "new-team-secret";

    fn team(_service: &CloudSyncService, version: u32) -> Team {
        let created = create_team_key_envelope(TEAM_PASSPHRASE, TeamKeyKdfParams::for_tests())
            .expect("create envelope");
        Team {
            id: "team-1".to_string(),
            name: "Platform".to_string(),
            owner_id: "owner-1".to_string(),
            description: None,
            key_verification: Some(created.verification),
            key_version: version,
            created_at: 100,
            updated_at: 200,
        }
    }

    fn manager(repo: TeamKeyCacheRepository) -> (TeamKeyManager, Arc<RwLock<CloudSyncService>>) {
        let service = Arc::new(RwLock::new(CloudSyncService::new()));
        (
            TeamKeyManager::new(repo, service.clone(), test_scope()),
            service,
        )
    }

    #[test]
    fn save_verified_key_persists_encrypted_cache_and_unlocks_service() {
        let repo = test_repo();
        let verification_service = CloudSyncService::new();
        let team = team(&verification_service, 1);
        let (manager, service) = manager(repo.clone());

        let status = manager
            .save_verified_team_key(&team, TEAM_PASSPHRASE, "personal-secret")
            .expect("team key saves");

        assert_eq!(TeamKeyLoadStatus::Unlocked, status);
        assert!(
            service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-1")
        );

        let cache = repo
            .get(&test_scope(), "team-1")
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
            .save_verified_team_key(&team, TEAM_PASSPHRASE, "personal-secret")
            .expect("team key saves");

        let (second_manager, second_service) = manager(repo);
        let status = second_manager
            .load_cached_team_key(&team, "personal-secret")
            .expect("cached key loads");

        assert_eq!(TeamKeyLoadStatus::Unlocked, status);
        assert!(
            second_service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-1")
        );
    }

    #[test]
    fn save_key_for_cached_team_uses_cached_verification() {
        let repo = test_repo();
        let verification_service = CloudSyncService::new();
        let team = team(&verification_service, 4);
        repo.upsert(&TeamKeyCache {
            scope: test_scope(),
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            key_version: team.key_version,
            cached_key_version: None,
            key_verification: team.key_verification.clone(),
            encrypted_team_key: None,
            last_verified_at: None,
            updated_at: team.updated_at,
            role: Some("member".to_string()),
        })
        .expect("cache seed");
        let (manager, service) = manager(repo.clone());

        let status = manager
            .save_key_for_cached_team("team-1", TEAM_PASSPHRASE, "personal-secret")
            .expect("team key saves");

        assert_eq!(TeamKeyLoadStatus::Unlocked, status);
        assert!(
            repo.get(&test_scope(), "team-1")
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
            key_verification: Some(
                create_team_key_envelope("other-team-secret", TeamKeyKdfParams::for_tests())
                    .expect("create missing envelope")
                    .verification,
            ),
            key_version: 1,
            created_at: 100,
            updated_at: 200,
        };
        let (first_manager, _) = manager(repo.clone());
        first_manager
            .save_verified_team_key(&valid_team, TEAM_PASSPHRASE, "personal-secret")
            .expect("team key saves");
        let (second_manager, service) = manager(repo);

        let statuses = second_manager.load_cached_team_keys(
            &[valid_team.clone(), missing_team.clone()],
            "personal-secret",
        );

        assert_eq!(
            vec![
                ("team-1".to_string(), Ok(TeamKeyLoadStatus::Unlocked)),
                ("team-2".to_string(), Ok(TeamKeyLoadStatus::Missing)),
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
            .save_verified_team_key(&team, TEAM_PASSPHRASE, "personal-secret")
            .expect("team key saves");
        let (second_manager, service) = manager(repo);

        let status = second_manager
            .load_cached_team_key_by_id("team-1", "personal-secret")
            .expect("cached key loads");

        assert_eq!(TeamKeyLoadStatus::Unlocked, status);
        assert!(
            service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-1")
        );
    }

    #[test]
    fn missing_cached_secret_clears_runtime_team_key() {
        let repo = test_repo();
        let verification_service = CloudSyncService::new();
        let team = team(&verification_service, 1);
        repo.upsert(&TeamKeyCache {
            scope: test_scope(),
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            key_version: team.key_version,
            cached_key_version: None,
            key_verification: team.key_verification.clone(),
            encrypted_team_key: None,
            last_verified_at: None,
            updated_at: team.updated_at,
            role: Some("member".to_string()),
        })
        .expect("cache seed");
        let (manager, service) = manager(repo);
        service
            .write()
            .expect("service lock")
            .set_team_key("team-1", "stale-runtime-key".to_string());

        let status = manager
            .load_cached_team_key(&team, "personal-secret")
            .expect("missing key status resolves");

        assert_eq!(TeamKeyLoadStatus::Missing, status);
        assert!(
            !service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-1")
        );
    }

    #[test]
    fn invalid_cached_secret_clears_runtime_team_key() {
        let repo = test_repo();
        let verification_service = CloudSyncService::new();
        let team = team(&verification_service, 1);
        repo.upsert(&TeamKeyCache {
            scope: test_scope(),
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            key_version: team.key_version,
            cached_key_version: Some(team.key_version),
            key_verification: team.key_verification.clone(),
            encrypted_team_key: Some("invalid-encrypted-key".to_string()),
            last_verified_at: Some(123),
            updated_at: team.updated_at,
            role: Some("member".to_string()),
        })
        .expect("cache seed");
        let (manager, service) = manager(repo);
        service
            .write()
            .expect("service lock")
            .set_team_key("team-1", "stale-runtime-key".to_string());

        let result = manager.load_cached_team_key(&team, "personal-secret");

        assert_eq!(Err(TeamKeyError::InvalidCachedKey), result);
        assert!(
            !service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-1")
        );
    }

    #[test]
    fn rotate_team_key_reencrypts_team_records_with_next_version() {
        let service = CloudSyncService::new();
        let team = team(&service, 3);
        let old_key = TEAM_PASSPHRASE;
        let new_key = NEW_PASSPHRASE;
        let old_data_key = unlock_team_key(team.key_verification.as_deref().unwrap(), old_key)
            .expect("unlock old key")
            .data_key;
        let record = CloudSyncData {
            id: "record-1".to_string(),
            owner_id: "owner-1".to_string(),
            team_id: Some(team.id.clone()),
            data_type: data_type::CONNECTION.to_string(),
            encrypted_data: crypto::encrypt_with_key(r#"{"name":"db"}"#, &old_data_key),
            key_version: team.key_version,
            checksum: "checksum".to_string(),
            version: 7,
            updated_at: 100,
            deleted_at: None,
        };

        let rotation = TeamKeyManager::rotate_team_key_records(
            &team,
            old_key,
            new_key,
            &[record],
            TeamKeyKdfParams::for_tests(),
        )
        .expect("rotation succeeds");

        assert_eq!(4, rotation.team.key_version);
        assert_ne!(team.key_verification, rotation.team.key_verification);
        assert_eq!(1, rotation.records.len());
        let rotated = &rotation.records[0];
        assert_eq!(4, rotated.key_version);
        assert_eq!(7, rotated.version);
        let decrypted = crypto::decrypt_with_key(&rotated.encrypted_data, &rotation.data_key)
            .expect("rotated data decrypts with new key");
        assert_eq!(r#"{"name":"db"}"#, decrypted);
        assert!(crypto::decrypt_with_key(&rotated.encrypted_data, &old_data_key).is_err());
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
            encrypted_data: crypto::encrypt_with_key("{}", "unused-data-key"),
            key_version: team.key_version,
            checksum: "checksum".to_string(),
            version: 1,
            updated_at: 100,
            deleted_at: None,
        };

        let result = TeamKeyManager::rotate_team_key_records(
            &team,
            "wrong-team-key",
            NEW_PASSPHRASE,
            &[record],
            TeamKeyKdfParams::for_tests(),
        );

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
        assert!(
            repo.get(&test_scope(), "team-1")
                .expect("cache read")
                .is_none()
        );
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
            .save_verified_team_key(&old_team, TEAM_PASSPHRASE, "personal-secret")
            .expect("team key saves");

        let status = manager
            .load_cached_team_key(&new_team, "personal-secret")
            .expect("cached key status resolves");

        assert_eq!(TeamKeyLoadStatus::VersionMismatch, status);
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
            scope: test_scope(),
            team_id: "team-1".to_string(),
            team_name: "Platform".to_string(),
            key_version: 1,
            cached_key_version: Some(1),
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
            .get(&test_scope(), "team-1")
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
