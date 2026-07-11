//! 云同步模块
//!
//! 提供连接配置的云端同步功能，支持端到端加密。
//!
//! ## 主要功能
//!
//! - 连接配置的加密上传和解密下载
//! - 主密钥管理（设置、修改、验证）
//! - 同步状态追踪
//! - 冲突检测和解决
//!
//! ## 使用流程
//!
//! 1. 用户登录云端账户
//! 2. 首次使用时设置主密钥，生成 key_verification 上传云端
//! 3. 非首次使用时输入主密钥，从云端获取 key_verification 验证
//! 4. 验证通过后解锁同步服务
//! 5. 执行同步操作（上传/下载/删除）

pub mod client;
pub mod conflict;
mod connection_sync;
pub mod engine;
mod generic_sync;
mod models;
pub mod personal;
pub mod queue;
mod service;
pub mod state_manager;
pub mod supabase;
pub mod sync_type;
pub mod team_key_envelope;
pub mod team_key_manager;
pub mod team_scope;

#[cfg(test)]
#[path = "team_key_envelope_tests.rs"]
mod team_key_envelope_tests;

#[cfg(test)]
#[path = "team_key_manager_status_tests.rs"]
mod team_key_manager_status_tests;
mod workspace_sync;

use std::sync::{Arc, RwLock};

use gpui::{App, Global};

pub use client::*;
pub use conflict::*;
pub use engine::*;
pub use models::*;
pub use queue::*;
pub use service::*;
pub use state_manager::*;
pub use sync_type::*;
pub use team_key_manager::*;
pub use team_scope::*;

use crate::crypto;
use crate::storage::{GlobalStorageState, StoredConnection, TeamKeyCacheRepository};

// ============================================================================
// 全局用户状态
// ============================================================================

/// 全局当前用户状态（供跨 crate 访问登录态）
#[derive(Clone, Default)]
pub struct GlobalCloudUser {
    user: Arc<RwLock<Option<UserInfo>>>,
}

impl Global for GlobalCloudUser {}

impl GlobalCloudUser {
    /// 获取当前用户
    pub fn get_user(cx: &App) -> Option<UserInfo> {
        if let Some(state) = cx.try_global::<GlobalCloudUser>() {
            state.user.read().ok().and_then(|u| u.clone())
        } else {
            None
        }
    }

    /// 是否已登录
    pub fn is_logged_in(cx: &App) -> bool {
        Self::get_user(cx).is_some()
    }

    /// 设置当前用户
    pub fn set_user(user: Option<UserInfo>, cx: &mut App) {
        if !cx.has_global::<GlobalCloudUser>() {
            cx.set_global(GlobalCloudUser::default());
        }
        if let Some(state) = cx.try_global::<GlobalCloudUser>() {
            if let Ok(mut guard) = state.user.write() {
                *guard = user;
            }
        }
    }
}

// ============================================================================
// 团队选项（供 UI 下拉使用）
// ============================================================================

/// 团队选择项
#[derive(Debug, Clone)]
pub struct TeamOption {
    pub id: String,
    pub name: String,
    pub key_status: TeamKeyCacheStatus,
    pub key_version: u32,
    pub key_verification: Option<String>,
    pub last_verified_at: Option<i64>,
    pub role: Option<String>,
}

fn team_option_from_cache(c: crate::storage::TeamKeyCache) -> TeamOption {
    let key_status = team_key_cache_status(&c);

    TeamOption {
        id: c.team_id,
        name: c.team_name,
        key_status,
        key_version: c.key_version,
        key_verification: c.key_verification,
        last_verified_at: c.last_verified_at,
        role: c.role,
    }
}

/// 获取可用团队列表（从本地 team_key_cache 缓存读取）
pub fn get_cached_team_options(cx: &App) -> Vec<TeamOption> {
    let Some(scope) = current_cloud_scope(cx) else {
        return Vec::new();
    };
    let Some(storage) = cx.try_global::<GlobalStorageState>() else {
        return Vec::new();
    };
    let Some(repo) = storage.storage.get::<TeamKeyCacheRepository>() else {
        return Vec::new();
    };
    match repo.list(&scope) {
        Ok(caches) => caches.into_iter().map(team_option_from_cache).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn ensure_team_key_ready_for_save(team_id: Option<&str>, cx: &App) -> Result<(), TeamKeyError> {
    let Some(team_id) = team_id else {
        return Ok(());
    };
    let raw_key = crypto::get_raw_master_key().ok_or(TeamKeyError::MissingPersonalKey)?;
    let scope = current_cloud_scope(cx).ok_or(TeamKeyError::MissingTeamKey)?;
    let repo = team_key_cache_repo(cx)?;
    let service = Arc::new(RwLock::new(CloudSyncService::new()));
    let manager = TeamKeyManager::new((*repo).clone(), service, scope);
    match manager.load_cached_team_key_by_id(team_id, &raw_key)? {
        TeamKeyLoadStatus::Unlocked | TeamKeyLoadStatus::LegacyUnlocked => Ok(()),
        TeamKeyLoadStatus::Missing => Err(TeamKeyError::MissingTeamKey),
        TeamKeyLoadStatus::VersionMismatch => Err(TeamKeyError::VersionMismatch),
    }
}

pub fn save_team_key_for_cached_team(
    team_id: &str,
    team_key: &str,
    cx: &App,
) -> Result<(), TeamKeyError> {
    let raw_key = crypto::get_raw_master_key().ok_or(TeamKeyError::MissingPersonalKey)?;
    let scope = current_cloud_scope(cx).ok_or(TeamKeyError::MissingTeamKey)?;
    let repo = team_key_cache_repo(cx)?;
    let service = Arc::new(RwLock::new(CloudSyncService::new()));
    TeamKeyManager::new((*repo).clone(), service, scope)
        .save_key_for_cached_team(team_id, team_key, &raw_key)?;
    Ok(())
}

pub fn forget_team_key_for_cached_team(team_id: &str, cx: &App) -> Result<(), TeamKeyError> {
    let scope = current_cloud_scope(cx).ok_or(TeamKeyError::MissingTeamKey)?;
    let repo = team_key_cache_repo(cx)?;
    let service = Arc::new(RwLock::new(CloudSyncService::new()));
    TeamKeyManager::new((*repo).clone(), service, scope).forget_team_key(team_id)?;
    Ok(())
}

fn team_key_cache_repo(cx: &App) -> Result<Arc<TeamKeyCacheRepository>, TeamKeyError> {
    let storage = cx
        .try_global::<GlobalStorageState>()
        .ok_or_else(|| TeamKeyError::Storage("GlobalStorageState not found".to_string()))?;
    storage
        .storage
        .get::<TeamKeyCacheRepository>()
        .ok_or_else(|| TeamKeyError::Storage("TeamKeyCacheRepository not found".to_string()))
}

fn current_cloud_scope(cx: &App) -> Option<CloudAccountScope> {
    let user = GlobalCloudUser::get_user(cx)?;
    let environment = crate::config::SupabaseConfig::get().project_url;
    Some(CloudAccountScope::new(environment, user.id))
}

// ============================================================================
// 权限判断
// ============================================================================

/// 判断当前用户是否可编辑指定连接
pub fn can_edit_connection(conn: &StoredConnection, cx: &App) -> bool {
    let Some(team_id) = &conn.team_id else {
        return true; // 个人连接，始终可编辑
    };

    let Some(user) = GlobalCloudUser::get_user(cx) else {
        return false; // 未登录，不可编辑团队连接
    };

    let Some(scope) = current_cloud_scope(cx) else {
        return false;
    };
    if let Some(storage) = cx.try_global::<GlobalStorageState>() {
        if let Some(repo) = storage.storage.get::<TeamKeyCacheRepository>() {
            if let Ok(Some(cache)) = repo.get(&scope, team_id) {
                return conn.owner_id.as_deref() == Some(&user.id)
                    || role_can_edit_team_connection(cache.role.as_deref());
            }
        }
    }

    false
}

fn role_can_edit_team_connection(role: Option<&str>) -> bool {
    matches!(role, Some("owner" | "admin"))
}

#[cfg(test)]
mod tests {
    use super::{role_can_edit_team_connection, team_option_from_cache};
    use crate::cloud_sync::{CloudAccountScope, TeamKeyCacheStatus};
    use crate::storage::TeamKeyCache;

    fn team_cache(encrypted_team_key: Option<&str>, last_verified_at: Option<i64>) -> TeamKeyCache {
        TeamKeyCache {
            scope: CloudAccountScope::new("https://project.supabase.co", "user-1"),
            team_id: "team-1".to_string(),
            team_name: "Platform".to_string(),
            key_version: 3,
            cached_key_version: encrypted_team_key.map(|_| 3),
            key_verification: Some("TEAMKEY2:test".to_string()),
            encrypted_team_key: encrypted_team_key.map(str::to_string),
            last_verified_at,
            updated_at: 100,
            role: Some("admin".to_string()),
        }
    }

    #[test]
    fn owner_and_admin_roles_can_edit_team_connections() {
        assert!(role_can_edit_team_connection(Some("owner")));
        assert!(role_can_edit_team_connection(Some("admin")));
    }

    #[test]
    fn member_and_missing_roles_cannot_edit_team_connections() {
        assert!(!role_can_edit_team_connection(Some("member")));
        assert!(!role_can_edit_team_connection(Some("unknown")));
        assert!(!role_can_edit_team_connection(None));
    }

    #[test]
    fn team_option_reports_missing_key_without_cached_secret() {
        let option = team_option_from_cache(team_cache(None, None));

        assert_eq!(TeamKeyCacheStatus::Missing, option.key_status);
        assert_eq!(3, option.key_version);
        assert_eq!(None, option.last_verified_at);
        assert_eq!(Some("admin".to_string()), option.role);
    }

    #[test]
    fn team_option_reports_cached_key_when_secret_was_verified() {
        let option = team_option_from_cache(team_cache(Some("ENC:cached"), Some(1234)));

        assert_eq!(TeamKeyCacheStatus::Cached, option.key_status);
        assert_eq!(Some(1234), option.last_verified_at);
    }
}
