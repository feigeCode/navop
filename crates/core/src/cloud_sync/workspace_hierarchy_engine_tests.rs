//! 工作空间层级同步的引擎级端到端测试
//!
//! 与单元测试不同，这里走完整链路：`list_local`（祖先优先）→ 同步计划 →
//! 操作队列 → 上传 / 下载 → 收尾对齐，即 `generic_sync` 的真实执行路径。
//!
//! 场景对齐 issue #356：在另一台设备上同步后，递归创建的分组必须仍是嵌套的。

use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use llm_connector::ChatRequest;

use crate::cloud_sync::client::{
    AuthResponse, CloudApiClient, CloudApiError, OAuthResponse, UserInfo,
};
use crate::cloud_sync::models::{
    CloudSyncData, CloudUserConfig, SyncResult, Team, TeamMember, WorkspaceParentLink, data_type,
};
use crate::cloud_sync::{CloudSyncService, SyncEngine};
use crate::llm::ChatStream;
use crate::storage::connection::SqliteConnection;
use crate::storage::migration::run_migrations;
use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionRepository, CredentialRepository, PendingCloudDeletionRepository, StorageManager,
    TeamKeyCacheRepository, TeamMembershipCacheRepository, Workspace, WorkspaceRepository,
};

const TEST_MASTER_KEY: &str = "workspace-hierarchy-engine-test-master-key";
const TEST_USER_ID: &str = "user-1";

/// 假云端：内存里存一份 sync_data，模拟多设备共享同一份云端数据
#[derive(Default)]
struct FakeCloudStore {
    records: Mutex<Vec<CloudSyncData>>,
}

impl FakeCloudStore {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn records_of_type(&self, data_type: &str) -> Vec<CloudSyncData> {
        self.records
            .lock()
            .expect("cloud store lock")
            .iter()
            .filter(|record| record.data_type == data_type)
            .cloned()
            .collect()
    }

    fn upsert(&self, record: &CloudSyncData) {
        let mut records = self.records.lock().expect("cloud store lock");
        match records.iter_mut().find(|existing| existing.id == record.id) {
            Some(existing) => *existing = record.clone(),
            None => records.push(record.clone()),
        }
    }

    fn remove(&self, id: &str) {
        self.records
            .lock()
            .expect("cloud store lock")
            .retain(|record| record.id != id);
    }

    fn record(&self, id: &str) -> CloudSyncData {
        self.records
            .lock()
            .expect("cloud store lock")
            .iter()
            .find(|record| record.id == id)
            .cloned()
            .unwrap_or_else(|| panic!("cloud record {id} not found"))
    }
}

struct FakeCloudClient {
    store: Arc<FakeCloudStore>,
}

fn not_used() -> CloudApiError {
    CloudApiError::Unknown("not used".to_string())
}

#[async_trait]
impl CloudApiClient for FakeCloudClient {
    fn environment_id(&self) -> &str {
        "test-environment"
    }

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
        data_type: Option<&str>,
        team_id: Option<&str>,
        _since: Option<i64>,
    ) -> Result<Vec<CloudSyncData>, CloudApiError> {
        let records = self
            .records_locked()
            .iter()
            .filter(|record| data_type.is_none_or(|kind| record.data_type == kind))
            .filter(|record| record.team_id.as_deref() == team_id)
            .cloned()
            .collect();
        Ok(records)
    }

    async fn create_sync_data(&self, data: &CloudSyncData) -> Result<CloudSyncData, CloudApiError> {
        self.store.upsert(data);
        Ok(data.clone())
    }

    async fn update_sync_data(&self, data: &CloudSyncData) -> Result<CloudSyncData, CloudApiError> {
        self.store.upsert(data);
        Ok(data.clone())
    }

    async fn delete_sync_data(&self, id: &str) -> Result<(), CloudApiError> {
        self.store.remove(id);
        Ok(())
    }

    async fn list_teams(&self) -> Result<Vec<Team>, CloudApiError> {
        Ok(Vec::new())
    }

    async fn list_team_members(&self, _team_id: &str) -> Result<Vec<TeamMember>, CloudApiError> {
        Ok(Vec::new())
    }

    async fn list_current_user_team_members(&self) -> Result<Vec<TeamMember>, CloudApiError> {
        Ok(Vec::new())
    }

    async fn initialize_team_key(&self, _team: &Team) -> Result<Team, CloudApiError> {
        Err(not_used())
    }

    async fn rotate_team_key(
        &self,
        _team: &Team,
        _records: &[CloudSyncData],
    ) -> Result<(), CloudApiError> {
        Err(not_used())
    }

    async fn chat(&self, _request: &ChatRequest) -> Result<String, CloudApiError> {
        Err(not_used())
    }

    async fn chat_stream(&self, _request: &ChatRequest) -> Result<ChatStream, CloudApiError> {
        Err(not_used())
    }
}

impl FakeCloudClient {
    fn records_locked(&self) -> Vec<CloudSyncData> {
        self.store.records.lock().expect("cloud store lock").clone()
    }
}

/// 一台设备：独立本地库 + 独立同步引擎，共享同一份假云端数据
struct Device {
    _temp: tempfile::TempDir,
    engine: SyncEngine,
    workspaces: WorkspaceRepository,
    service: Arc<RwLock<CloudSyncService>>,
}

impl Device {
    fn new(store: Arc<FakeCloudStore>) -> Self {
        let temp = tempfile::tempdir().expect("temp directory");
        let connection =
            SqliteConnection::open(temp.path().join("device.db")).expect("open sqlite");
        connection
            .with_connection(run_migrations)
            .expect("migrations run");

        let storage = StorageManager::new_with_connection(connection.clone());
        let workspaces = WorkspaceRepository::new(connection.clone());
        storage.register(workspaces.clone());
        storage.register(ConnectionRepository::new(connection.clone()));
        storage.register(CredentialRepository::new(connection.clone()));
        storage.register(PendingCloudDeletionRepository::new(connection.clone()));
        storage.register(TeamKeyCacheRepository::new(connection.clone()));
        storage.register(TeamMembershipCacheRepository::new(connection));

        let mut service = CloudSyncService::new();
        service.set_logged_in(TEST_USER_ID.to_string());
        service.set_master_key_directly(TEST_MASTER_KEY.to_string());
        let service = Arc::new(RwLock::new(service));

        let engine = SyncEngine::new(
            Arc::new(FakeCloudClient { store }),
            service.clone(),
            storage,
        );

        Self {
            _temp: temp,
            engine,
            workspaces,
            service,
        }
    }

    fn insert_workspace(&self, name: &str, parent_id: Option<i64>) -> i64 {
        let mut workspace = Workspace::new(name.to_string());
        workspace.parent_id = parent_id;
        self.workspaces
            .insert(&mut workspace)
            .expect("workspace insert")
    }

    fn workspace(&self, local_id: i64) -> Workspace {
        self.workspaces
            .get(local_id)
            .expect("workspace query")
            .expect("workspace exists")
    }

    fn workspace_by_cloud_id(&self, cloud_id: &str) -> Workspace {
        self.workspaces
            .list()
            .expect("workspace list")
            .into_iter()
            .find(|workspace| workspace.cloud_id.as_deref() == Some(cloud_id))
            .unwrap_or_else(|| panic!("本地不存在云端 ID 为 {cloud_id} 的分组"))
    }

    fn cloud_id(&self, local_id: i64) -> String {
        self.workspace(local_id)
            .cloud_id
            .expect("分组同步后应已取得云端 ID")
    }

    fn decrypt_link(&self, record: &CloudSyncData) -> WorkspaceParentLink {
        self.service
            .read()
            .expect("service lock")
            .decrypt_sync_data_workspace_with_parent_link(record)
            .expect("decrypt workspace payload")
            .1
    }

    async fn sync(&self) -> SyncResult {
        let result = self.engine.sync().await.expect("同步成功");
        assert!(
            result.errors.is_empty(),
            "同步过程中不应出现错误: {:?}",
            result.errors
        );
        result
    }
}

/// 旧版本客户端上传的分组载荷：只有 name / color / icon，不含层级信息
fn legacy_workspace_record(
    device: &Device,
    cloud_id: &str,
    name: &str,
    updated_at_millis: i64,
) -> CloudSyncData {
    let payload = format!(r#"{{"name":"{name}","color":null,"icon":null}}"#);
    let encrypted_data = device
        .service
        .read()
        .expect("service lock")
        .encrypt_blob(&payload, None)
        .expect("legacy payload encryption");

    CloudSyncData {
        id: cloud_id.to_string(),
        owner_id: TEST_USER_ID.to_string(),
        team_id: None,
        data_type: data_type::WORKSPACE.to_string(),
        encrypted_data,
        key_version: 1,
        checksum: String::new(),
        version: 1,
        updated_at: updated_at_millis,
        deleted_at: None,
    }
}

/// 本地分组当前的更新时间戳（秒），用于构造与云端一致的旧记录
fn local_updated_seconds(device: &Device, local_id: i64) -> i64 {
    device.workspace(local_id).updated_at.unwrap_or(0)
}

/// 新版本格式的「明确根分组」载荷
fn root_payload(device: &Device, name: &str) -> String {
    let workspace = Workspace::new(name.to_string());
    device
        .service
        .read()
        .expect("service lock")
        .prepare_workspace_sync_data_upload(&workspace, WorkspaceParentLink::Root, None, &[])
        .expect("root payload")
        .encrypted_data
}

#[tokio::test]
async fn engine_upload_records_recursive_group_hierarchy_in_cloud() {
    let store = FakeCloudStore::new();
    let device = Device::new(store.clone());
    let root = device.insert_workspace("A", None);
    let middle = device.insert_workspace("B", Some(root));
    let leaf = device.insert_workspace("C", Some(middle));

    let result = device.sync().await;

    assert_eq!(3, result.uploaded);
    let records = store.records_of_type(data_type::WORKSPACE);
    assert_eq!(3, records.len());
    // 父分组先上传并拿到云端 ID，子分组随后才能写入稳定引用
    assert_eq!(
        WorkspaceParentLink::Root,
        device.decrypt_link(&store.record(&device.cloud_id(root)))
    );
    assert_eq!(
        WorkspaceParentLink::Cloud(device.cloud_id(root)),
        device.decrypt_link(&store.record(&device.cloud_id(middle)))
    );
    assert_eq!(
        WorkspaceParentLink::Cloud(device.cloud_id(middle)),
        device.decrypt_link(&store.record(&device.cloud_id(leaf)))
    );
}

#[tokio::test]
async fn engine_download_restores_recursive_nesting_on_another_device() {
    let store = FakeCloudStore::new();
    let first = Device::new(store.clone());
    let first_root = first.insert_workspace("A", None);
    let first_middle = first.insert_workspace("B", Some(first_root));
    let first_leaf = first.insert_workspace("C", Some(first_middle));
    first.sync().await;
    let (root_cloud_id, middle_cloud_id, leaf_cloud_id) = (
        first.cloud_id(first_root),
        first.cloud_id(first_middle),
        first.cloud_id(first_leaf),
    );

    // 另一台电脑：全新本地库，同一把主密钥、同一份云端数据
    let second = Device::new(store.clone());
    let result = second.sync().await;

    assert_eq!(3, result.downloaded);
    let root = second.workspace_by_cloud_id(&root_cloud_id);
    let middle = second.workspace_by_cloud_id(&middle_cloud_id);
    let leaf = second.workspace_by_cloud_id(&leaf_cloud_id);
    assert_eq!(None, root.parent_id);
    assert_eq!(root.id, middle.parent_id);
    assert_eq!(middle.id, leaf.parent_id);
}

#[tokio::test]
async fn engine_sync_upgrades_legacy_cloud_payload_without_flattening_local_tree() {
    let store = FakeCloudStore::new();
    let device = Device::new(store.clone());
    let root = device.insert_workspace("A", None);
    let leaf = device.insert_workspace("B", Some(root));
    device.sync().await;

    // 模拟旧版本客户端留下的云端记录：无层级信息，且更新时间与本地一致
    let root_cloud_id = device.cloud_id(root);
    let leaf_cloud_id = device.cloud_id(leaf);
    store.upsert(&legacy_workspace_record(
        &device,
        &root_cloud_id,
        "A",
        local_updated_seconds(&device, root) * 1000,
    ));
    store.upsert(&legacy_workspace_record(
        &device,
        &leaf_cloud_id,
        "B",
        local_updated_seconds(&device, leaf) * 1000,
    ));

    let result = device.sync().await;

    // 旧载荷没有层级信息 → 本地嵌套保持不动；子分组补一次带层级的上传
    // （根分组本来就没有父分组，无需补传）
    assert_eq!(1, result.uploaded);
    assert_eq!(Some(root), device.workspace(leaf).parent_id);
    assert_eq!(
        WorkspaceParentLink::Cloud(root_cloud_id.clone()),
        device.decrypt_link(&store.record(&leaf_cloud_id))
    );
    assert_eq!(
        WorkspaceParentLink::Unknown,
        device.decrypt_link(&store.record(&root_cloud_id))
    );
}

#[tokio::test]
async fn engine_sync_keeps_local_nesting_when_legacy_cloud_payload_is_newer() {
    let store = FakeCloudStore::new();
    let device = Device::new(store.clone());
    let root = device.insert_workspace("A", None);
    let leaf = device.insert_workspace("B", Some(root));
    device.sync().await;

    // 云端更新时间更晚 → 走「更新本地」分支，旧载荷必须保留本地父子关系
    let root_cloud_id = device.cloud_id(root);
    let leaf_cloud_id = device.cloud_id(leaf);
    let newer = (local_updated_seconds(&device, leaf) + 3_600) * 1000;
    store.upsert(&legacy_workspace_record(
        &device,
        &root_cloud_id,
        "A",
        newer,
    ));
    store.upsert(&legacy_workspace_record(
        &device,
        &leaf_cloud_id,
        "B",
        newer,
    ));

    let result = device.sync().await;

    assert_eq!(2, result.downloaded);
    assert_eq!(Some(root), device.workspace(leaf).parent_id);
}

#[tokio::test]
async fn engine_sync_clears_local_parent_when_cloud_group_was_moved_to_root() {
    let store = FakeCloudStore::new();
    let first = Device::new(store.clone());
    let first_root = first.insert_workspace("A", None);
    let first_leaf = first.insert_workspace("B", Some(first_root));
    first.sync().await;
    let (root_cloud_id, leaf_cloud_id) = (first.cloud_id(first_root), first.cloud_id(first_leaf));

    // 第二台设备拉取到嵌套结构
    let second = Device::new(store.clone());
    second.sync().await;
    let leaf_local_id = second
        .workspace_by_cloud_id(&leaf_cloud_id)
        .id
        .expect("本地分组 ID");
    assert!(second.workspace(leaf_local_id).parent_id.is_some());

    // 别处把子分组移出父分组：云端记录变成「明确的根分组」且更新更晚
    let mut moved = store.record(&leaf_cloud_id);
    moved.version += 1;
    moved.updated_at = (local_updated_seconds(&second, leaf_local_id) + 3_600) * 1000;
    moved.encrypted_data = root_payload(&second, "B");
    store.upsert(&moved);

    second.sync().await;

    assert_eq!(None, second.workspace_by_cloud_id(&leaf_cloud_id).parent_id);
    assert_eq!(None, second.workspace_by_cloud_id(&root_cloud_id).parent_id);
}

#[tokio::test]
async fn engine_upload_follows_ancestor_order_not_sidebar_order() {
    let store = FakeCloudStore::new();
    let device = Device::new(store.clone());
    let root = device.insert_workspace("A", None);
    let child = device.insert_workspace("B", None);
    // 用户把 B 拖进 A，而侧边栏顺序里 B 排在 A 前面
    device
        .workspaces
        .update_parent_id(child, Some(root))
        .expect("move group into parent");
    device
        .workspaces
        .update_sort_orders(&[(root, 5), (child, 0)])
        .expect("reorder sidebar");

    let result = device.sync().await;

    // 父分组必须先上传，否则子分组拿不到父分组的云端 ID
    assert_eq!(2, result.uploaded);
    assert_eq!(
        WorkspaceParentLink::Cloud(device.cloud_id(root)),
        device.decrypt_link(&store.record(&device.cloud_id(child)))
    );
}
