//! 团队管理引擎接口测试
//!
//! 验证 `SyncEngine` 的团队管理方法：写操作透传到云客户端、服务端报错被
//! 映射成中文（`SyncError::TeamAdmin`），以及写成功后的本地缓存刷新。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use llm_connector::ChatRequest;

use crate::cloud_sync::client::{
    AuthResponse, CloudApiClient, CloudApiError, OAuthResponse, UserInfo,
};
use crate::cloud_sync::models::{
    CloudSyncData, CloudUserConfig, Team, TeamMember, TeamMemberWithEmail, TeamRole,
};
use crate::cloud_sync::service::SyncError;
use crate::cloud_sync::{CloudSyncService, SyncEngine};
use crate::llm::ChatStream;
use crate::storage::connection::SqliteConnection;
use crate::storage::migration::run_migrations;
use crate::storage::{StorageManager, TeamKeyCacheRepository, TeamMembershipCacheRepository};

/// 记录团队管理调用参数
#[derive(Default)]
struct TeamCalls {
    create: Mutex<Vec<(String, Option<String>)>>,
    add: Mutex<Vec<(String, String, TeamRole)>>,
    set_role: Mutex<Vec<(String, String, TeamRole)>>,
    remove: Mutex<Vec<(String, String)>>,
    leave: Mutex<Vec<String>>,
    list_teams: AtomicUsize,
}

fn not_used() -> CloudApiError {
    CloudApiError::Unknown("not used".to_string())
}

struct FakeTeamClient {
    calls: Arc<TeamCalls>,
    teams: Vec<Team>,
    created_team_id: String,
    updated_team: Option<Team>,
    members: Vec<TeamMemberWithEmail>,
    team_error: Option<CloudApiError>,
}

#[async_trait]
impl CloudApiClient for FakeTeamClient {
    async fn sign_in_with_password(
        &self,
        _email: &str,
        _password: &str,
    ) -> Result<AuthResponse, CloudApiError> {
        Err(not_used())
    }

    async fn sign_in_with_oauth(
        &self,
        _provider: &str,
        _redirect_url: &str,
    ) -> Result<OAuthResponse, CloudApiError> {
        Err(not_used())
    }

    async fn sign_up(&self, _email: &str, _password: &str) -> Result<AuthResponse, CloudApiError> {
        Err(not_used())
    }

    async fn sign_out(&self) -> Result<(), CloudApiError> {
        Err(not_used())
    }

    async fn get_current_user(&self) -> Result<Option<UserInfo>, CloudApiError> {
        Err(not_used())
    }

    async fn refresh_token(&self, _refresh_token: &str) -> Result<AuthResponse, CloudApiError> {
        Err(not_used())
    }

    async fn sign_in_with_otp(&self, _email: &str) -> Result<(), CloudApiError> {
        Err(not_used())
    }

    async fn verify_otp(&self, _email: &str, _token: &str) -> Result<AuthResponse, CloudApiError> {
        Err(not_used())
    }

    async fn get_user_config(&self) -> Result<Option<CloudUserConfig>, CloudApiError> {
        Err(not_used())
    }

    async fn save_user_config(&self, _config: &CloudUserConfig) -> Result<(), CloudApiError> {
        Err(not_used())
    }

    async fn get_subscription(
        &self,
    ) -> Result<Option<crate::license::SubscriptionInfo>, CloudApiError> {
        Err(not_used())
    }

    async fn list_models(&self) -> Result<Vec<String>, CloudApiError> {
        Err(not_used())
    }

    async fn list_sync_data(
        &self,
        _data_type: Option<&str>,
        _team_id: Option<&str>,
        _since: Option<i64>,
    ) -> Result<Vec<CloudSyncData>, CloudApiError> {
        Err(not_used())
    }

    async fn create_sync_data(
        &self,
        _data: &CloudSyncData,
    ) -> Result<CloudSyncData, CloudApiError> {
        Err(not_used())
    }

    async fn update_sync_data(
        &self,
        _data: &CloudSyncData,
    ) -> Result<CloudSyncData, CloudApiError> {
        Err(not_used())
    }

    async fn delete_sync_data(&self, _id: &str) -> Result<(), CloudApiError> {
        Err(not_used())
    }

    async fn list_teams(&self) -> Result<Vec<Team>, CloudApiError> {
        self.calls.list_teams.fetch_add(1, Ordering::SeqCst);
        Ok(self.teams.clone())
    }

    async fn list_team_members(&self, _team_id: &str) -> Result<Vec<TeamMember>, CloudApiError> {
        Ok(Vec::new())
    }

    async fn list_current_user_team_members(&self) -> Result<Vec<TeamMember>, CloudApiError> {
        Ok(Vec::new())
    }

    async fn rotate_team_key(
        &self,
        _team: &Team,
        _records: &[CloudSyncData],
    ) -> Result<(), CloudApiError> {
        Err(not_used())
    }

    async fn initialize_team_key(&self, _team: &Team) -> Result<Team, CloudApiError> {
        Err(not_used())
    }

    async fn create_team_with_owner(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<String, CloudApiError> {
        if let Some(error) = &self.team_error {
            return Err(clone_error(error));
        }
        self.calls
            .create
            .lock()
            .expect("create calls lock")
            .push((name.to_string(), description.map(str::to_string)));
        Ok(self.created_team_id.clone())
    }

    async fn update_team_profile(
        &self,
        _team_id: &str,
        _name: &str,
        _description: Option<&str>,
    ) -> Result<Team, CloudApiError> {
        if let Some(error) = &self.team_error {
            return Err(clone_error(error));
        }
        self.updated_team.clone().ok_or_else(not_used)
    }

    async fn add_team_member_by_email(
        &self,
        team_id: &str,
        email: &str,
        role: TeamRole,
    ) -> Result<String, CloudApiError> {
        if let Some(error) = &self.team_error {
            return Err(clone_error(error));
        }
        self.calls.add.lock().expect("add calls lock").push((
            team_id.to_string(),
            email.to_string(),
            role,
        ));
        Ok("member-new".to_string())
    }

    async fn list_team_members_with_email(
        &self,
        _team_id: &str,
    ) -> Result<Vec<TeamMemberWithEmail>, CloudApiError> {
        if let Some(error) = &self.team_error {
            return Err(clone_error(error));
        }
        Ok(self.members.clone())
    }

    async fn set_team_member_role(
        &self,
        team_id: &str,
        member_id: &str,
        role: TeamRole,
    ) -> Result<(), CloudApiError> {
        if let Some(error) = &self.team_error {
            return Err(clone_error(error));
        }
        self.calls
            .set_role
            .lock()
            .expect("set role calls lock")
            .push((team_id.to_string(), member_id.to_string(), role));
        Ok(())
    }

    async fn remove_team_member(
        &self,
        team_id: &str,
        member_id: &str,
    ) -> Result<(), CloudApiError> {
        if let Some(error) = &self.team_error {
            return Err(clone_error(error));
        }
        self.calls
            .remove
            .lock()
            .expect("remove calls lock")
            .push((team_id.to_string(), member_id.to_string()));
        Ok(())
    }

    async fn leave_team(&self, team_id: &str) -> Result<(), CloudApiError> {
        if let Some(error) = &self.team_error {
            return Err(clone_error(error));
        }
        self.calls
            .leave
            .lock()
            .expect("leave calls lock")
            .push(team_id.to_string());
        Ok(())
    }

    async fn chat(&self, _request: &ChatRequest) -> Result<String, CloudApiError> {
        Err(not_used())
    }

    async fn chat_stream(&self, _request: &ChatRequest) -> Result<ChatStream, CloudApiError> {
        Err(not_used())
    }
}

/// `CloudApiError` 不是 `Clone`，测试里按变体重建
fn clone_error(error: &CloudApiError) -> CloudApiError {
    match error {
        CloudApiError::NotAuthenticated => CloudApiError::NotAuthenticated,
        CloudApiError::AuthenticationFailed(message) => {
            CloudApiError::AuthenticationFailed(message.clone())
        }
        CloudApiError::EmailConfirmationRequired(message) => {
            CloudApiError::EmailConfirmationRequired(message.clone())
        }
        CloudApiError::NetworkError(message) => CloudApiError::NetworkError(message.clone()),
        CloudApiError::ServerError(message) => CloudApiError::ServerError(message.clone()),
        CloudApiError::ParseError(message) => CloudApiError::ParseError(message.clone()),
        CloudApiError::NotFound(message) => CloudApiError::NotFound(message.clone()),
        CloudApiError::Conflict(message) => CloudApiError::Conflict(message.clone()),
        CloudApiError::Unknown(message) => CloudApiError::Unknown(message.clone()),
    }
}

fn team(name: &str) -> Team {
    Team {
        id: "team-1".to_string(),
        name: name.to_string(),
        owner_id: "user-1".to_string(),
        description: None,
        key_verification: None,
        key_version: 1,
        created_at: 1_000,
        updated_at: 1_000,
    }
}

fn test_engine(client: FakeTeamClient) -> (SyncEngine, StorageManager) {
    // 纳秒时间戳在 macOS 上步进仅 1µs，12 个测试线程同时启动时极易取到
    // 同一时刻 → 同名 db 文件 → 两个连接对同一文件并发跑迁移，撞
    // `_migrations` 主键（UNIQUE constraint failed）。 AtomicUsize 计数器
    // 保证文件名在进程内绝对唯一（与 usage_history_tests 等处同款模式）。
    static DB_COUNTER: AtomicUsize = AtomicUsize::new(0);
    let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!(
        "onetcli-team-admin-engine-{}-{}-{counter}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&db_path);
    let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
    conn.with_connection(|conn| run_migrations(conn))
        .expect("run migrations");
    let storage = StorageManager::new_with_connection(conn.clone());
    storage.register(TeamKeyCacheRepository::new(conn.clone()));
    storage.register(TeamMembershipCacheRepository::new(conn));

    let mut service = CloudSyncService::new();
    service.set_logged_in("user-1".to_string());
    let service = Arc::new(RwLock::new(service));

    (
        SyncEngine::new(Arc::new(client), service, storage.clone()),
        storage,
    )
}

fn fake_client(teams: Vec<Team>) -> FakeTeamClient {
    FakeTeamClient {
        calls: Arc::new(TeamCalls::default()),
        teams,
        created_team_id: "team-new".to_string(),
        updated_team: None,
        members: Vec::new(),
        team_error: None,
    }
}

fn team_error(message: &str) -> Option<CloudApiError> {
    Some(CloudApiError::ServerError(message.to_string()))
}

#[tokio::test]
async fn create_team_returns_id_and_refreshes_team_cache() {
    let _crypto_guard = crate::crypto::crypto_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let client = fake_client(vec![team("Platform")]);
    let calls = client.calls.clone();
    let (engine, _storage) = test_engine(client);

    let team_id = engine
        .create_team("Platform", Some("平台组"))
        .await
        .expect("create succeeds");

    assert_eq!("team-new", team_id);
    assert_eq!(
        vec![("Platform".to_string(), Some("平台组".to_string()))],
        *calls.create.lock().expect("create calls lock")
    );
    // 写操作之后会刷新本地团队缓存
    assert_eq!(1, calls.list_teams.load(Ordering::SeqCst));
}

#[tokio::test]
async fn create_team_maps_server_message_to_chinese() {
    let mut client = fake_client(Vec::new());
    client.team_error = team_error("Team name is required");
    let (engine, _storage) = test_engine(client);

    let error = engine
        .create_team("", None)
        .await
        .expect_err("rejected create must fail");

    match error {
        SyncError::TeamAdmin(message) => assert_eq!("Team name is required", message),
        other => panic!("unexpected error: {other}"),
    }
}

#[tokio::test]
async fn update_team_profile_updates_cached_team_without_refetch() {
    let _crypto_guard = crate::crypto::crypto_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut client = fake_client(vec![team("Platform")]);
    client.updated_team = Some(team("Renamed"));
    let calls = client.calls.clone();
    let (engine, _storage) = test_engine(client);

    engine
        .refresh_team_key_cache()
        .await
        .expect("initial refresh");
    assert_eq!(1, calls.list_teams.load(Ordering::SeqCst));

    engine
        .update_team_profile("team-1", "Renamed", None)
        .await
        .expect("profile update succeeds");

    let cached = engine.get_cached_teams();
    assert_eq!(1, cached.len());
    assert_eq!("Renamed", cached[0].name);
    // 服务端已经回传完整团队记录，不应再拉一次列表
    assert_eq!(1, calls.list_teams.load(Ordering::SeqCst));
}

#[tokio::test]
async fn add_team_member_maps_permission_error() {
    let mut client = fake_client(Vec::new());
    client.team_error = team_error("Only owners can add owners");
    let (engine, _storage) = test_engine(client);

    let error = engine
        .add_team_member("team-1", "member@example.com", TeamRole::Member)
        .await
        .expect_err("non-owner add must fail");

    assert_eq!("只有团队所有者可以添加所有者", error.to_string());
    assert!(matches!(error, SyncError::TeamAdmin(_)));
}

#[tokio::test]
async fn add_team_member_forwards_team_email_and_role() {
    let client = fake_client(Vec::new());
    let calls = client.calls.clone();
    let (engine, _storage) = test_engine(client);

    engine
        .add_team_member("team-1", "member@example.com", TeamRole::Admin)
        .await
        .expect("add succeeds");

    assert_eq!(
        vec![(
            "team-1".to_string(),
            "member@example.com".to_string(),
            TeamRole::Admin
        )],
        *calls.add.lock().expect("add calls lock")
    );
}

#[tokio::test]
async fn set_team_member_role_forwards_ids_and_role() {
    let client = fake_client(Vec::new());
    let calls = client.calls.clone();
    let (engine, _storage) = test_engine(client);

    engine
        .set_team_member_role("team-1", "member-1", TeamRole::Owner)
        .await
        .expect("role update succeeds");

    assert_eq!(
        vec![(
            "team-1".to_string(),
            "member-1".to_string(),
            TeamRole::Owner
        )],
        *calls.set_role.lock().expect("set role calls lock")
    );
}

#[tokio::test]
async fn set_team_member_role_maps_assign_owner_error() {
    let mut client = fake_client(Vec::new());
    client.team_error = team_error("Only owners can assign owners");
    let (engine, _storage) = test_engine(client);

    let error = engine
        .set_team_member_role("team-1", "member-1", TeamRole::Owner)
        .await
        .expect_err("assign owner must fail");

    assert_eq!("只有团队所有者可以指派所有者", error.to_string());
}

#[tokio::test]
async fn remove_team_member_maps_owner_removal_error() {
    let mut client = fake_client(Vec::new());
    client.team_error = team_error("Owners cannot be removed from the website console");
    let (engine, _storage) = test_engine(client);

    let error = engine
        .remove_team_member("team-1", "owner-member")
        .await
        .expect_err("removing owner must fail");

    assert_eq!(
        "不能直接移除团队所有者，请先把所有者降级",
        error.to_string()
    );
}

#[tokio::test]
async fn remove_team_member_forwards_ids() {
    let client = fake_client(Vec::new());
    let calls = client.calls.clone();
    let (engine, _storage) = test_engine(client);

    engine
        .remove_team_member("team-1", "member-1")
        .await
        .expect("remove succeeds");

    assert_eq!(
        vec![("team-1".to_string(), "member-1".to_string())],
        *calls.remove.lock().expect("remove calls lock")
    );
}

#[tokio::test]
async fn leave_team_forwards_team_id_and_refreshes_cache() {
    let client = fake_client(vec![team("Platform")]);
    let calls = client.calls.clone();
    let (engine, _storage) = test_engine(client);

    engine.leave_team("team-1").await.expect("leave succeeds");

    assert_eq!(
        vec!["team-1".to_string()],
        *calls.leave.lock().expect("leave calls lock")
    );
    assert_eq!(1, calls.list_teams.load(Ordering::SeqCst));
}

#[tokio::test]
async fn leave_team_maps_owner_self_demotion_error() {
    let mut client = fake_client(Vec::new());
    client.team_error = team_error("Owners cannot change their own role");
    let (engine, _storage) = test_engine(client);

    let error = engine
        .leave_team("team-1")
        .await
        .expect_err("owner cannot leave");

    assert_eq!(
        "拥有者不能移除自己的拥有者角色，请先转让所有权",
        error.to_string()
    );
}

#[tokio::test]
async fn list_team_members_with_email_returns_members() {
    let mut client = fake_client(Vec::new());
    client.members = vec![TeamMemberWithEmail {
        id: "member-1".to_string(),
        team_id: "team-1".to_string(),
        user_id: "user-1".to_string(),
        email: "owner@example.com".to_string(),
        role: TeamRole::Owner,
        joined_at: 1_000,
    }];
    let (engine, _storage) = test_engine(client);

    let members = engine
        .list_team_members_with_email("team-1")
        .await
        .expect("list succeeds");

    assert_eq!(1, members.len());
    assert_eq!("owner@example.com", members[0].email);
    assert_eq!(TeamRole::Owner, members[0].role);
}

#[tokio::test]
async fn list_team_members_with_email_maps_membership_error() {
    let mut client = fake_client(Vec::new());
    client.team_error = team_error("Team membership required");
    let (engine, _storage) = test_engine(client);

    let error = engine
        .list_team_members_with_email("team-1")
        .await
        .expect_err("non-member must fail");

    assert_eq!("你不是该团队成员", error.to_string());
}
