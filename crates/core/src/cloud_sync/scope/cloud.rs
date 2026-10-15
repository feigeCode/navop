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
use crate::cloud_sync::scope::state::{SyncItemState, SyncItemStateRepository};
use crate::cloud_sync::service::{CloudSyncService, SyncError};
use crate::cloud_sync::sync_type::{SyncTypeHandler, SyncableItem};
use crate::llm::storage::ProviderRepository;
use crate::llm::types::ProviderConfig;
use crate::storage::quick_command::QuickCommandRepository;
use crate::storage::sftp_favorite_path::{SftpFavoritePathRepository, SftpFavoriteRow};
use crate::storage::traits::Repository;

use super::notes::{NoteSyncItem, SharedNoteAdapter};
use super::payloads::{NotePlainData, SettingsPlainData};

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

fn sftp_plain(row: &SftpFavoriteRow) -> SftpFavoritePlainData {
    SftpFavoritePlainData {
        format_version: PAYLOAD_FORMAT_VERSION,
        path: row.path.clone(),
        sort_order: row.sort_order,
        connection_cloud_id: None,
        connection_name: Some(row.connection_key.clone()),
    }
}

/// 新增类型的统一处理器（一个 impl 覆盖全部新增 data_type）
pub struct ScopeSyncTypeHandler {
    record_type: &'static str,
    state: SyncItemStateRepository,
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
        Self {
            record_type,
            state,
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
        self.state
            .list(&self.scope_env, &self.user_id)
            .ok()?
            .into_iter()
            .find(|s| {
                s.data_type == self.record_type && synthetic_id(&s.data_type, &s.local_key) == id
            })
            .map(|s| s.local_key)
    }

    fn persist(
        &self,
        key: &str,
        cloud_id: Option<String>,
        checksum: &str,
    ) -> Result<(), SyncError> {
        let mut state = self.state_of(key).unwrap_or_else(|| self.base_state(key));
        if let Some(cloud_id) = cloud_id {
            state.cloud_id = Some(cloud_id);
        }
        if !checksum.is_empty() {
            state.checksum = checksum.to_string();
        }
        state.last_synced_at = Some(now_seconds());
        state.content_ts = now_seconds();
        self.state.upsert(&state).map_err(storage_error)
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
                let mut command = crate::storage::QuickCommand::new(plain.command.clone());
                command.name = plain.name;
                command.description = plain.description;
                command.shortcut = plain.shortcut;
                command.pinned = plain.pinned;
                command.sort_order = plain.sort_order;
                command.group_name = plain.group_name;
                command.group_color = plain.group_color;
                command.is_builtin = false;
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
                let row = SftpFavoriteRow {
                    id: existing_id.unwrap_or(0),
                    connection_id: None,
                    connection_key: plain.connection_name.clone().unwrap_or_default(),
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
                    None => repo.insert_row(&row).map_err(storage_error)?,
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
                crate::cloud_sync::scope::personal::write_saved_sql(&plain.name, &plain.content);
                item.key = plain.name.clone();
                item.id = synthetic_id(self.record_type, &item.key);
            }
            data_type::NOTE => {
                let plain: NotePlainData = serde_json::from_value(payload)
                    .map_err(|e| SyncError::DataFormatError(e.to_string()))?;
                if let Some(adapter) = &self.notes {
                    adapter.write(&NoteSyncItem {
                        relative_path: plain.relative_path.clone(),
                        modified_at: now_seconds(),
                        content: plain.content.clone(),
                    });
                }
                item.key = plain.relative_path.clone();
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

        self.persist(&item.key, item.cloud_id.clone(), &item.checksum.clone())
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
                    let key = favorite.id.to_string();
                    items.push(ScopeSyncItem {
                        record_type: self.record_type.to_string(),
                        key: key.clone(),
                        id: favorite.id,
                        name: favorite.path.clone(),
                        checksum: payload_checksum(
                            &serde_json::to_value(sftp_plain(&favorite)).unwrap_or_default(),
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
                for provider in repo.list().map_err(storage_error)? {
                    if provider.is_builtin() {
                        continue;
                    }
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
                for (key, content) in crate::cloud_sync::scope::personal::collect_saved_sql_files()
                {
                    let plain = SavedSqlPlainData {
                        format_version: PAYLOAD_FORMAT_VERSION,
                        db_type: "default".to_string(),
                        connection_cloud_id: None,
                        database: None,
                        name: key.clone(),
                        content,
                    };
                    let id = synthetic_id(self.record_type, &key);
                    items.push(ScopeSyncItem {
                        record_type: self.record_type.to_string(),
                        key: key.clone(),
                        id,
                        name: key.clone(),
                        checksum: payload_checksum(
                            &serde_json::to_value(&plain).unwrap_or_default(),
                        ),
                        updated_at: self
                            .state_of(&key)
                            .map(|s| s.content_ts)
                            .unwrap_or_else(now_seconds),
                        cloud_id: self.state_of(&key).and_then(|s| s.cloud_id),
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
                        let id = synthetic_id(self.record_type, &note.relative_path);
                        items.push(ScopeSyncItem {
                            record_type: self.record_type.to_string(),
                            key: note.relative_path.clone(),
                            id,
                            name: note.title(),
                            checksum: payload_checksum(
                                &serde_json::to_value(NotePlainData {
                                    format_version: PAYLOAD_FORMAT_VERSION,
                                    notebook_id: String::new(),
                                    notebook_name: String::new(),
                                    document_id: note.relative_path.clone(),
                                    relative_path: note.relative_path.clone(),
                                    title: note.title(),
                                    content: note.content.clone(),
                                })
                                .unwrap_or_default(),
                            ),
                            updated_at: note.modified_at,
                            cloud_id: self
                                .state_of(&note.relative_path)
                                .and_then(|st| st.cloud_id),
                            payload: None,
                        });
                    }
                }
            }
            _ => {}
        }

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
        let Some(key) = self.key_for_id(id) else {
            return Ok(());
        };
        match self.record_type {
            data_type::QUICK_COMMAND => {
                if let Some(repo) = engine.storage.get::<QuickCommandRepository>() {
                    repo.delete(id).map_err(storage_error)?;
                }
            }
            data_type::SFTP_FAVORITE => {
                if let Some(repo) = engine.storage.get::<SftpFavoritePathRepository>() {
                    repo.delete_row(id).map_err(storage_error)?;
                }
            }
            data_type::LLM_PROVIDER => {
                if let Some(repo) = engine.storage.get::<ProviderRepository>() {
                    repo.delete(id).map_err(storage_error)?;
                }
            }
            data_type::SAVED_SQL => {
                crate::cloud_sync::scope::personal::delete_saved_sql(&key);
            }
            data_type::NOTE => {
                if let Some(adapter) = &self.notes {
                    adapter.delete(&key);
                }
            }
            // 设置类 Singleton 与笔记没有删除语义
            _ => {}
        }
        let _ = self
            .state
            .delete(&self.scope_env, &self.user_id, self.record_type, &key);
        Ok(())
    }

    fn on_uploaded(
        &self,
        _engine: &SyncEngine,
        local_id: i64,
        cloud_id: &str,
    ) -> Result<(), SyncError> {
        let Some(key) = self.key_for_id(local_id) else {
            return Ok(());
        };
        self.persist(&key, Some(cloud_id.to_string()), "")
    }

    fn decrypt_name(&self, _service: &CloudSyncService, _data: &CloudSyncData) -> Option<String> {
        Some(self.display_name().to_string())
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
                self.build(service, &key, &sftp_plain(&favorite))
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
                let content =
                    crate::cloud_sync::scope::personal::read_saved_sql(&key).unwrap_or_default();
                let plain = SavedSqlPlainData {
                    format_version: PAYLOAD_FORMAT_VERSION,
                    db_type: "default".to_string(),
                    connection_cloud_id: None,
                    database: None,
                    name: key.clone(),
                    content,
                };
                self.build(service, &key, &plain)
            }
            data_type::NOTE => {
                let note = self.notes.as_ref().and_then(|adapter| {
                    adapter.list().into_iter().find(|n| n.relative_path == key)
                });
                let plain = match note {
                    Some(note) => NotePlainData {
                        format_version: PAYLOAD_FORMAT_VERSION,
                        notebook_id: String::new(),
                        notebook_name: String::new(),
                        document_id: note.relative_path.clone(),
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
    fn build(
        &self,
        service: &CloudSyncService,
        key: &str,
        plain: &impl serde::Serialize,
    ) -> Result<CloudSyncData, SyncError> {
        match self.state_of(key).and_then(|s| s.cloud_id) {
            Some(cloud_id) => build_record_with_id(service, self.record_type, plain, cloud_id),
            None => build_record(service, self.record_type, plain),
        }
    }
}
