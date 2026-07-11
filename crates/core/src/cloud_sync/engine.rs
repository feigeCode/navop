//! 云同步引擎
//!
//! 统一管理同步流程，将同步逻辑从 UI 层解耦。
//!
//! ## 设计原则
//!
//! - 参考 Dropbox Nucleus 架构的三棵树模型
//! - 支持冲突检测和多种解决策略
//! - 提供完整同步和增量同步两种模式

use super::connection_sync::ConnectionSyncHandler;
use super::generic_sync::generic_sync;
use super::sync_type::SyncTypeHandler;
use super::workspace_sync::WorkspaceSyncType;
use crate::cloud_sync::CloudAccountScope;
use crate::cloud_sync::client::CloudApiClient;
use crate::cloud_sync::models::{ConflictResolution, SyncResult, Team, TeamRole};
use crate::cloud_sync::queue::OperationQueue;
use crate::cloud_sync::service::{CloudSyncService, SyncError};
use crate::cloud_sync::team_key_envelope::TeamKeyKdfParams;
use crate::cloud_sync::team_key_manager::{TeamKeyError, TeamKeyLoadStatus, TeamKeyManager};
use crate::crypto;
use crate::storage::{StorageManager, TeamKeyCache, TeamKeyCacheRepository};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub type SyncFuture<'a> = Pin<Box<dyn Future<Output = Result<SyncResult, SyncError>> + Send + 'a>>;

pub trait SyncHandler: Send + Sync {
    fn name(&self) -> &'static str;
    fn sync<'a>(&'a self, engine: &'a SyncEngine) -> SyncFuture<'a>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamKeyRotationResult {
    pub re_encrypted: usize,
    pub key_version: u32,
}

/// 泛型桥接器：将 `SyncTypeHandler` 适配为 `SyncHandler`
///
/// 通过 `generic_sync` 通用流程执行同步，使新数据类型只需实现
/// `SyncTypeHandler` trait 即可接入同步引擎。
pub struct TypedSyncBridge<H: SyncTypeHandler> {
    handler: H,
}

impl<H: SyncTypeHandler> SyncHandler for TypedSyncBridge<H> {
    fn name(&self) -> &'static str {
        self.handler.display_name()
    }

    fn sync<'a>(&'a self, engine: &'a SyncEngine) -> SyncFuture<'a> {
        Box::pin(generic_sync(engine, &self.handler))
    }
}

/// 同步引擎
///
/// 核心职责：
/// 1. 协调本地存储和云端 API 的交互
/// 2. 计算同步计划，检测冲突
/// 3. 执行同步操作并更新状态
pub struct SyncEngine {
    /// 云端 API 客户端
    pub(crate) cloud_client: Arc<dyn CloudApiClient>,
    /// 加解密服务
    pub(crate) crypto_service: Arc<std::sync::RwLock<CloudSyncService>>,
    /// 本地存储管理器
    pub(crate) storage: StorageManager,
    /// 冲突解决策略
    pub(crate) conflict_strategy: ConflictResolution,
    handlers: Vec<Box<dyn SyncHandler>>,
    /// 当前用户所在团队列表（同步开始时获取）
    pub(crate) cached_teams: std::sync::RwLock<Vec<Team>>,
}

impl SyncEngine {
    /// 创建新的同步引擎
    pub fn new(
        cloud_client: Arc<dyn CloudApiClient>,
        crypto_service: Arc<std::sync::RwLock<CloudSyncService>>,
        storage: StorageManager,
    ) -> Self {
        Self {
            cloud_client,
            crypto_service,
            storage,
            conflict_strategy: ConflictResolution::UseCloud, // 默认使用云端版本
            handlers: vec![
                Box::new(TypedSyncBridge {
                    handler: WorkspaceSyncType,
                }),
                Box::new(ConnectionSyncHandler),
            ],
            cached_teams: std::sync::RwLock::new(Vec::new()),
        }
    }

    /// 设置冲突解决策略
    pub fn with_conflict_strategy(mut self, strategy: ConflictResolution) -> Self {
        self.conflict_strategy = strategy;
        self
    }

    pub fn register_handler(&mut self, handler: Box<dyn SyncHandler>) {
        self.handlers.push(handler);
    }

    /// 注册一个类型化同步处理器
    ///
    /// 通过 `TypedSyncBridge` 适配为 `SyncHandler`，自动接入 `generic_sync` 通用流程。
    pub fn register_type<H: SyncTypeHandler>(mut self, handler: H) -> Self {
        self.handlers.push(Box::new(TypedSyncBridge { handler }));
        self
    }

    /// 获取当前时间戳（秒）
    pub(crate) fn current_timestamp() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// 确保加密服务已解锁
    fn ensure_unlocked(&self) -> Result<(), SyncError> {
        // 如果本地 crypto 模块已解锁但同步服务未解锁，同步密钥状态
        if crypto::has_master_key() {
            if let Some(raw_key) = crypto::get_raw_master_key() {
                let mut service_write = self
                    .crypto_service
                    .write()
                    .map_err(|_| SyncError::StorageError("同步服务锁获取失败".to_string()))?;
                if !service_write.is_unlocked() {
                    tracing::info!("[同步引擎] 从本地 crypto 模块同步密钥状态");
                    service_write.set_master_key_directly(raw_key);
                }
            }
        }

        let service = self
            .crypto_service
            .read()
            .map_err(|_| SyncError::StorageError("同步服务锁获取失败".to_string()))?;

        if !service.is_unlocked() {
            return Err(SyncError::NotUnlocked);
        }

        Ok(())
    }

    /// 执行完整同步
    ///
    /// ## 同步流程
    /// 1. 获取团队列表并缓存
    /// 2. 先同步工作空间（无外键依赖）
    /// 3. 再同步连接（依赖工作空间）
    pub async fn sync(&self) -> Result<SyncResult, SyncError> {
        tracing::info!("========== 开始云同步 ==========");

        self.ensure_unlocked()?;

        match self.refresh_team_key_cache().await {
            Ok(count) => {
                tracing::info!("[同步] 已缓存 {} 个团队", count);
            }
            Err(e) => {
                tracing::warn!("[同步] 获取团队列表失败: {}（将仅同步个人数据）", e);
            }
        }

        let mut result = SyncResult::default();

        for handler in &self.handlers {
            match handler.sync(self).await {
                Ok(sync_result) => {
                    result.uploaded += sync_result.uploaded;
                    result.downloaded += sync_result.downloaded;
                    result.deleted += sync_result.deleted;
                    result.conflicts.extend(sync_result.conflicts);
                    result.errors.extend(sync_result.errors);
                }
                Err(e) => {
                    tracing::error!("[同步] {}同步失败: {}", handler.name(), e);
                    result
                        .errors
                        .push(format!("{}同步失败: {}", handler.name(), e));
                }
            }
        }

        tracing::info!(
            "========== 同步完成: 上传 {} 个, 下载 {} 个, 错误 {} 个 ==========",
            result.uploaded,
            result.downloaded,
            result.errors.len()
        );

        Ok(result)
    }

    pub(crate) fn take_operation_queue(&self, key: &str) -> Result<OperationQueue, SyncError> {
        let mut service = self
            .crypto_service
            .write()
            .map_err(|_| SyncError::StorageError("同步服务锁获取失败".to_string()))?;

        Ok(service.take_operation_queue(key))
    }

    pub(crate) fn store_operation_queue(
        &self,
        key: &str,
        queue: OperationQueue,
    ) -> Result<(), SyncError> {
        let mut service = self
            .crypto_service
            .write()
            .map_err(|_| SyncError::StorageError("同步服务锁获取失败".to_string()))?;

        service.store_operation_queue(key, queue);
        Ok(())
    }

    /// 获取缓存的团队列表
    pub(crate) fn get_cached_teams(&self) -> Vec<Team> {
        self.cached_teams
            .read()
            .map(|teams| teams.clone())
            .unwrap_or_default()
    }

    /// 检查团队密钥是否已解锁
    pub(crate) fn is_team_unlocked(&self, team_id: &str) -> bool {
        self.crypto_service
            .read()
            .map(|service| service.is_team_unlocked(team_id))
            .unwrap_or(false)
    }

    fn account_scope(&self) -> Result<CloudAccountScope, SyncError> {
        let user_id = self
            .crypto_service
            .read()
            .map_err(|_| SyncError::StorageError("同步服务锁获取失败".to_string()))?
            .user_id()
            .map(str::to_string)
            .ok_or(SyncError::NotLoggedIn)?;
        Ok(CloudAccountScope::new(
            self.cloud_client.environment_id(),
            user_id,
        ))
    }

    /// 刷新当前用户团队列表并写入本地团队密钥缓存。
    ///
    /// 只同步团队元数据和当前用户角色，不上传或下载连接/工作空间数据。
    pub async fn refresh_team_key_cache(&self) -> Result<usize, SyncError> {
        let scope = self.account_scope()?;
        let teams = self
            .cloud_client
            .list_teams()
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        tracing::info!("[同步] 获取到 {} 个团队", teams.len());
        let cached_count = self.cache_team_roles(&teams, &scope).await;
        self.restore_cached_team_keys(&teams, &scope);
        self.cached_teams
            .write()
            .map_err(|_| SyncError::StorageError("团队缓存锁获取失败".to_string()))?
            .clone_from(&teams);
        Ok(cached_count)
    }

    /// 缓存团队角色信息到 team_key_cache 表
    async fn cache_team_roles(&self, teams: &[Team], scope: &CloudAccountScope) -> usize {
        let repo = match self.storage.get::<TeamKeyCacheRepository>() {
            Some(repo) => repo,
            None => return 0,
        };
        let mut cached_count = 0;
        for team in teams {
            if self.cache_team_role(repo.as_ref(), team, scope).await {
                cached_count += 1;
            }
        }
        cached_count
    }

    async fn cache_team_role(
        &self,
        repo: &TeamKeyCacheRepository,
        team: &Team,
        scope: &CloudAccountScope,
    ) -> bool {
        let members = match self.cloud_client.list_team_members(&team.id).await {
            Ok(members) => members,
            Err(e) => {
                tracing::warn!("[同步] 获取团队 {} 成员列表失败: {}", team.id, e);
                return false;
            }
        };
        let Some(member) = members.iter().find(|m| m.user_id == scope.user_id) else {
            return false;
        };
        let existing_cache = match repo.get(scope, &team.id) {
            Ok(cache) => cache,
            Err(e) => {
                tracing::warn!("[同步] 读取团队 {} 缓存失败: {}", team.id, e);
                None
            }
        };
        let cache = team_key_cache_for_cloud_team(scope, team, member.role, existing_cache);
        match repo.upsert(&cache) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("[同步] 更新团队 {} 缓存失败: {}", team.id, e);
                false
            }
        }
    }

    fn restore_cached_team_keys(&self, teams: &[Team], scope: &CloudAccountScope) {
        let Some(personal_key) = crypto::get_raw_master_key() else {
            return;
        };
        let Some(repo) = self.storage.get::<TeamKeyCacheRepository>() else {
            return;
        };
        let manager =
            TeamKeyManager::new((*repo).clone(), self.crypto_service.clone(), scope.clone());
        for (team_id, status) in manager.load_cached_team_keys(teams, &personal_key) {
            match status {
                Ok(TeamKeyLoadStatus::Unlocked) => {
                    tracing::info!("[同步] 已从本地缓存解锁团队 {}", team_id);
                }
                Ok(TeamKeyLoadStatus::Missing) => {}
                Ok(TeamKeyLoadStatus::VersionMismatch) => {
                    tracing::warn!("[同步] 团队 {} 的本地密钥版本已过期", team_id);
                }
                Ok(TeamKeyLoadStatus::Invalid) => {
                    tracing::warn!("[同步] 团队 {} 的密钥格式无效", team_id);
                }
                Err(error) => {
                    tracing::warn!("[同步] 恢复团队 {} 密钥失败: {}", team_id, error);
                }
            }
        }
    }

    pub async fn rotate_team_key(
        &self,
        team_id: &str,
        old_key: &str,
        new_key: &str,
    ) -> Result<TeamKeyRotationResult, SyncError> {
        self.ensure_unlocked()?;
        let teams = self
            .cloud_client
            .list_teams()
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        let team = teams
            .iter()
            .find(|team| team.id == team_id)
            .cloned()
            .ok_or_else(|| SyncError::StorageError("团队不存在或无权限".to_string()))?;
        let records = self
            .cloud_client
            .list_sync_data(None, Some(team_id), None)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        let records_to_rotate = records
            .into_iter()
            .filter(|record| !record.encrypted_data.is_empty())
            .collect::<Vec<_>>();
        let rotation = TeamKeyManager::rotate_team_key_records(
            &team,
            old_key,
            new_key,
            &records_to_rotate,
            TeamKeyKdfParams::production(),
        )
        .map_err(|e| SyncError::DataFormatError(e.to_string()))?;
        self.cloud_client
            .rotate_team_key(&rotation.team, &rotation.records)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        self.save_rotated_team_key(&rotation.team, new_key)?;
        Ok(TeamKeyRotationResult {
            re_encrypted: rotation.records.len(),
            key_version: rotation.team.key_version,
        })
    }

    pub async fn save_or_initialize_team_key_for_cached_team(
        &self,
        team_id: &str,
        team_key: &str,
        personal_key: &str,
    ) -> Result<TeamKeyLoadStatus, SyncError> {
        let scope = self.account_scope()?;
        let repo = self
            .storage
            .get::<TeamKeyCacheRepository>()
            .ok_or_else(|| {
                SyncError::StorageError("TeamKeyCacheRepository not found".to_string())
            })?;
        let cache = repo
            .get(&scope, team_id)
            .map_err(|e| SyncError::StorageError(e.to_string()))?
            .ok_or_else(|| SyncError::StorageError("团队缓存不存在".to_string()))?;
        let manager = TeamKeyManager::new((*repo).clone(), self.crypto_service.clone(), scope);
        if cache.key_verification.is_some() {
            return manager
                .save_key_for_cached_team(team_id, team_key, personal_key)
                .map_err(team_key_error_to_sync_error);
        }

        let team = initial_team_key_team_from_cache(&cache);
        let prepared = TeamKeyManager::prepare_initial_team_key(
            &team,
            team_key,
            TeamKeyKdfParams::production(),
        )
        .map_err(team_key_error_to_sync_error)?;
        let initialized = self
            .cloud_client
            .initialize_team_key(&prepared.team)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        let status = manager
            .save_verified_team_key(&initialized, team_key, personal_key)
            .map_err(team_key_error_to_sync_error)?;
        self.update_cached_team(initialized)?;
        Ok(status)
    }

    fn save_rotated_team_key(&self, team: &Team, new_key: &str) -> Result<(), SyncError> {
        let scope = self.account_scope()?;
        let personal_key = crypto::get_raw_master_key().ok_or(SyncError::NotUnlocked)?;
        let repo = self
            .storage
            .get::<TeamKeyCacheRepository>()
            .ok_or_else(|| {
                SyncError::StorageError("TeamKeyCacheRepository not found".to_string())
            })?;
        TeamKeyManager::new((*repo).clone(), self.crypto_service.clone(), scope)
            .save_verified_team_key(team, new_key, &personal_key)
            .map_err(|e| SyncError::StorageError(e.to_string()))?;
        self.update_cached_team(team.clone())
    }

    fn update_cached_team(&self, team: Team) -> Result<(), SyncError> {
        let mut cache = self
            .cached_teams
            .write()
            .map_err(|_| SyncError::StorageError("团队缓存锁获取失败".to_string()))?;
        if let Some(existing) = cache.iter_mut().find(|cached| cached.id == team.id) {
            *existing = team;
        } else {
            cache.push(team);
        }
        Ok(())
    }

    /// 使用指定的策略映射应用冲突解决方案
    ///
    /// 允许为每个冲突单独指定解决策略，而不是使用全局策略
    pub async fn apply_conflict_resolutions(
        &self,
        conflicts: Vec<crate::cloud_sync::models::SyncConflict>,
        strategies: std::collections::HashMap<String, ConflictResolution>,
    ) -> Result<SyncResult, SyncError> {
        self.ensure_unlocked()?;

        let mut result = SyncResult::default();
        result.conflicts = conflicts.clone();

        // 为每个冲突应用指定的策略
        for conflict in &conflicts {
            let cloud_id = &conflict.cloud.id;
            let strategy = strategies
                .get(cloud_id)
                .copied()
                .unwrap_or(self.conflict_strategy);

            let resolved_action = self.create_resolved_action(conflict, strategy);

            if let Err(e) = self.apply_single_conflict(&resolved_action).await {
                result.errors.push(format!("应用冲突解决失败: {}", e));
            }
        }

        Ok(result)
    }

    /// 创建冲突解决操作
    fn create_resolved_action(
        &self,
        conflict: &crate::cloud_sync::models::SyncConflict,
        strategy: ConflictResolution,
    ) -> crate::cloud_sync::connection_sync::ResolvedConflictAction {
        use crate::cloud_sync::connection_sync::ResolvedConflictAction;

        match strategy {
            ConflictResolution::UseCloud => ResolvedConflictAction {
                conflict: conflict.clone(),
                resolution: ConflictResolution::UseCloud,
                result_connection: None,
            },
            ConflictResolution::UseLocal => ResolvedConflictAction {
                conflict: conflict.clone(),
                resolution: ConflictResolution::UseLocal,
                result_connection: Some(conflict.local.clone()),
            },
            ConflictResolution::KeepBoth => {
                let mut copy = conflict.local.clone();
                copy.id = None;
                copy.cloud_id = None;
                copy.last_synced_at = None;
                let timestamp = Self::current_timestamp();
                copy.name = format!("{} (冲突副本 {})", copy.name, timestamp);

                ResolvedConflictAction {
                    conflict: conflict.clone(),
                    resolution: ConflictResolution::KeepBoth,
                    result_connection: Some(copy),
                }
            }
        }
    }

    /// 应用单个冲突解决方案
    async fn apply_single_conflict(
        &self,
        resolved: &crate::cloud_sync::connection_sync::ResolvedConflictAction,
    ) -> Result<(), SyncError> {
        use crate::storage::ConnectionRepository;
        use crate::storage::traits::Repository;

        match resolved.resolution {
            ConflictResolution::UseCloud => {
                if resolved.conflict.conflict_type
                    == crate::cloud_sync::models::ConflictType::LocalModifiedCloudDeleted
                {
                    return self.apply_cloud_deleted_connection_conflict(&resolved.conflict, false);
                }
                // 更新本地连接
                let service = self
                    .crypto_service
                    .read()
                    .map_err(|_| SyncError::StorageError("同步服务锁获取失败".to_string()))?;

                let (mut updated, workspace_cloud_id) = service
                    .decrypt_sync_data_connection_with_workspace_cloud_id(
                        &resolved.conflict.cloud,
                    )?;
                drop(service);
                updated.id = resolved.conflict.local.id;
                updated.cloud_id = Some(resolved.conflict.cloud.id.clone());
                updated.workspace_id =
                    self.local_workspace_id_for_cloud_id(workspace_cloud_id.as_deref())?;
                updated.last_synced_at = Some(Self::current_timestamp());

                let repo = self.storage.get::<ConnectionRepository>().ok_or_else(|| {
                    SyncError::StorageError("ConnectionRepository not found".to_string())
                })?;

                repo.update(&updated)
                    .map_err(|e| SyncError::StorageError(e.to_string()))?;

                Ok(())
            }
            ConflictResolution::UseLocal => {
                self.apply_use_local_connection_conflict(&resolved.conflict)
                    .await
            }
            ConflictResolution::KeepBoth => {
                if resolved.conflict.conflict_type
                    == crate::cloud_sync::models::ConflictType::LocalModifiedCloudDeleted
                {
                    return self.apply_cloud_deleted_connection_conflict(&resolved.conflict, true);
                }
                // 创建本地副本
                if let Some(copy) = &resolved.result_connection {
                    let repo = self.storage.get::<ConnectionRepository>().ok_or_else(|| {
                        SyncError::StorageError("ConnectionRepository not found".to_string())
                    })?;

                    let mut new_conn = copy.clone();
                    repo.insert(&mut new_conn)
                        .map_err(|e| SyncError::StorageError(e.to_string()))?;
                }

                // 同时更新本地连接为云端版本
                let service = self
                    .crypto_service
                    .read()
                    .map_err(|_| SyncError::StorageError("同步服务锁获取失败".to_string()))?;

                let (mut updated, workspace_cloud_id) = service
                    .decrypt_sync_data_connection_with_workspace_cloud_id(
                        &resolved.conflict.cloud,
                    )?;
                drop(service);
                updated.id = resolved.conflict.local.id;
                updated.cloud_id = Some(resolved.conflict.cloud.id.clone());
                updated.workspace_id =
                    self.local_workspace_id_for_cloud_id(workspace_cloud_id.as_deref())?;
                updated.last_synced_at = Some(Self::current_timestamp());

                let repo = self.storage.get::<ConnectionRepository>().ok_or_else(|| {
                    SyncError::StorageError("ConnectionRepository not found".to_string())
                })?;

                repo.update(&updated)
                    .map_err(|e| SyncError::StorageError(e.to_string()))?;

                Ok(())
            }
        }
    }
}

fn team_key_cache_for_cloud_team(
    scope: &CloudAccountScope,
    team: &Team,
    role: TeamRole,
    existing: Option<TeamKeyCache>,
) -> TeamKeyCache {
    let (cached_key_version, encrypted_team_key, last_verified_at) = existing
        .map(|cache| {
            (
                cache.cached_key_version,
                cache.encrypted_team_key,
                cache.last_verified_at,
            )
        })
        .unwrap_or((None, None, None));

    TeamKeyCache {
        scope: scope.clone(),
        team_id: team.id.clone(),
        team_name: team.name.clone(),
        key_version: team.key_version,
        cached_key_version,
        key_verification: team.key_verification.clone(),
        encrypted_team_key,
        last_verified_at,
        updated_at: team.updated_at,
        role: Some(role.to_string()),
    }
}

fn initial_team_key_team_from_cache(cache: &TeamKeyCache) -> Team {
    Team {
        id: cache.team_id.clone(),
        name: cache.team_name.clone(),
        owner_id: String::new(),
        description: None,
        key_verification: None,
        key_version: cache.key_version.max(1),
        created_at: 0,
        updated_at: cache.updated_at,
    }
}

fn team_key_error_to_sync_error(error: TeamKeyError) -> SyncError {
    match error {
        TeamKeyError::Storage(message) => SyncError::StorageError(message),
        TeamKeyError::ServiceLock => SyncError::StorageError(error.to_string()),
        _ => SyncError::DataFormatError(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::team_key_cache_for_cloud_team;
    use crate::cloud_sync::client::{
        AuthResponse, CloudApiClient, CloudApiError, OAuthResponse, UserInfo,
    };
    use crate::cloud_sync::models::{CloudSyncData, CloudUserConfig, Team, TeamMember, TeamRole};
    use crate::cloud_sync::{CloudAccountScope, CloudSyncService, SyncEngine, TeamKeyLoadStatus};
    use crate::crypto;
    use crate::storage::connection::SqliteConnection;
    use crate::storage::migration::run_migrations;
    use crate::storage::{StorageManager, TeamKeyCache, TeamKeyCacheRepository};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex, RwLock};

    fn team() -> Team {
        Team {
            id: "team-1".to_string(),
            name: "Platform".to_string(),
            owner_id: "owner-1".to_string(),
            description: None,
            key_verification: None,
            key_version: 7,
            created_at: 100,
            updated_at: 200,
        }
    }

    fn test_scope() -> CloudAccountScope {
        CloudAccountScope::new("default", "user-1")
    }

    fn member(role: TeamRole) -> TeamMember {
        TeamMember {
            id: "member-1".to_string(),
            team_id: "team-1".to_string(),
            user_id: "user-1".to_string(),
            role,
            joined_at: 300,
        }
    }

    fn test_storage() -> (StorageManager, TeamKeyCacheRepository) {
        let db_path = std::env::temp_dir().join(format!(
            "onetcli-refresh-team-key-cache-{}-{}.db",
            std::process::id(),
            unique_suffix()
        ));
        let _ = std::fs::remove_file(&db_path);
        let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
        conn.with_connection(|conn| run_migrations(conn))
            .expect("run migrations");
        let storage = StorageManager::new_with_connection(conn.clone());
        let repo = TeamKeyCacheRepository::new(conn);
        storage.register(repo.clone());
        (storage, repo)
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    }

    struct FakeCloudClient {
        teams: Vec<Team>,
        members: Vec<TeamMember>,
        initialized_team: Arc<Mutex<Option<Team>>>,
    }

    #[async_trait]
    impl CloudApiClient for FakeCloudClient {
        async fn sign_in_with_password(
            &self,
            _email: &str,
            _password: &str,
        ) -> Result<AuthResponse, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn sign_in_with_oauth(
            &self,
            _provider: &str,
            _redirect_url: &str,
        ) -> Result<OAuthResponse, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn sign_up(
            &self,
            _email: &str,
            _password: &str,
        ) -> Result<AuthResponse, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn sign_out(&self) -> Result<(), CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn get_current_user(&self) -> Result<Option<UserInfo>, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn refresh_token(&self, _refresh_token: &str) -> Result<AuthResponse, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn sign_in_with_otp(&self, _email: &str) -> Result<(), CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn verify_otp(
            &self,
            _email: &str,
            _token: &str,
        ) -> Result<AuthResponse, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn get_user_config(&self) -> Result<Option<CloudUserConfig>, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn save_user_config(&self, _config: &CloudUserConfig) -> Result<(), CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn get_subscription(
            &self,
        ) -> Result<Option<crate::license::SubscriptionInfo>, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn list_models(&self) -> Result<Vec<String>, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn list_sync_data(
            &self,
            _data_type: Option<&str>,
            _team_id: Option<&str>,
            _since: Option<i64>,
        ) -> Result<Vec<CloudSyncData>, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn create_sync_data(
            &self,
            _data: &CloudSyncData,
        ) -> Result<CloudSyncData, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn update_sync_data(
            &self,
            _data: &CloudSyncData,
        ) -> Result<CloudSyncData, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn delete_sync_data(&self, _id: &str) -> Result<(), CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn list_teams(&self) -> Result<Vec<Team>, CloudApiError> {
            Ok(self.teams.clone())
        }

        async fn list_team_members(
            &self,
            _team_id: &str,
        ) -> Result<Vec<TeamMember>, CloudApiError> {
            Ok(self.members.clone())
        }

        async fn rotate_team_key(
            &self,
            _team: &Team,
            _records: &[CloudSyncData],
        ) -> Result<(), CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn initialize_team_key(&self, team: &Team) -> Result<Team, CloudApiError> {
            *self.initialized_team.lock().expect("init lock") = Some(team.clone());
            Ok(team.clone())
        }

        async fn chat(
            &self,
            _request: &llm_connector::ChatRequest,
        ) -> Result<String, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }

        async fn chat_stream(
            &self,
            _request: &llm_connector::ChatRequest,
        ) -> Result<crate::llm::ChatStream, CloudApiError> {
            Err(CloudApiError::Unknown("not used".to_string()))
        }
    }

    #[test]
    fn builds_team_cache_for_cloud_team_without_existing_key() {
        let cache = team_key_cache_for_cloud_team(&test_scope(), &team(), TeamRole::Admin, None);

        assert_eq!("team-1", cache.team_id);
        assert_eq!("Platform", cache.team_name);
        assert_eq!(7, cache.key_version);
        assert_eq!(None, cache.encrypted_team_key);
        assert_eq!(None, cache.last_verified_at);
        assert_eq!(Some("admin".to_string()), cache.role);
    }

    #[test]
    fn preserving_existing_team_key_when_cloud_team_metadata_changes() {
        let existing = TeamKeyCache {
            scope: test_scope(),
            team_id: "team-1".to_string(),
            team_name: "Old name".to_string(),
            key_version: 3,
            cached_key_version: Some(3),
            key_verification: Some("old-verification".to_string()),
            encrypted_team_key: Some("encrypted-key".to_string()),
            last_verified_at: Some(1234),
            updated_at: 5678,
            role: Some("member".to_string()),
        };

        let cache =
            team_key_cache_for_cloud_team(&test_scope(), &team(), TeamRole::Owner, Some(existing));

        assert_eq!("Platform", cache.team_name);
        assert_eq!(7, cache.key_version);
        assert_eq!(Some("encrypted-key".to_string()), cache.encrypted_team_key);
        assert_eq!(team().key_verification, cache.key_verification);
        assert_eq!(Some(1234), cache.last_verified_at);
        assert_eq!(Some("owner".to_string()), cache.role);
    }

    #[tokio::test]
    async fn refresh_team_key_cache_loads_cloud_team_metadata() {
        let (storage, repo) = test_storage();
        let service = Arc::new(RwLock::new(CloudSyncService::new()));
        service
            .write()
            .expect("service lock")
            .set_logged_in("user-1".to_string());
        let engine = SyncEngine::new(
            Arc::new(FakeCloudClient {
                teams: vec![Team {
                    key_verification: Some("verification".to_string()),
                    ..team()
                }],
                members: vec![member(TeamRole::Owner)],
                initialized_team: Arc::new(Mutex::new(None)),
            }),
            service,
            storage,
        );

        let refreshed = engine.refresh_team_key_cache().await.unwrap();

        let cache = repo
            .get(&test_scope(), "team-1")
            .expect("cache read")
            .expect("team cache exists");
        assert_eq!(1, refreshed);
        assert_eq!("Platform", cache.team_name);
        assert_eq!(Some("verification".to_string()), cache.key_verification);
        assert_eq!(Some("owner".to_string()), cache.role);
    }

    #[tokio::test]
    async fn save_or_initialize_team_key_initializes_missing_verification() {
        let (storage, repo) = test_storage();
        repo.upsert(&TeamKeyCache {
            scope: test_scope(),
            team_id: "team-1".to_string(),
            team_name: "Platform".to_string(),
            key_version: 0,
            cached_key_version: None,
            key_verification: None,
            encrypted_team_key: None,
            last_verified_at: None,
            updated_at: 200,
            role: Some("owner".to_string()),
        })
        .expect("cache seed");
        let initialized_team = Arc::new(Mutex::new(None));
        let service = Arc::new(RwLock::new(CloudSyncService::new()));
        service
            .write()
            .expect("service lock")
            .set_logged_in("user-1".to_string());
        let engine = SyncEngine::new(
            Arc::new(FakeCloudClient {
                teams: Vec::new(),
                members: Vec::new(),
                initialized_team: initialized_team.clone(),
            }),
            service.clone(),
            storage,
        );

        let status = engine
            .save_or_initialize_team_key_for_cached_team(
                "team-1",
                "team-secret-key",
                "personal-secret",
            )
            .await
            .unwrap();

        let initialized = initialized_team
            .lock()
            .expect("init lock")
            .clone()
            .expect("team initialized");
        let cache = repo
            .get(&test_scope(), "team-1")
            .expect("cache read")
            .expect("cache exists");
        assert_eq!(TeamKeyLoadStatus::Unlocked, status);
        assert_eq!(1, initialized.key_version);
        assert!(
            initialized
                .key_verification
                .as_deref()
                .unwrap()
                .starts_with("TEAMKEY2:")
        );
        assert!(cache.key_verification.is_some());
        assert!(cache.encrypted_team_key.is_some());
        assert!(
            service
                .read()
                .expect("service lock")
                .is_team_unlocked("team-1")
        );
    }
}
