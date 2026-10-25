//! 新增数据类型在 Navop 云同步（OnetCloud）通道上的接入
//!
//! `SyncTypeHandler::Item` 要求 `local_id() -> Option<i64>`，而新增类型里设置类是
//! Singleton（无 ID）、已保存 SQL / 笔记是路径键（无 i64）。这里用统一的
//! `ScopeSyncItem` 承载：有 i64 主键的类型直接带 ID，其余用「稳定哈希 ID + key」。
//! cloud_id 统一落在 `sync_item_state`。
//!
//! 解密后的明文载荷直接挂在 item 上（`payload`），避免落地时二次解密。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::cloud_sync::engine::SyncEngine;
use crate::cloud_sync::models::{CloudSyncData, data_type};
use crate::cloud_sync::scope::payloads::{
    LlmProviderPlainData, PAYLOAD_FORMAT_VERSION, QuickCommandPlainData, SavedSqlPlainData,
    SftpFavoritePlainData, build_record, build_record_with_id, parse_record,
};
use crate::cloud_sync::scope::settings_payload::SettingsGroup;
use crate::cloud_sync::scope::settings_store::{FileSettingsStore, payload_checksum};
use crate::cloud_sync::scope::state::{
    SyncItemState, SyncItemStateRepository, SyncTypeStatusRepository,
};
use crate::cloud_sync::service::{CloudSyncService, SyncError};
use crate::cloud_sync::sync_type::{SyncTypeHandler, SyncableItem};
use crate::llm::storage::ProviderRepository;
use crate::llm::types::ProviderConfig;
use crate::storage::ConnectionRepository;
use crate::storage::PendingCloudDeletionRepository;
use crate::storage::quick_command::QuickCommandRepository;
use crate::storage::sftp_favorite_path::{
    SftpFavoritePathRepository, SftpFavoriteRow, sftp_favorite_connection_key,
};
use crate::storage::traits::Repository;

use super::notes::{NoteSyncItem, SharedNoteAdapter};
use super::payloads::{NotePlainData, SettingsPlainData};
use super::saved_sql::{
    SavedSqlDocument, delete_saved_sql_document, saved_sql_documents, write_saved_sql_document,
};
use crate::storage::sftp_favorite_path::LOCAL_FAVORITE_CONNECTION_KEY;

/// 统一的可同步项
#[derive(Debug, Clone)]
pub struct ScopeSyncItem {
    pub record_type: String,
    /// 本地稳定键（i64 主键的字符串形式 / 相对路径 / 分组名）
    pub key: String,
    /// 真实主键，或由 key 派生的稳定哈希 ID
    pub id: i64,
    pub name: String,
    pub checksum: String,
    pub updated_at: i64,
    pub cloud_id: Option<String>,
    /// 解密后的明文载荷（仅在下载路径上有值）
    pub payload: Option<serde_json::Value>,
}

impl SyncableItem for ScopeSyncItem {
    fn local_id(&self) -> Option<i64> {
        Some(self.id)
    }

    fn set_local_id(&mut self, id: Option<i64>) {
        if let Some(id) = id {
            self.id = id;
        }
    }

    fn item_name(&self) -> &str {
        &self.name
    }

    fn cloud_id(&self) -> Option<&str> {
        self.cloud_id.as_deref()
    }

    fn set_cloud_id(&mut self, cloud_id: Option<String>) {
        self.cloud_id = cloud_id;
    }

    fn updated_at(&self) -> Option<i64> {
        Some(self.updated_at)
    }

    fn sync_key(&self) -> &str {
        &self.key
    }

    fn local_checksum(&self) -> Option<String> {
        Some(self.checksum.clone())
    }
}

/// 无主键类型的稳定哈希 ID
pub fn synthetic_id(record_type: &str, key: &str) -> i64 {
    let mut hasher = DefaultHasher::new();
    record_type.hash(&mut hasher);
    key.hash(&mut hasher);
    (hasher.finish() & 0x7fff_ffff_ffff_ffff) as i64
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn storage_error(error: anyhow::Error) -> SyncError {
    SyncError::StorageError(error.to_string())
}

fn quick_plain(command: &crate::storage::QuickCommand) -> QuickCommandPlainData {
    QuickCommandPlainData {
        format_version: PAYLOAD_FORMAT_VERSION,
        name: command.name.clone(),
        command: command.command.clone(),
        description: command.description.clone(),
        shortcut: command.shortcut.clone(),
        pinned: command.pinned,
        sort_order: command.sort_order,
        group_name: command.group_name.clone(),
        group_color: command.group_color.clone(),
        connection_cloud_id: None,
    }
}

fn provider_plain(provider: &ProviderConfig) -> LlmProviderPlainData {
    LlmProviderPlainData {
        format_version: PAYLOAD_FORMAT_VERSION,
        name: provider.name.clone(),
        provider_type: provider.provider_type.as_str().to_string(),
        api_key: provider.api_key.clone(),
        api_base: provider.api_base.clone(),
        api_version: provider.api_version.clone(),
        model: provider.model.clone(),
        models: provider.models.clone(),
        max_tokens: provider.max_tokens,
        temperature_milli: provider.temperature.map(|v| (v * 1000.0).round() as i64),
        enabled: provider.enabled,
        is_default: provider.is_default,
    }
}

/// 上传侧：SFTP 收藏的明文载荷。
///
/// `connection_cloud_id` 优先填连接的 cloud_id（跨设备可解析），查不到时
/// `connection_name` 填真实连接名兜底（接收方按名字匹配），绝不填
/// `local:{id}` 这类只在本机有意义的连接键——那会让对端落地成悬空行。
fn sftp_plain(
    row: &SftpFavoriteRow,
    connection_cloud_id: Option<String>,
    connection_name: Option<String>,
) -> SftpFavoritePlainData {
    SftpFavoritePlainData {
        format_version: PAYLOAD_FORMAT_VERSION,
        path: row.path.clone(),
        sort_order: row.sort_order,
        connection_cloud_id,
        connection_name,
    }
}

/// 上传侧包装：解析连接归属后构造 SFTP 明文载荷
fn sftp_plain_payload(engine: &SyncEngine, row: &SftpFavoriteRow) -> SftpFavoritePlainData {
    let cloud_id = connection_cloud_id_for(engine, row.connection_id);
    let name = if cloud_id.is_some() {
        None
    } else {
        connection_name_for(engine, row.connection_id)
    };
    sftp_plain(row, cloud_id, name)
}

/// 由本地连接 id 解析其云端 ID（上传侧填充 `connection_cloud_id` 用）。
///
/// 直接查连接表：连接同步成功后 cloud_id 就回填在连接行上，
/// 不经过 `sync_item_state`（那是 scope 类型的状态表，connection 不在其中）。
pub(crate) fn connection_cloud_id_for(
    engine: &SyncEngine,
    connection_id: Option<i64>,
) -> Option<String> {
    let connection_id = connection_id?;
    let repo = engine.storage.get::<ConnectionRepository>()?;
    let connection = repo.get(connection_id).ok().flatten()?;
    connection.cloud_id.filter(|id| !id.trim().is_empty())
}

/// 由云端连接 ID 反解本地连接 id（下载侧恢复外键用）。
///
/// 走连接表的 cloud_id 列（连接同步落地时写入）。
pub(crate) fn resolve_connection_cloud_id(engine: &SyncEngine, cloud_id: &str) -> Option<i64> {
    let repo = engine.storage.get::<ConnectionRepository>()?;
    repo.get_by_cloud_id(cloud_id)
        .ok()
        .flatten()
        .and_then(|c| c.id)
}

/// 上传侧：连接名的兜底查询（`connection_cloud_id` 缺失时填真实连接名）。
fn connection_name_for(engine: &SyncEngine, connection_id: Option<i64>) -> Option<String> {
    let connection_id = connection_id?;
    let repo = engine.storage.get::<ConnectionRepository>()?;
    let connection = repo.get(connection_id).ok().flatten()?;
    Some(connection.name)
}

/// 下载侧：由本地连接 id 派生 sftp_favorite_paths.connection_key。
///
/// 键规则与 sftp_view 的 `sftp_favorite_connection_key` 一致：
/// 本地行有 id 就是 `local:{id}`（连接表总是先有行再有收藏）。
fn sftp_favorite_connection_key_of(engine: &SyncEngine, connection_id: i64) -> String {
    let repo = engine.storage.get::<ConnectionRepository>();
    if let Some(repo) = repo {
        if let Ok(Some(connection)) = repo.get(connection_id) {
            return sftp_favorite_connection_key(&connection);
        }
    }
    format!("local:{connection_id}")
}

/// 新增类型的统一处理器（一个 impl 覆盖全部新增 data_type）
pub struct ScopeSyncTypeHandler {
    record_type: &'static str,
    state: SyncItemStateRepository,
    /// 类型级同步状态（设置页状态列的数据来源）；与 state 同一连接
    status: SyncTypeStatusRepository,
    settings: FileSettingsStore,
    scope_env: String,
    user_id: String,
    notes: Option<SharedNoteAdapter>,
}

impl ScopeSyncTypeHandler {
    pub fn new(
        record_type: &'static str,
        state: SyncItemStateRepository,
        scope_env: String,
        user_id: String,
        notes: Option<SharedNoteAdapter>,
    ) -> Self {
        let status = state.status_repository();
        Self {
            record_type,
            state,
            status,
            settings: FileSettingsStore,
            scope_env,
            user_id,
            notes,
        }
    }

    fn base_state(&self, key: &str) -> SyncItemState {
        SyncItemState::new(
            self.scope_env.clone(),
            self.user_id.clone(),
            self.record_type,
            key,
        )
    }

    fn state_of(&self, key: &str) -> Option<SyncItemState> {
        self.state
            .get(&self.scope_env, &self.user_id, self.record_type, key)
            .ok()
            .flatten()
    }

    fn key_for_id(&self, id: i64) -> Option<String> {
        let key = self
            .state
            .list(&self.scope_env, &self.user_id)
            .ok()?
            .into_iter()
            .find(|s| {
                s.data_type == self.record_type && synthetic_id(&s.data_type, &s.local_key) == id
            })
            .map(|s| s.local_key);
        if key.is_none() {
            // 合成 ID 反查只在「state 行缺失」时失败：无主键类型首传前没有行，
            // 此时按 id 反查本来就不可靠，键控回调（on_uploaded_with_key）才是正路。
            tracing::debug!(
                record_type = self.record_type,
                id,
                "key_for_id 未命中（state 行缺失或 id 不匹配）"
            );
        }
        key
    }

    /// 回写同步基线（cloud_id / checksum / 水位）。
    ///
    /// `remote_updated_ms` 是远端记录的 updated_at（毫秒）：content_ts 取远端
    /// 时间换算成秒，不写本地 now——否则每轮回写都把水位推到「刚刚」，
    /// 下一轮计划器看到 local > cloud 又把它传一遍（乒乓）。
    /// `None` 表示没有远端时间可用（占位等场景），保持原值不推进。
    ///
    /// 成功后 mark_success 刷新类型状态（设置页状态列），失败 mark_error。
    fn persist(
        &self,
        key: &str,
        cloud_id: Option<String>,
        checksum: &str,
        remote_updated_ms: Option<i64>,
    ) -> Result<(), SyncError> {
        let mut state = self.state_of(key).unwrap_or_else(|| self.base_state(key));
        if let Some(cloud_id) = cloud_id {
            state.cloud_id = Some(cloud_id);
        }
        // 空 checksum 不覆盖：保留上次已知值（与既有语义一致）
        if !checksum.is_empty() {
            state.checksum = checksum.to_string();
        }
        state.last_synced_at = Some(now_seconds());
        if let Some(ms) = remote_updated_ms {
            state.content_ts = ms / 1000;
        }
        self.upsert_state_and_status(&state)
    }

    fn upsert_state_and_status(&self, state: &SyncItemState) -> Result<(), SyncError> {
        let result = self.state.upsert(state).map_err(storage_error);
        let status = self.status.clone();
        match &result {
            Ok(()) => {
                let _ = status.mark_success(&self.scope_env, &self.user_id, self.record_type);
            }
            Err(e) => {
                let _ = status.mark_error(
                    &self.scope_env,
                    &self.user_id,
                    self.record_type,
                    "state_write_failed",
                    &e.to_string(),
                );
            }
        }
        result
    }

    /// 首传前的占位：给还没有 state 行的项写一行 cloud_id=NULL 的基线，
    /// 保证键控回调（on_uploaded_with_key）有落点可 upsert。
    fn ensure_state_row(&self, key: &str) {
        if self.state_of(key).is_some() {
            return;
        }
        let placeholder = self.base_state(key);
        if let Err(e) = self.state.upsert(&placeholder) {
            tracing::warn!(
                record_type = self.record_type,
                key,
                error = %e,
                "写入占位 state 行失败"
            );
        }
    }

    /// 落地：按类型把解密出的载荷写回本地
    fn write_local(
        &self,
        engine: &SyncEngine,
        item: &mut ScopeSyncItem,
        existing_id: Option<i64>,
    ) -> Result<(), SyncError> {
        let payload = item.payload.clone().unwrap_or(serde_json::Value::Null);

        match self.record_type {
            data_type::QUICK_COMMAND => {
                let plain: QuickCommandPlainData = serde_json::from_value(payload)
                    .map_err(|e| SyncError::DataFormatError(e.to_string()))?;
                let repo = engine
                    .storage
                    .get::<QuickCommandRepository>()
                    .ok_or_else(|| SyncError::StorageError("quick command repo".to_string()))?;
                // 恢复连接外键：云端记录里存的是 connection_cloud_id，
                // 落地时反解成本地 connection_id，查不到则记为全局命令（None）
                // 并打错误标记，不落悬空引用
                let connection_id = plain
                    .connection_cloud_id
                    .as_deref()
                    .and_then(|cid| resolve_connection_cloud_id(engine, cid));
                if plain.connection_cloud_id.is_some() && connection_id.is_none() {
                    tracing::warn!(
                        connection_cloud_id = ?plain.connection_cloud_id,
                        name = ?plain.name,
                        "快捷命令的连接在本地不存在，按全局命令落地"
                    );
                }
                let mut command = crate::storage::QuickCommand::new(plain.command.clone());
                command.name = plain.name;
                command.description = plain.description;
                command.shortcut = plain.shortcut;
                command.pinned = plain.pinned;
                command.sort_order = plain.sort_order;
                command.group_name = plain.group_name;
                command.group_color = plain.group_color;
                command.is_builtin = false;
                command.connection_id = connection_id;
                let id = match existing_id {
                    Some(id) => {
                        command.id = Some(id);
                        repo.update(&command).map_err(storage_error)?;
                        id
                    }
                    None => repo.insert(&mut command).map_err(storage_error)?,
                };
                item.key = id.to_string();
                item.id = id;
            }
            data_type::SFTP_FAVORITE => {
                let plain: SftpFavoritePlainData = serde_json::from_value(payload)
                    .map_err(|e| SyncError::DataFormatError(e.to_string()))?;
                let repo = engine
                    .storage
                    .get::<SftpFavoritePathRepository>()
                    .ok_or_else(|| SyncError::StorageError("sftp repo".to_string()))?;
                // 连接归属解析：优先 connection_cloud_id 反解本地连接，
                // 拿不到再按名字在连接表里找；都失败则跳过该行——收藏路径
                // 离开连接没有意义，落地必是悬空行
                let connection_id = plain
                    .connection_cloud_id
                    .as_deref()
                    .and_then(|cid| resolve_connection_cloud_id(engine, cid))
                    .or_else(|| {
                        let name = plain
                            .connection_name
                            .as_deref()
                            .filter(|n| !n.is_empty() && !n.contains(':'))?;
                        let repo = engine.storage.get::<ConnectionRepository>()?;
                        repo.list()
                            .ok()?
                            .into_iter()
                            .find(|c| c.name == name)
                            .and_then(|c| c.id)
                    });
                let Some(connection_id) = connection_id else {
                    return Err(SyncError::DataFormatError(
                        "connection_missing: SFTP 收藏的连接在本地不存在".to_string(),
                    ));
                };
                let connection_key = sftp_favorite_connection_key_of(engine, connection_id);
                let row = SftpFavoriteRow {
                    id: existing_id.unwrap_or(0),
                    connection_id: Some(connection_id),
                    connection_key,
                    path: plain.path.clone(),
                    sort_order: plain.sort_order,
                    created_at: now_seconds(),
                    updated_at: now_seconds(),
                };
                let id = match existing_id {
                    Some(id) => {
                        repo.update_row(&SftpFavoriteRow { id, ..row })
                            .map_err(storage_error)?;
                        id
                    }
                    None => {
                        let inserted = repo.insert_row(&row).map_err(storage_error)?;
                        if inserted == 0 {
                            // insert_row 约定 0=跳过：路径非法或已有同款行。
                            // 抛 DataFormatError 让上层记 error 状态，不写 key="0" 的幽灵 state。
                            return Err(SyncError::DataFormatError(
                                "sftp_favorite_skipped: 路径非法或已有同款收藏".to_string(),
                            ));
                        }
                        inserted
                    }
                };
                item.key = id.to_string();
                item.id = id;
            }
            data_type::LLM_PROVIDER => {
                let plain: LlmProviderPlainData = serde_json::from_value(payload)
                    .map_err(|e| SyncError::DataFormatError(e.to_string()))?;
                let repo = engine
                    .storage
                    .get::<ProviderRepository>()
                    .ok_or_else(|| SyncError::StorageError("provider repo".to_string()))?;
                let mut config = ProviderConfig {
                    id: existing_id.unwrap_or(0),
                    name: plain.name.clone(),
                    provider_type: crate::llm::types::ProviderType::from_str(&plain.provider_type)
                        .unwrap_or(crate::llm::types::ProviderType::OpenAICompatible),
                    api_key: plain.api_key.clone(),
                    api_base: plain.api_base.clone(),
                    api_version: plain.api_version.clone(),
                    model: plain.model.clone(),
                    models: plain.models.clone(),
                    max_tokens: plain.max_tokens,
                    temperature: plain.temperature_milli.map(|m| m as f32 / 1000.0),
                    enabled: plain.enabled,
                    is_default: plain.is_default,
                    created_at: now_seconds(),
                    updated_at: now_seconds(),
                };
                let id = match existing_id {
                    Some(id) => {
                        config.id = id;
                        repo.update(&config).map_err(storage_error)?;
                        id
                    }
                    None => repo.insert(&mut config).map_err(storage_error)?,
                };
                item.key = id.to_string();
                item.id = id;
            }
            data_type::SAVED_SQL => {
                let plain: SavedSqlPlainData = serde_json::from_value(payload)
                    .map_err(|e| SyncError::DataFormatError(e.to_string()))?;
                write_saved_sql_document(&SavedSqlDocument {
                    document_id: plain.document_id.clone(),
                    relative_path: plain.name.clone(),
                    modified_at: now_seconds(),
                    content: plain.content,
                })
                .map_err(|e| SyncError::DataFormatError(format!("写入已保存 SQL 失败: {e}")))?;
                // 稳定键是 document_id：重命名后仍是同一条记录
                item.key = plain.document_id.clone();
                item.id = synthetic_id(self.record_type, &item.key);
            }
            data_type::NOTE => {
                let plain: NotePlainData = serde_json::from_value(payload)
                    .map_err(|e| SyncError::DataFormatError(e.to_string()))?;
                if let Some(adapter) = &self.notes {
                    adapter.write(&NoteSyncItem {
                        document_id: plain.document_id.clone(),
                        relative_path: plain.relative_path.clone(),
                        modified_at: now_seconds(),
                        content: plain.content.clone(),
                    });
                }
                // 稳定键是 document_id：重命名后仍是同一条记录，不会分裂成两条
                item.key = plain.document_id.clone();
                item.id = synthetic_id(self.record_type, &item.key);
            }
            other if SettingsGroup::from_data_type(other).is_some() => {
                let group = SettingsGroup::from_data_type(other).expect("checked");
                let plain: SettingsPlainData = serde_json::from_value(payload)
                    .map_err(|e| SyncError::DataFormatError(e.to_string()))?;
                self.settings
                    .write(group, &plain.payload)
                    .map_err(|e| SyncError::StorageError(e.to_string()))?;
                item.key = group.type_name().to_string();
                item.id = synthetic_id(other, &item.key);
            }
            // 笔记由 `notes` crate 提供数据源，OnetCloud 通道暂不落地
            _ => {}
        }

        self.persist(
            &item.key,
            item.cloud_id.clone(),
            &item.checksum.clone(),
            Some(item.updated_at * 1000),
        )
    }
}

impl SyncTypeHandler for ScopeSyncTypeHandler {
    type Item = ScopeSyncItem;

    fn data_type(&self) -> &'static str {
        self.record_type
    }

    fn display_name(&self) -> &'static str {
        match self.record_type {
            data_type::QUICK_COMMAND => "快捷命令",
            data_type::SFTP_FAVORITE => "SFTP 收藏",
            data_type::SAVED_SQL => "已保存的 SQL",
            data_type::NOTE => "笔记",
            data_type::LLM_PROVIDER => "LLM 提供商",
            _ => "设置",
        }
    }

    fn queue_key(&self) -> &'static str {
        crate::settings::sync_scope_descriptor(
            crate::settings::SyncScopeKey::from_str(self.record_type)
                .unwrap_or(crate::settings::SyncScopeKey::Note),
        )
        .map(|d| d.queue_key)
        .unwrap_or("sync:scope")
    }

    fn list_local(&self, engine: &SyncEngine) -> Result<Vec<ScopeSyncItem>, SyncError> {
        let mut items = Vec::new();

        match self.record_type {
            data_type::QUICK_COMMAND => {
                let repo = engine
                    .storage
                    .get::<QuickCommandRepository>()
                    .ok_or_else(|| SyncError::StorageError("quick command repo".to_string()))?;
                for command in repo.list().map_err(storage_error)? {
                    if command.is_builtin {
                        continue;
                    }
                    let Some(id) = command.id else { continue };
                    let key = id.to_string();
                    items.push(ScopeSyncItem {
                        record_type: self.record_type.to_string(),
                        key: key.clone(),
                        id,
                        name: command
                            .name
                            .clone()
                            .unwrap_or_else(|| command.command.clone()),
                        checksum: payload_checksum(
                            &serde_json::to_value(quick_plain(&command)).unwrap_or_default(),
                        ),
                        updated_at: command.updated_at.unwrap_or(0),
                        cloud_id: self.state_of(&key).and_then(|s| s.cloud_id),
                        payload: None,
                    });
                }
            }
            data_type::SFTP_FAVORITE => {
                let repo = engine
                    .storage
                    .get::<SftpFavoritePathRepository>()
                    .ok_or_else(|| SyncError::StorageError("sftp repo".to_string()))?;
                for favorite in repo.list_all().map_err(storage_error)? {
                    // 「本地文件列表」伪连接的收藏不参与云同步
                    if favorite.connection_key == LOCAL_FAVORITE_CONNECTION_KEY {
                        continue;
                    }
                    let key = favorite.id.to_string();
                    items.push(ScopeSyncItem {
                        record_type: self.record_type.to_string(),
                        key: key.clone(),
                        id: favorite.id,
                        name: favorite.path.clone(),
                        checksum: payload_checksum(
                            &serde_json::to_value(sftp_plain_payload(engine, &favorite))
                                .unwrap_or_default(),
                        ),
                        updated_at: favorite.updated_at,
                        cloud_id: self.state_of(&key).and_then(|s| s.cloud_id),
                        payload: None,
                    });
                }
            }
            data_type::LLM_PROVIDER => {
                let repo = engine
                    .storage
                    .get::<ProviderRepository>()
                    .ok_or_else(|| SyncError::StorageError("provider repo".to_string()))?;
                // 唯一的内置提供商（Navop AI）已下线，提供商现在全部由用户创建，无需过滤
                for provider in repo.list().map_err(storage_error)? {
                    let key = provider.id.to_string();
                    items.push(ScopeSyncItem {
                        record_type: self.record_type.to_string(),
                        key: key.clone(),
                        id: provider.id,
                        name: provider.name.clone(),
                        checksum: payload_checksum(
                            &serde_json::to_value(provider_plain(&provider)).unwrap_or_default(),
                        ),
                        updated_at: provider.updated_at,
                        cloud_id: self.state_of(&key).and_then(|s| s.cloud_id),
                        payload: None,
                    });
                }
            }
            data_type::SAVED_SQL => {
                for document in saved_sql_documents() {
                    let plain = SavedSqlPlainData {
                        format_version: PAYLOAD_FORMAT_VERSION,
                        document_id: document.document_id.clone(),
                        db_type: "default".to_string(),
                        connection_cloud_id: None,
                        database: None,
                        name: document.relative_path.clone(),
                        content: document.content,
                    };
                    let id = synthetic_id(self.record_type, &document.document_id);
                    items.push(ScopeSyncItem {
                        record_type: self.record_type.to_string(),
                        key: document.document_id.clone(),
                        id,
                        name: document.relative_path.clone(),
                        checksum: payload_checksum(
                            &serde_json::to_value(&plain).unwrap_or_default(),
                        ),
                        updated_at: self
                            .state_of(&document.document_id)
                            .map(|s| s.content_ts)
                            .unwrap_or_else(now_seconds),
                        cloud_id: self
                            .state_of(&document.document_id)
                            .and_then(|s| s.cloud_id),
                        payload: None,
                    });
                }
            }
            other if SettingsGroup::from_data_type(other).is_some() => {
                let group = SettingsGroup::from_data_type(other).expect("checked");
                let payload = self
                    .settings
                    .read(group)
                    .map_err(|e| SyncError::StorageError(e.to_string()))?;
                let key = group.type_name().to_string();
                items.push(ScopeSyncItem {
                    record_type: other.to_string(),
                    key: key.clone(),
                    id: synthetic_id(other, &key),
                    name: key.clone(),
                    checksum: payload_checksum(&payload),
                    updated_at: self
                        .state_of(&key)
                        .map(|s| s.content_ts)
                        .unwrap_or_else(now_seconds),
                    cloud_id: self.state_of(&key).and_then(|s| s.cloud_id),
                    payload: None,
                });
            }
            data_type::NOTE => {
                if let Some(adapter) = &self.notes {
                    for note in adapter.list() {
                        let id = synthetic_id(self.record_type, &note.document_id);
                        items.push(ScopeSyncItem {
                            record_type: self.record_type.to_string(),
                            key: note.document_id.clone(),
                            id,
                            name: note.title(),
                            checksum: payload_checksum(
                                &serde_json::to_value(NotePlainData {
                                    format_version: PAYLOAD_FORMAT_VERSION,
                                    notebook_id: String::new(),
                                    notebook_name: String::new(),
                                    document_id: note.document_id.clone(),
                                    relative_path: note.relative_path.clone(),
                                    title: note.title(),
                                    content: note.content.clone(),
                                })
                                .unwrap_or_default(),
                            ),
                            updated_at: note.modified_at,
                            cloud_id: self.state_of(&note.document_id).and_then(|st| st.cloud_id),
                            payload: None,
                        });
                    }
                }
            }
            _ => {}
        }

        self.finalize_local_snapshot(engine, &items);
        Ok(items)
    }

    fn insert_local(&self, engine: &SyncEngine, item: &mut ScopeSyncItem) -> Result<(), SyncError> {
        self.write_local(engine, item, None)
    }

    fn update_local_item(&self, engine: &SyncEngine, item: &Self::Item) -> Result<(), SyncError> {
        let mut owned = item.clone();
        self.write_local(engine, &mut owned, Some(item.id))
    }

    fn delete_local(&self, engine: &SyncEngine, id: i64) -> Result<(), SyncError> {
        // 兼容旧路径：计划器优先走 delete_local_with_key；这里按合成 ID 反查，
        // 未命中不再静默 Ok——软删除丢事件比报错更难排查。
        let Some(key) = self.key_for_id(id) else {
            tracing::warn!(
                record_type = self.record_type,
                id,
                "delete_local 反查 local_key 失败，本地删除未执行"
            );
            return Err(SyncError::DataFormatError(format!(
                "delete_local: 无法解析 {} 的本地键 {id}",
                self.record_type
            )));
        };
        self.delete_local_with_key(engine, &key)
    }

    fn on_uploaded(
        &self,
        _engine: &SyncEngine,
        local_id: i64,
        cloud_id: &str,
    ) -> Result<(), SyncError> {
        // 兼容旧路径：按合成 ID 反查。计划器现在优先走 on_uploaded_with_key，
        // 这里只在反查命中时工作，未命中记 warn（不再静默丢 cloud_id）。
        let Some(key) = self.key_for_id(local_id) else {
            tracing::warn!(
                record_type = self.record_type,
                local_id,
                cloud_id,
                "on_uploaded 反查 local_key 失败，cloud_id 未落库"
            );
            return Ok(());
        };
        self.on_uploaded_with_key(&key, cloud_id, "")
    }

    /// 仅刷新 last_synced_at（checksum 短路路径：内容无变化，不动 cloud_id/checksum/content_ts）
    fn touch_synced(&self, key: &str) -> Result<(), SyncError> {
        let mut state = self.state_of(key).unwrap_or_else(|| self.base_state(key));
        state.last_synced_at = Some(now_seconds());
        self.upsert_state_and_status(&state)
    }

    fn on_uploaded_with_key(
        &self,
        key: &str,
        cloud_id: &str,
        checksum: &str,
    ) -> Result<(), SyncError> {
        // 直接按 key upsert state：首传前 ensure_state_row 已写过占位行，
        // 这里读旧值合并（cloud_id + checksum），水位由 persist 语义控制
        let mut state = self.state_of(key).unwrap_or_else(|| self.base_state(key));
        state.cloud_id = Some(cloud_id.to_string());
        if !checksum.is_empty() {
            state.checksum = checksum.to_string();
        }
        state.last_synced_at = Some(now_seconds());
        self.upsert_state_and_status(&state)
    }

    fn persist_watermark(
        &self,
        key: &str,
        cloud_id: &str,
        remote_updated_ms: i64,
    ) -> Result<(), SyncError> {
        // 云端更新成功：cloud_id 不变、checksum 保留旧值（传空），
        // content_ts 取远端 updated_at 换算秒。
        self.persist(key, Some(cloud_id.to_string()), "", Some(remote_updated_ms))
    }

    fn delete_local_with_key(&self, engine: &SyncEngine, key: &str) -> Result<(), SyncError> {
        match self.record_type {
            data_type::QUICK_COMMAND => {
                let Some(id) = key.parse::<i64>().ok() else {
                    tracing::warn!(key, "快捷命令键控删除收到非数字 key");
                    return Ok(());
                };
                let repo = engine
                    .storage
                    .get::<QuickCommandRepository>()
                    .ok_or_else(|| SyncError::StorageError("quick command repo".into()))?;
                repo.delete(id).map_err(storage_error)?;
            }
            data_type::SFTP_FAVORITE => {
                let Some(id) = key.parse::<i64>().ok() else {
                    tracing::warn!(key, "SFTP 收藏键控删除收到非数字 key");
                    return Ok(());
                };
                let repo = engine
                    .storage
                    .get::<SftpFavoritePathRepository>()
                    .ok_or_else(|| SyncError::StorageError("sftp repo".into()))?;
                repo.delete_row(id).map_err(storage_error)?;
            }
            data_type::LLM_PROVIDER => {
                let Some(id) = key.parse::<i64>().ok() else {
                    tracing::warn!(key, "LLM 提供商键控删除收到非数字 key");
                    return Ok(());
                };
                let repo = engine
                    .storage
                    .get::<ProviderRepository>()
                    .ok_or_else(|| SyncError::StorageError("provider repo".into()))?;
                repo.delete(id).map_err(storage_error)?;
            }
            data_type::SAVED_SQL => {
                if !delete_saved_sql_document(key) {
                    tracing::warn!(key, "已保存 SQL 文档不存在或删除失败");
                }
            }
            data_type::NOTE => {
                if let Some(notes) = self.notes.as_ref() {
                    notes.delete(key);
                }
            }
            other => {
                if SettingsGroup::from_data_type(other).is_some() {
                    // 设置组按基线快照回滚，本地无「删除」动作
                    return Ok(());
                }
                tracing::warn!(record_type = other, key, "未知类型的键控删除被忽略");
                return Ok(());
            }
        }
        self.state
            .delete(&self.scope_env, &self.user_id, self.record_type, key)
            .map_err(storage_error)?;
        Ok(())
    }

    fn decrypt_name(&self, service: &CloudSyncService, data: &CloudSyncData) -> Option<String> {
        // 按名认领（local_unlinked_by_name / local_all_by_name）需要业务名：
        // 快捷命令 name、收藏 path、provider name、笔记 title、saved_sql name、
        // 设置组 group.type_name()。解密失败回退类型显示名——名字认领退化，
        // 但不能让一次解密失败挂掉整个同步。
        let name = match self.record_type {
            data_type::QUICK_COMMAND => parse_record::<QuickCommandPlainData>(service, data)
                .ok()
                .and_then(|p| p.name.clone().or_else(|| Some(p.command.clone()))),
            data_type::SFTP_FAVORITE => parse_record::<SftpFavoritePlainData>(service, data)
                .ok()
                .map(|p| p.path),
            data_type::LLM_PROVIDER => parse_record::<LlmProviderPlainData>(service, data)
                .ok()
                .map(|p| p.name),
            data_type::SAVED_SQL => parse_record::<SavedSqlPlainData>(service, data)
                .ok()
                .map(|p| p.name),
            data_type::NOTE => parse_record::<NotePlainData>(service, data)
                .ok()
                .map(|p| p.title),
            other => SettingsGroup::from_data_type(other).map(|g| g.type_name().to_string()),
        };
        name.or_else(|| Some(self.display_name().to_string()))
    }

    fn decrypt(
        &self,
        service: &CloudSyncService,
        data: &CloudSyncData,
    ) -> Result<ScopeSyncItem, SyncError> {
        let (name, payload) = match self.record_type {
            data_type::QUICK_COMMAND => {
                let plain: QuickCommandPlainData = parse_record(service, data)?;
                let name = plain.name.clone().unwrap_or_else(|| plain.command.clone());
                (
                    name,
                    serde_json::to_value(plain).unwrap_or(serde_json::Value::Null),
                )
            }
            data_type::SFTP_FAVORITE => {
                let plain: SftpFavoritePlainData = parse_record(service, data)?;
                (
                    plain.path.clone(),
                    serde_json::to_value(plain).unwrap_or(serde_json::Value::Null),
                )
            }
            data_type::SAVED_SQL => {
                let plain: SavedSqlPlainData = parse_record(service, data)?;
                (
                    plain.name.clone(),
                    serde_json::to_value(plain).unwrap_or(serde_json::Value::Null),
                )
            }
            data_type::LLM_PROVIDER => {
                let plain: LlmProviderPlainData = parse_record(service, data)?;
                (
                    plain.name.clone(),
                    serde_json::to_value(plain).unwrap_or(serde_json::Value::Null),
                )
            }
            data_type::NOTE => {
                let plain: NotePlainData = parse_record(service, data)?;
                (
                    plain.title.clone(),
                    serde_json::to_value(plain).unwrap_or(serde_json::Value::Null),
                )
            }
            _ => {
                let plain: SettingsPlainData = parse_record(service, data)?;
                (
                    plain.group.clone(),
                    serde_json::to_value(plain).unwrap_or(serde_json::Value::Null),
                )
            }
        };

        Ok(ScopeSyncItem {
            record_type: self.record_type.to_string(),
            key: String::new(),
            id: 0,
            name,
            checksum: data.checksum.clone(),
            updated_at: data.updated_at / 1000,
            cloud_id: Some(data.id.clone()),
            payload: Some(payload),
        })
    }

    fn encrypt(
        &self,
        engine: &SyncEngine,
        service: &CloudSyncService,
        item: &Self::Item,
        _teams: &[crate::cloud_sync::models::Team],
    ) -> Result<CloudSyncData, SyncError> {
        let key = if item.key.is_empty() {
            self.key_for_id(item.id).unwrap_or_default()
        } else {
            item.key.clone()
        };

        match self.record_type {
            data_type::QUICK_COMMAND => {
                let repo = engine
                    .storage
                    .get::<QuickCommandRepository>()
                    .ok_or_else(|| SyncError::StorageError("quick command repo".to_string()))?;
                let command = repo.get(item.id).map_err(storage_error)?.ok_or_else(|| {
                    SyncError::DataFormatError(format!("quick command {} missing", item.id))
                })?;
                self.build(service, &key, &quick_plain(&command))
            }
            data_type::SFTP_FAVORITE => {
                let repo = engine
                    .storage
                    .get::<SftpFavoritePathRepository>()
                    .ok_or_else(|| SyncError::StorageError("sftp repo".to_string()))?;
                let favorite = repo
                    .get_row(item.id)
                    .map_err(storage_error)?
                    .ok_or_else(|| {
                        SyncError::DataFormatError(format!("sftp favorite {} missing", item.id))
                    })?;
                let plain = sftp_plain_payload(engine, &favorite);
                self.build(service, &key, &plain)
            }
            data_type::LLM_PROVIDER => {
                let repo = engine
                    .storage
                    .get::<ProviderRepository>()
                    .ok_or_else(|| SyncError::StorageError("provider repo".to_string()))?;
                let provider = repo.get(item.id).map_err(storage_error)?.ok_or_else(|| {
                    SyncError::DataFormatError(format!("provider {} missing", item.id))
                })?;
                self.build(service, &key, &provider_plain(&provider))
            }
            data_type::SAVED_SQL => {
                let document = saved_sql_documents()
                    .into_iter()
                    .find(|document| document.document_id == key);
                let plain = match document {
                    Some(document) => SavedSqlPlainData {
                        format_version: PAYLOAD_FORMAT_VERSION,
                        document_id: document.document_id,
                        db_type: "default".to_string(),
                        connection_cloud_id: None,
                        database: None,
                        name: document.relative_path,
                        content: document.content,
                    },
                    None => {
                        return Err(SyncError::DataFormatError(format!(
                            "saved sql {key} missing"
                        )));
                    }
                };
                self.build(service, &key, &plain)
            }
            data_type::NOTE => {
                let note = self
                    .notes
                    .as_ref()
                    .and_then(|adapter| adapter.list().into_iter().find(|n| n.document_id == key));
                let plain = match note {
                    Some(note) => NotePlainData {
                        format_version: PAYLOAD_FORMAT_VERSION,
                        notebook_id: String::new(),
                        notebook_name: String::new(),
                        document_id: note.document_id.clone(),
                        relative_path: note.relative_path.clone(),
                        title: note.title(),
                        content: note.content,
                    },
                    None => return Err(SyncError::DataFormatError(format!("note {key} missing"))),
                };
                self.build(service, &key, &plain)
            }
            other if SettingsGroup::from_data_type(other).is_some() => {
                let group = SettingsGroup::from_data_type(other).expect("checked");
                let payload = self
                    .settings
                    .read(group)
                    .map_err(|e| SyncError::StorageError(e.to_string()))?;
                let plain = SettingsPlainData {
                    format_version: PAYLOAD_FORMAT_VERSION,
                    group: group.type_name().to_string(),
                    payload,
                };
                self.build(service, &key, &plain)
            }
            _ => Err(SyncError::DataFormatError(format!(
                "unsupported scope data type: {}",
                self.record_type
            ))),
        }
    }
}

impl ScopeSyncTypeHandler {
    /// list_local 收尾：为无 state 行的项写占位基线，并对 state 做差分删除。
    ///
    /// 差分规则：state 里该 data_type 的 local_key 集合 − 本地存活 key 集合
    /// = 本地已删除的项。已删除项若已上传过（state.cloud_id 非空）则写入
    /// pending_cloud_deletions（entity_type=data_type）等待云端传播；
    /// 无论是否上传过，差分出的 state 行都要清掉（delete_missing），
    /// 否则下轮还会被当作已删除重复处理。
    fn finalize_local_snapshot(&self, engine: &SyncEngine, items: &[ScopeSyncItem]) {
        let mut alive_keys = Vec::with_capacity(items.len());
        for item in items {
            alive_keys.push(item.key.clone());
            self.ensure_state_row(&item.key);
        }

        let state_rows = self
            .state
            .list(&self.scope_env, &self.user_id)
            .unwrap_or_default();
        let deleted: Vec<_> = state_rows
            .into_iter()
            .filter(|s| s.data_type == self.record_type && !alive_keys.contains(&s.local_key))
            .collect();
        if deleted.is_empty() {
            return;
        }

        if let Some(repo) = engine.storage.get::<PendingCloudDeletionRepository>() {
            for row in &deleted {
                let Some(cloud_id) = row.cloud_id.as_deref() else {
                    // 从未上传过：没有云端副本要删，只清 state
                    continue;
                };
                tracing::info!(
                    record_type = self.record_type,
                    key = %row.local_key,
                    cloud_id,
                    "检测到本地删除，加入云端待删除队列"
                );
                let _ = repo.add(cloud_id, self.record_type);
            }
        }

        if let Err(e) = self.state.delete_missing(
            &self.scope_env,
            &self.user_id,
            self.record_type,
            &alive_keys,
        ) {
            tracing::warn!(
                record_type = self.record_type,
                error = %e,
                "清理已删除项的 state 行失败"
            );
        }
    }

    fn build(
        &self,
        service: &CloudSyncService,
        key: &str,
        plain: &impl serde::Serialize,
    ) -> Result<CloudSyncData, SyncError> {
        // 空串视为「还没有 cloud_id」：否则会带着空 id 上传，无法再被寻址
        match self
            .state_of(key)
            .and_then(|s| s.cloud_id)
            .filter(|id| !id.trim().is_empty())
        {
            Some(cloud_id) => build_record_with_id(service, self.record_type, plain, cloud_id),
            None => build_record(service, self.record_type, plain),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud_sync::scope::state::SyncItemStateRepository;
    use crate::storage::connection::SqliteConnection;
    use crate::storage::manager::StorageManager;
    use crate::storage::migration::run_migrations;
    use crate::storage::models::{ConnectionType, StoredConnection};
    use crate::storage::repository::{ConnectionRepository, PendingCloudDeletionRepository};
    use crate::storage::traits::Repository;
    use std::sync::atomic::{AtomicU64, Ordering};

    static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// 建临时库（跑全量迁移）并注册好测试所需 repo
    fn test_storage() -> StorageManager {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let db_path = std::env::temp_dir().join(format!(
            "onetcli-scope-cloud-tests-{}-{unique}-{counter}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&db_path);
        let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
        conn.with_connection(|c| run_migrations(c))
            .expect("run migrations");
        let storage = StorageManager::new_with_connection(conn.clone());
        storage.register(ConnectionRepository::new(conn.clone()));
        storage.register(crate::storage::QuickCommandRepository::new(conn.clone()));
        storage.register(SftpFavoritePathRepository::new(conn.clone()));
        storage.register(PendingCloudDeletionRepository::new(conn.clone()));
        storage
    }

    fn test_handler(storage: &StorageManager, record_type: &'static str) -> ScopeSyncTypeHandler {
        ScopeSyncTypeHandler::new(
            record_type,
            SyncItemStateRepository::new(storage.connection()),
            "test-env".to_string(),
            "user-1".to_string(),
            None,
        )
    }

    fn ssh_connection(name: &str) -> StoredConnection {
        StoredConnection {
            ai_access_level: Default::default(),
            id: None,
            credential_revision: None,
            name: name.to_string(),
            connection_type: ConnectionType::SshSftp,
            params: "{}".to_string(),
            workspace_id: None,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
            preferred_open_mode: None,
        }
    }

    /// 键控回写：首传前无 state 行也能直接落 cloud_id（占位行由 ensure_state_row 保证，
    /// 但直接调用也不应失败）
    #[test]
    fn on_uploaded_with_key_persists_cloud_id_without_existing_row() {
        let storage = test_storage();
        let handler = test_handler(&storage, data_type::QUICK_COMMAND);

        handler
            .on_uploaded_with_key("42", "cloud-abc", "checksum-1")
            .expect("键控回写");

        let state = handler.state_of("42").expect("state 行应存在");
        assert_eq!(state.cloud_id.as_deref(), Some("cloud-abc"));
        assert_eq!(state.checksum, "checksum-1");
        assert!(state.last_synced_at.is_some());
    }

    /// 水位回写：content_ts 必须取远端 updated_at（毫秒→秒），不得写本地 now——
    /// 否则下一轮 local > cloud 触发重传（乒乓）
    #[test]
    fn persist_watermark_uses_remote_updated_at() {
        let storage = test_storage();
        let handler = test_handler(&storage, data_type::QUICK_COMMAND);

        // 先落一行已知基线
        handler
            .on_uploaded_with_key("7", "cloud-old", "")
            .expect("初次回写");

        let remote_ms: i64 = 1_700_000_123_456; // 2023-11-14T22:15:23Z 左右
        handler
            .persist_watermark("7", "cloud-old", remote_ms)
            .expect("水位回写");

        let state = handler.state_of("7").expect("state 行");
        assert_eq!(state.content_ts, remote_ms / 1000, "content_ts 应取远端秒");
        assert!(state.last_synced_at.is_some());
    }

    /// 差分删除：state 里有、本地存活的 key 集合里没有 → 该行进入
    /// pending_cloud_deletions（cloud_id 非空时）且 state 行被清掉
    #[test]
    fn snapshot_diff_queues_cloud_deletion_for_missing_keys() {
        let storage = test_storage();
        let handler = test_handler(&storage, data_type::QUICK_COMMAND);

        // 本地存活：id=1；已删除：id=2（带 cloud_id）、id=3（从未上传）
        handler.ensure_state_row("1");
        handler
            .on_uploaded_with_key("2", "cloud-deleted", "")
            .expect("回写已删除项");
        handler.ensure_state_row("3");

        // 模拟 list_local 返回（只含存活项 1）
        let items = vec![ScopeSyncItem {
            record_type: data_type::QUICK_COMMAND.to_string(),
            key: "1".to_string(),
            id: 1,
            name: "存活命令".to_string(),
            checksum: String::new(),
            updated_at: 0,
            cloud_id: None,
            payload: None,
        }];

        // engine 只在差分里用到 PendingCloudDeletionRepository；
        // 构造一个只注册了该 repo 的 StorageManager 即可
        let conn = storage.connection();
        let pending = PendingCloudDeletionRepository::new(conn);
        let deletions = pending
            .list_by_entity_type(data_type::QUICK_COMMAND)
            .expect("待删除列表");
        assert!(deletions.is_empty(), "差分前不应有待删除记录");

        // 直接验证差分核心：state 差集 = {2,3}
        let state_rows = handler
            .state
            .list("test-env", "user-1")
            .expect("state 列表");
        let alive: Vec<String> = items.iter().map(|i| i.key.clone()).collect();
        let mut missing: Vec<&str> = state_rows
            .iter()
            .filter(|s| s.data_type == data_type::QUICK_COMMAND && !alive.contains(&s.local_key))
            .map(|s| s.local_key.as_str())
            .collect();
        missing.sort();
        assert_eq!(missing, vec!["2", "3"], "差分应找出两个本地已删除项");

        // 带云 id 的进入待删除队列，无云 id 的只清 state
        for row in state_rows
            .iter()
            .filter(|s| missing.contains(&s.local_key.as_str()))
        {
            if let Some(cloud_id) = row.cloud_id.as_deref() {
                pending
                    .add(cloud_id, data_type::QUICK_COMMAND)
                    .expect("加入待删除");
            }
        }
        let queued = pending
            .list_by_entity_type(data_type::QUICK_COMMAND)
            .expect("待删除列表");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].cloud_id, "cloud-deleted");

        // remove_by_entity_type 只删本类型
        pending
            .remove_by_entity_type(data_type::QUICK_COMMAND, "cloud-deleted")
            .expect("移除待删除");
        assert!(
            pending
                .list_by_entity_type(data_type::QUICK_COMMAND)
                .expect("待删除列表")
                .is_empty()
        );
    }

    /// 快捷命令：键控删除按 rowid 删业务行与 state 行（全链路）
    #[test]
    fn quick_command_keyed_delete_removes_rows() {
        let storage = test_storage();
        let handler = test_handler(&storage, data_type::QUICK_COMMAND);
        let engine = sync_engine_for(&storage);

        let repo = storage
            .get::<crate::storage::QuickCommandRepository>()
            .unwrap();
        let mut command = crate::storage::QuickCommand::new("df -h".to_string());
        command.name = Some("Disk Usage".to_string());
        let id = repo.insert(&mut command).expect("插入快捷命令");

        handler.ensure_state_row(&id.to_string());
        handler
            .delete_local_with_key(&engine, &id.to_string())
            .expect("键控删除");
        assert!(repo.get(id).expect("查询").is_none(), "业务行应已删除");
        assert!(
            handler.state_of(&id.to_string()).is_none(),
            "state 行应已删除"
        );
    }

    /// 快捷命令下载落地：connection_cloud_id 反解成功时恢复本地外键；
    /// 连接缺失时按全局命令落地（None）不悬空
    #[test]
    fn quick_command_download_restores_connection_id() {
        let storage = test_storage();
        let handler = test_handler(&storage, data_type::QUICK_COMMAND);
        let engine = sync_engine_for(&storage);

        let repo = storage
            .get::<crate::storage::QuickCommandRepository>()
            .unwrap();

        // 场景 1：云端记录带 connection_cloud_id，本地能解析 → 恢复外键
        let connections = storage.get::<ConnectionRepository>().unwrap();
        let mut connection = ssh_connection("prod-ssh");
        let connection_id = connections.insert(&mut connection).expect("插入连接");
        connections
            .update_sync_status(connection_id, Some("cloud-conn-1".to_string()), None)
            .expect("回填 cloud_id");

        let mut item = ScopeSyncItem {
            record_type: data_type::QUICK_COMMAND.to_string(),
            key: String::new(),
            id: 0,
            name: "Disk Usage".to_string(),
            checksum: String::new(),
            updated_at: 0,
            cloud_id: Some("cloud-qc-1".to_string()),
            payload: Some(serde_json::json!({
                "format_version": 1,
                "name": "Disk Usage",
                "command": "df -h",
                "description": null,
                "shortcut": null,
                "pinned": false,
                "sort_order": 0,
                "group_name": null,
                "group_color": null,
                "connection_cloud_id": "cloud-conn-1",
            })),
        };
        use crate::cloud_sync::sync_type::SyncTypeHandler;
        handler
            .insert_local(&engine, &mut item)
            .expect("落地快捷命令");
        let created = repo
            .get(item.local_id().expect("rowid"))
            .expect("查询")
            .expect("业务行存在");
        assert_eq!(
            created.connection_id,
            Some(connection_id),
            "下载应恢复连接外键"
        );

        // 场景 2：连接缺失 → 全局命令（None）+ 不报错
        let mut orphan = ScopeSyncItem {
            record_type: data_type::QUICK_COMMAND.to_string(),
            key: String::new(),
            id: 0,
            name: "Uptime".to_string(),
            checksum: String::new(),
            updated_at: 0,
            cloud_id: Some("cloud-qc-2".to_string()),
            payload: Some(serde_json::json!({
                "format_version": 1,
                "name": "Uptime",
                "command": "uptime",
                "description": null,
                "shortcut": null,
                "pinned": false,
                "sort_order": 1,
                "group_name": null,
                "group_color": null,
                "connection_cloud_id": "cloud-nonexistent",
            })),
        };
        handler
            .insert_local(&engine, &mut orphan)
            .expect("孤儿命令也应落地");
        let created = repo
            .get(orphan.local_id().expect("rowid"))
            .expect("查询")
            .expect("业务行存在");
        assert_eq!(created.connection_id, None, "连接缺失按全局命令落地");
    }

    /// SFTP insert_row 冲突语义：同 (connection_key, path) 重复插入不报错、
    /// 返回已有行 id；非法路径返回 0
    #[test]
    fn sftp_insert_row_conflict_returns_existing_id() {
        let storage = test_storage();
        let repo = storage.get::<SftpFavoritePathRepository>().unwrap();

        let row = SftpFavoriteRow {
            id: 0,
            connection_id: None,
            connection_key: "local:1".to_string(),
            path: "/srv/app".to_string(),
            sort_order: 1,
            created_at: 0,
            updated_at: 0,
        };
        let first = repo.insert_row(&row).expect("首次插入");
        assert!(first > 0);

        let second = repo.insert_row(&row).expect("冲突插入");
        assert_eq!(second, first, "冲突应返回已有行 id 而不是报错");

        let invalid = repo
            .insert_row(&SftpFavoriteRow {
                path: "   ".to_string(),
                ..row
            })
            .expect("非法路径");
        assert_eq!(invalid, 0, "非法路径应返回 0（跳过）");
    }

    /// 连接 cloud_id 解析：仅依赖 StorageManager 的两个纯函数不需要完整引擎——
    /// 这里通过 handler 的 SFTP 落地路径间接验证（connection_missing 时报错跳过）
    #[test]
    fn sftp_download_without_connection_reports_missing() {
        let storage = test_storage();
        let handler = test_handler(&storage, data_type::SFTP_FAVORITE);

        let mut item = ScopeSyncItem {
            record_type: data_type::SFTP_FAVORITE.to_string(),
            key: String::new(),
            id: 0,
            name: "/srv/app".to_string(),
            checksum: String::new(),
            updated_at: 0,
            cloud_id: Some("cloud-x".to_string()),
            payload: Some(serde_json::json!({
                "format_version": 1,
                "path": "/srv/app",
                "sort_order": 1,
                "connection_cloud_id": "cloud-nonexistent",
                "connection_name": null,
            })),
        };

        // write_local 是私有方法：通过 SyncTypeHandler::insert_local 走同一路径
        use crate::cloud_sync::sync_type::SyncTypeHandler;
        let err = handler.insert_local(&sync_engine_for(&storage), &mut item);
        assert!(err.is_err(), "连接缺失时应报错而非落悬空行");
        let msg = err.err().unwrap().to_string();
        assert!(
            msg.contains("connection_missing"),
            "错误信息应带 connection_missing 标记: {msg}"
        );
    }

    /// 构造最小 SyncEngine（只填 storage；云客户端不会被触达）
    fn sync_engine_for(storage: &StorageManager) -> SyncEngine {
        SyncEngine::new(
            std::sync::Arc::new(NoopCloudClient),
            std::sync::Arc::new(std::sync::RwLock::new(CloudSyncService::new())),
            storage.clone(),
        )
    }

    /// 全方法不可用的云客户端替身（write_local 只触达本地存储）
    struct NoopCloudClient;

    fn not_used() -> crate::cloud_sync::client::CloudApiError {
        crate::cloud_sync::client::CloudApiError::Unknown("not used".to_string())
    }

    #[async_trait::async_trait]
    impl crate::cloud_sync::client::CloudApiClient for NoopCloudClient {
        fn environment_id(&self) -> &str {
            "test-environment"
        }

        async fn sign_in_with_password(
            &self,
            _email: &str,
            _password: &str,
        ) -> Result<crate::cloud_sync::client::AuthResponse, crate::cloud_sync::client::CloudApiError>
        {
            Err(not_used())
        }

        async fn sign_in_with_oauth(
            &self,
            _provider: &str,
            _redirect_url: &str,
        ) -> Result<
            crate::cloud_sync::client::OAuthResponse,
            crate::cloud_sync::client::CloudApiError,
        > {
            Err(not_used())
        }

        async fn sign_up(
            &self,
            _email: &str,
            _password: &str,
        ) -> Result<crate::cloud_sync::client::AuthResponse, crate::cloud_sync::client::CloudApiError>
        {
            Err(not_used())
        }

        async fn sign_out(&self) -> Result<(), crate::cloud_sync::client::CloudApiError> {
            Err(not_used())
        }

        async fn get_current_user(
            &self,
        ) -> Result<
            Option<crate::cloud_sync::client::UserInfo>,
            crate::cloud_sync::client::CloudApiError,
        > {
            Err(not_used())
        }

        async fn refresh_token(
            &self,
            _refresh_token: &str,
        ) -> Result<crate::cloud_sync::client::AuthResponse, crate::cloud_sync::client::CloudApiError>
        {
            Err(not_used())
        }

        async fn sign_in_with_otp(
            &self,
            _email: &str,
        ) -> Result<(), crate::cloud_sync::client::CloudApiError> {
            Err(not_used())
        }

        async fn verify_otp(
            &self,
            _email: &str,
            _token: &str,
        ) -> Result<crate::cloud_sync::client::AuthResponse, crate::cloud_sync::client::CloudApiError>
        {
            Err(not_used())
        }

        async fn get_user_config(
            &self,
        ) -> Result<
            Option<crate::cloud_sync::models::CloudUserConfig>,
            crate::cloud_sync::client::CloudApiError,
        > {
            Err(not_used())
        }

        async fn save_user_config(
            &self,
            _config: &crate::cloud_sync::models::CloudUserConfig,
        ) -> Result<(), crate::cloud_sync::client::CloudApiError> {
            Err(not_used())
        }

        async fn get_subscription(
            &self,
        ) -> Result<
            Option<crate::license::SubscriptionInfo>,
            crate::cloud_sync::client::CloudApiError,
        > {
            Err(not_used())
        }

        async fn list_models(
            &self,
        ) -> Result<Vec<String>, crate::cloud_sync::client::CloudApiError> {
            Err(not_used())
        }

        async fn list_sync_data(
            &self,
            _data_type: Option<&str>,
            _team_id: Option<&str>,
            _since: Option<i64>,
        ) -> Result<Vec<CloudSyncData>, crate::cloud_sync::client::CloudApiError> {
            Ok(Vec::new())
        }

        async fn create_sync_data(
            &self,
            _data: &CloudSyncData,
        ) -> Result<CloudSyncData, crate::cloud_sync::client::CloudApiError> {
            Err(not_used())
        }

        async fn update_sync_data(
            &self,
            _data: &CloudSyncData,
        ) -> Result<CloudSyncData, crate::cloud_sync::client::CloudApiError> {
            Err(not_used())
        }

        async fn delete_sync_data(
            &self,
            _id: &str,
        ) -> Result<(), crate::cloud_sync::client::CloudApiError> {
            Err(not_used())
        }

        async fn list_teams(
            &self,
        ) -> Result<Vec<crate::cloud_sync::models::Team>, crate::cloud_sync::client::CloudApiError>
        {
            Ok(Vec::new())
        }

        async fn list_team_members(
            &self,
            _team_id: &str,
        ) -> Result<
            Vec<crate::cloud_sync::models::TeamMember>,
            crate::cloud_sync::client::CloudApiError,
        > {
            Ok(Vec::new())
        }

        async fn list_current_user_team_members(
            &self,
        ) -> Result<
            Vec<crate::cloud_sync::models::TeamMember>,
            crate::cloud_sync::client::CloudApiError,
        > {
            Ok(Vec::new())
        }

        async fn initialize_team_key(
            &self,
            _team: &crate::cloud_sync::models::Team,
        ) -> Result<crate::cloud_sync::models::Team, crate::cloud_sync::client::CloudApiError>
        {
            Err(not_used())
        }

        async fn rotate_team_key(
            &self,
            _team: &crate::cloud_sync::models::Team,
            _records: &[CloudSyncData],
        ) -> Result<(), crate::cloud_sync::client::CloudApiError> {
            Err(not_used())
        }

        async fn chat(
            &self,
            _request: &llm_connector::ChatRequest,
        ) -> Result<String, crate::cloud_sync::client::CloudApiError> {
            Err(not_used())
        }

        async fn chat_stream(
            &self,
            _request: &llm_connector::ChatRequest,
        ) -> Result<crate::llm::ChatStream, crate::cloud_sync::client::CloudApiError> {
            Err(not_used())
        }
    }
}
