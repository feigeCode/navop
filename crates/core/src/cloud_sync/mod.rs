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

use std::collections::HashMap;
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
use crate::storage::{
    GlobalStorageState, StorageManager, StoredConnection, TeamKeyCache, TeamKeyCacheRepository,
    TeamMembershipCache, TeamMembershipCacheRepository, TeamMembershipState,
};

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
    pub membership_state: TeamMembershipState,
}

fn team_option_from_cache(
    membership: TeamMembershipCache,
    key_cache: Option<TeamKeyCache>,
) -> TeamOption {
    let key_status = key_cache
        .as_ref()
        .map(team_key_cache_status)
        .unwrap_or(TeamKeyCacheStatus::Missing);

    TeamOption {
        id: membership.team_id,
        name: membership.team_name,
        key_status,
        key_version: key_cache.as_ref().map_or(0, |cache| cache.key_version),
        key_verification: key_cache
            .as_ref()
            .and_then(|cache| cache.key_verification.clone()),
        last_verified_at: key_cache.and_then(|cache| cache.last_verified_at),
        role: membership.role,
        membership_state: membership.state,
    }
}

/// 获取当前仍有效且可用于连接归属选择的团队列表。
pub fn get_cached_team_options(cx: &App) -> Vec<TeamOption> {
    let Some(scope) = current_cloud_scope(cx) else {
        return Vec::new();
    };
    let Some(storage) = cx.try_global::<GlobalStorageState>() else {
        return Vec::new();
    };
    get_cached_team_options_for_scope(&storage.storage, &scope)
}

pub fn get_cached_team_options_for_scope(
    storage: &StorageManager,
    scope: &CloudAccountScope,
) -> Vec<TeamOption> {
    team_options_for_scope(storage, scope, true)
}

/// 获取团队展示列表，包括已退出和暂时无法确认成员状态的团队。
pub fn get_cached_team_display_options_for_scope(
    storage: &StorageManager,
    scope: &CloudAccountScope,
) -> Vec<TeamOption> {
    team_options_for_scope(storage, scope, false)
}

fn team_options_for_scope(
    storage: &StorageManager,
    scope: &CloudAccountScope,
    active_only: bool,
) -> Vec<TeamOption> {
    let Some(membership_repo) = storage.get::<TeamMembershipCacheRepository>() else {
        return Vec::new();
    };
    let Some(key_repo) = storage.get::<TeamKeyCacheRepository>() else {
        return Vec::new();
    };
    let memberships = membership_repo.list(scope).unwrap_or_default();
    let key_caches = key_repo
        .list(scope)
        .unwrap_or_default()
        .into_iter()
        .map(|cache| (cache.team_id.clone(), cache))
        .collect::<HashMap<_, _>>();
    memberships
        .into_iter()
        .filter(|membership| !active_only || membership.state == TeamMembershipState::Active)
        .map(|membership| {
            let key_cache = key_caches.get(&membership.team_id).cloned();
            team_option_from_cache(membership, key_cache)
        })
        .collect()
}

pub fn ensure_team_key_ready_for_save(team_id: Option<&str>, cx: &App) -> Result<(), TeamKeyError> {
    let Some(team_id) = team_id else {
        return Ok(());
    };
    let raw_key = crypto::get_raw_master_key().ok_or(TeamKeyError::MissingPersonalKey)?;
    let scope = current_cloud_scope(cx).ok_or(TeamKeyError::MissingTeamKey)?;
    let membership_repo = team_membership_cache_repo(cx)?;
    let membership = membership_repo
        .get(&scope, team_id)
        .map_err(|error| TeamKeyError::Storage(error.to_string()))?
        .ok_or(TeamKeyError::MissingTeamKey)?;
    if membership.state != TeamMembershipState::Active {
        return Err(TeamKeyError::MissingTeamKey);
    }
    let repo = team_key_cache_repo(cx)?;
    let service = Arc::new(RwLock::new(CloudSyncService::new()));
    let manager = TeamKeyManager::new((*repo).clone(), service, scope);
    match manager.load_cached_team_key_by_id(team_id, &raw_key)? {
        TeamKeyLoadStatus::Unlocked => Ok(()),
        TeamKeyLoadStatus::Missing => Err(TeamKeyError::MissingTeamKey),
        TeamKeyLoadStatus::VersionMismatch => Err(TeamKeyError::VersionMismatch),
        TeamKeyLoadStatus::Invalid => Err(TeamKeyError::InvalidTeamKey),
    }
}

fn team_membership_cache_repo(
    cx: &App,
) -> Result<Arc<TeamMembershipCacheRepository>, TeamKeyError> {
    let storage = cx
        .try_global::<GlobalStorageState>()
        .ok_or_else(|| TeamKeyError::Storage("GlobalStorageState not found".to_string()))?;
    storage
        .storage
        .get::<TeamMembershipCacheRepository>()
        .ok_or_else(|| TeamKeyError::Storage("TeamMembershipCacheRepository not found".to_string()))
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

    let Some(_user) = GlobalCloudUser::get_user(cx) else {
        return false; // 未登录，不可编辑团队连接
    };

    let Some(scope) = current_cloud_scope(cx) else {
        return false;
    };
    if let Some(storage) = cx.try_global::<GlobalStorageState>() {
        if let Some(repo) = storage.storage.get::<TeamMembershipCacheRepository>() {
            if let Ok(Some(cache)) = repo.get(&scope, team_id) {
                return team_membership_can_edit(cache.state, cache.role.as_deref());
            }
        }
    }

    false
}

fn role_can_edit_team_connection(role: Option<&str>) -> bool {
    matches!(role, Some("owner" | "admin"))
}

fn team_membership_can_edit(state: TeamMembershipState, role: Option<&str>) -> bool {
    state == TeamMembershipState::Active && role_can_edit_team_connection(role)
}

#[cfg(test)]
mod tests {
    use super::{
        get_cached_team_display_options_for_scope, get_cached_team_options_for_scope,
        role_can_edit_team_connection, team_membership_can_edit, team_option_from_cache,
    };
    use crate::cloud_sync::team_key_envelope::{TeamKeyKdfParams, create_team_key_envelope};
    use crate::cloud_sync::{CloudAccountScope, TeamKeyCacheStatus};
    use crate::storage::connection::SqliteConnection;
    use crate::storage::migration::run_migrations;
    use crate::storage::{
        StorageManager, TeamKeyCache, TeamKeyCacheRepository, TeamMembershipCache,
        TeamMembershipCacheRepository, TeamMembershipState,
    };

    fn team_cache(encrypted_team_key: Option<&str>, last_verified_at: Option<i64>) -> TeamKeyCache {
        TeamKeyCache {
            scope: CloudAccountScope::new("https://project.supabase.co", "user-1"),
            team_id: "team-1".to_string(),
            team_name: "Platform".to_string(),
            key_version: 3,
            cached_key_version: encrypted_team_key.map(|_| 3),
            key_verification: Some(
                create_team_key_envelope(
                    "correct horse battery staple",
                    TeamKeyKdfParams::for_tests(),
                )
                .expect("create envelope")
                .verification,
            ),
            encrypted_team_key: encrypted_team_key.map(str::to_string),
            last_verified_at,
            updated_at: 100,
            role: Some("admin".to_string()),
        }
    }

    fn membership() -> TeamMembershipCache {
        TeamMembershipCache {
            scope: CloudAccountScope::new("https://project.supabase.co", "user-1"),
            team_id: "team-1".to_string(),
            team_name: "Platform".to_string(),
            role: Some("admin".to_string()),
            state: TeamMembershipState::Active,
            last_seen_at: Some(100),
            updated_at: 100,
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
    fn departed_and_unknown_memberships_cannot_edit_even_with_admin_role() {
        assert!(team_membership_can_edit(
            TeamMembershipState::Active,
            Some("admin")
        ));
        assert!(!team_membership_can_edit(
            TeamMembershipState::Departed,
            Some("admin")
        ));
        assert!(!team_membership_can_edit(
            TeamMembershipState::Unknown,
            Some("owner")
        ));
    }

    #[test]
    fn team_option_reports_missing_key_without_cached_secret() {
        let option = team_option_from_cache(membership(), Some(team_cache(None, None)));

        assert_eq!(TeamKeyCacheStatus::Missing, option.key_status);
        assert_eq!(3, option.key_version);
        assert_eq!(None, option.last_verified_at);
        assert_eq!(Some("admin".to_string()), option.role);
    }

    #[test]
    fn team_option_reports_cached_key_when_secret_was_verified() {
        let option = team_option_from_cache(
            membership(),
            Some(team_cache(Some("ENC:cached"), Some(1234))),
        );

        assert_eq!(TeamKeyCacheStatus::Cached, option.key_status);
        assert_eq!(Some(1234), option.last_verified_at);
    }

    #[test]
    fn selectable_teams_exclude_departed_but_display_cache_retains_them() {
        let temp = tempfile::tempdir().expect("temp directory");
        let connection = SqliteConnection::open(temp.path().join("teams.db")).expect("open sqlite");
        connection
            .with_connection(run_migrations)
            .expect("run migrations");
        let storage = StorageManager::new_with_connection(connection.clone());
        let key_repo = TeamKeyCacheRepository::new(connection.clone());
        let membership_repo = TeamMembershipCacheRepository::new(connection);
        storage.register(key_repo.clone());
        storage.register(membership_repo.clone());
        let scope = CloudAccountScope::new("https://project.supabase.co", "user-1");
        membership_repo.upsert(&membership()).expect("active team");
        membership_repo
            .upsert(&TeamMembershipCache {
                team_id: "team-2".to_string(),
                team_name: "Former Team".to_string(),
                role: None,
                state: TeamMembershipState::Departed,
                ..membership()
            })
            .expect("departed team");
        key_repo
            .upsert(&team_cache(Some("ENC:cached"), Some(1234)))
            .expect("active key");

        let selectable = get_cached_team_options_for_scope(&storage, &scope);
        let display = get_cached_team_display_options_for_scope(&storage, &scope);

        assert_eq!(
            vec!["team-1"],
            selectable
                .iter()
                .map(|team| team.id.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(2, display.len());
        assert!(display.iter().any(|team| {
            team.id == "team-2" && team.membership_state == TeamMembershipState::Departed
        }));
    }
}
