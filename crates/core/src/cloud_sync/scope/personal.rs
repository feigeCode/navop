//! 新增数据类型在 Personal 同步通道上的接入
//!
//! 复用 `PersonalSyncLocalSource` 的契约，把新类型统一成
//! `list → export → apply_remote → mark_synced → delete` 五个动作，
//! 交给既有的 `PersonalSyncWorker` / `PersonalSyncPlanner` 驱动。

use std::collections::HashSet;
use std::sync::{Arc, RwLock, RwLockReadGuard};

use async_trait::async_trait;

use crate::cloud_sync::models::{CloudSyncData, data_type};
use crate::cloud_sync::personal::{
    PersonalSyncItemSnapshot, PersonalSyncLocalSource, SyncStoreError,
};
use crate::cloud_sync::scope::notes::{NOTE_MAX_CONTENT_BYTES, NoteSyncItem, SharedNoteAdapter};
use crate::cloud_sync::scope::payloads::{
    LlmProviderPlainData, PAYLOAD_FORMAT_VERSION, QuickCommandPlainData, SftpFavoritePlainData,
    build_record_with_id, parse_record,
};
use crate::cloud_sync::scope::payloads::{NotePlainData, SavedSqlPlainData};
use crate::cloud_sync::scope::saved_sql::{
    SavedSqlDocument, delete_saved_sql_document, saved_sql_documents, write_saved_sql_document,
};
use crate::cloud_sync::scope::settings_payload::{SettingsGroup, apply_payload, merge_three_way};
use crate::cloud_sync::scope::settings_store::{
    FileSettingsStore, canonical_json, payload_checksum,
};
use crate::cloud_sync::scope::state::{
    SyncItemState, SyncItemStateRepository, SyncTypeStatusRepository,
};
use crate::cloud_sync::service::CloudSyncService;
use crate::llm::storage::ProviderRepository;
use crate::llm::types::ProviderConfig;
use crate::storage::get_config_dir;
use crate::storage::quick_command::QuickCommandRepository;
use crate::storage::sftp_favorite_path::{SftpFavoritePathRepository, SftpFavoriteRow};
use crate::storage::traits::Repository;

use super::payloads::SettingsPlainData;

/// 已保存 SQL 的根目录（`config/queries`）
const SAVED_SQL_ROOT: &str = "queries";

/// 新增类型的本地操作集合
pub struct SyncScopeSource {
    scope_env: String,
    user_id: String,
    /// 用户勾选后实际生效的 data_type 集合
    enabled: HashSet<String>,
    state: SyncItemStateRepository,
    status: SyncTypeStatusRepository,
    quick_commands: QuickCommandRepository,
    sftp: SftpFavoritePathRepository,
    providers: ProviderRepository,
    notes: Option<SharedNoteAdapter>,
    settings: FileSettingsStore,
    service: Arc<RwLock<CloudSyncService>>,
}

impl SyncScopeSource {
    pub fn new(
        scope_env: String,
        user_id: String,
        enabled: HashSet<String>,
        state: SyncItemStateRepository,
        status: SyncTypeStatusRepository,
        quick_commands: QuickCommandRepository,
        sftp: SftpFavoritePathRepository,
        providers: ProviderRepository,
        notes: Option<SharedNoteAdapter>,
        service: Arc<RwLock<CloudSyncService>>,
    ) -> Self {
        Self {
            scope_env,
            user_id,
            enabled,
            state,
            status,
            quick_commands,
            sftp,
            providers,
            notes,
            settings: FileSettingsStore,
            service,
        }
    }

    pub fn enabled(&self) -> &HashSet<String> {
        &self.enabled
    }

    pub fn state_repo(&self) -> &SyncItemStateRepository {
        &self.state
    }

    // ---------- 通用辅助 ----------

    fn base(&self, record_type: &str, local_key: &str) -> SyncItemState {
        SyncItemState::new(
            self.scope_env.clone(),
            self.user_id.clone(),
            record_type,
            local_key,
        )
    }

    fn state_of(&self, record_type: &str, local_key: &str) -> Option<SyncItemState> {
        self.state
            .get(&self.scope_env, &self.user_id, record_type, local_key)
            .ok()
            .flatten()
    }

    fn snapshot(
        &self,
        record_type: &str,
        local_key: &str,
        content_ts: i64,
        checksum: &str,
    ) -> PersonalSyncItemSnapshot {
        let known = self.state_of(record_type, local_key);
        PersonalSyncItemSnapshot {
            local_id: format!("{record_type}:{local_key}"),
            cloud_id: known.as_ref().and_then(|s| s.cloud_id.clone()),
            data_type: record_type.to_string(),
            updated_at: content_ts,
            last_synced_at: known.as_ref().and_then(|s| s.last_synced_at),
            checksum: checksum.to_string(),
            team_id: None,
        }
    }

    fn lock_service(&self) -> Result<RwLockReadGuard<'_, CloudSyncService>, SyncStoreError> {
        self.service
            .read()
            .map_err(|_| SyncStoreError::Io("cloud sync service lock poisoned".to_string()))
    }

    fn mark_success(&self, record_type: &str) {
        let _ = self
            .status
            .mark_success(&self.scope_env, &self.user_id, record_type);
    }

    fn mark_error(&self, record_type: &str, code: &str, message: &str) {
        let _ = self
            .status
            .mark_error(&self.scope_env, &self.user_id, record_type, code, message);
    }

    fn persist_state(&self, state: &SyncItemState) -> Result<(), SyncStoreError> {
        self.state
            .upsert(state)
            .map_err(|e| SyncStoreError::Io(e.to_string()))
    }

    /// 按 cloud_id 反查本地键
    fn find_local_key(&self, record_type: &str, cloud_id: &str) -> Option<String> {
        self.state
            .list(&self.scope_env, &self.user_id)
            .ok()?
            .into_iter()
            .find(|s| s.data_type == record_type && s.cloud_id.as_deref() == Some(cloud_id))
            .map(|s| s.local_key)
    }

    fn record_synced(
        &self,
        record_type: &str,
        local_key: &str,
        record: &CloudSyncData,
        base_payload: Option<String>,
    ) {
        let mut state = self
            .state_of(record_type, local_key)
            .unwrap_or_else(|| self.base(record_type, local_key));
        state.cloud_id = Some(record.id.clone());
        state.checksum = record.checksum.clone();
        state.last_synced_at = Some(now_seconds());
        state.content_ts = now_seconds();
        state.base_payload = base_payload;
        let _ = self.persist_state(&state);
    }

    // ---------- 枚举 ----------

    fn list_quick_commands(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError> {
        let mut items = Vec::new();
        for command in self
            .quick_commands
            .list()
            .map_err(|e| SyncStoreError::Io(e.to_string()))?
        {
            // 只同步用户自建命令：内置默认命令每台设备本地已有（R1）
            if command.is_builtin {
                continue;
            }
            let Some(id) = command.id else { continue };
            let checksum = payload_checksum(
                &serde_json::to_value(quick_command_plain(&command)).unwrap_or_default(),
            );
            items.push(self.snapshot(
                data_type::QUICK_COMMAND,
                &id.to_string(),
                command.updated_at.unwrap_or(0),
                &checksum,
            ));
        }
        Ok(items)
    }

    fn list_sftp_favorites(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError> {
        let mut items = Vec::new();
        for favorite in self
            .sftp
            .list_all()
            .map_err(|e| SyncStoreError::Io(e.to_string()))?
        {
            let plain = SftpFavoritePlainData {
                format_version: PAYLOAD_FORMAT_VERSION,
                path: favorite.path.clone(),
                sort_order: favorite.sort_order,
                connection_cloud_id: None,
                connection_name: Some(favorite.connection_key.clone()),
            };
            let checksum = payload_checksum(&serde_json::to_value(&plain).unwrap_or_default());
            items.push(self.snapshot(
                data_type::SFTP_FAVORITE,
                &favorite.id.to_string(),
                favorite.updated_at,
                &checksum,
            ));
        }
        Ok(items)
    }

    fn list_providers(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError> {
        let mut items = Vec::new();
        for provider in self
            .providers
            .list()
            .map_err(|e| SyncStoreError::Io(e.to_string()))?
        {
            // 唯一的内置提供商（Navop AI）已下线，提供商现在全部由用户创建，无需过滤
            let plain = provider_plain(&provider);
            let checksum = payload_checksum(&serde_json::to_value(&plain).unwrap_or_default());
            items.push(self.snapshot(
                data_type::LLM_PROVIDER,
                &provider.id.to_string(),
                provider.updated_at,
                &checksum,
            ));
        }
        Ok(items)
    }

    fn list_saved_sql(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError> {
        let mut items = Vec::new();
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
            let checksum = payload_checksum(&serde_json::to_value(&plain).unwrap_or_default());
            items.push(self.snapshot(
                data_type::SAVED_SQL,
                &document.document_id,
                document.modified_at,
                &checksum,
            ));
        }
        Ok(items)
    }

    fn list_notes(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError> {
        let Some(adapter) = &self.notes else {
            return Ok(Vec::new());
        };
        let mut items = Vec::new();
        for note in adapter.list() {
            // Q4：单篇超过 1 MB 跳过，不重试
            if note.content.len() > NOTE_MAX_CONTENT_BYTES {
                self.mark_error(
                    data_type::NOTE,
                    "note_too_large",
                    &format!("{} 超过 1 MB，已跳过", note.title()),
                );
                continue;
            }
            let plain = NotePlainData {
                format_version: PAYLOAD_FORMAT_VERSION,
                notebook_id: String::new(),
                notebook_name: String::new(),
                document_id: note.document_id.clone(),
                relative_path: note.relative_path.clone(),
                title: note.title(),
                content: note.content.clone(),
            };
            let checksum = payload_checksum(&serde_json::to_value(&plain).unwrap_or_default());
            items.push(self.snapshot(
                data_type::NOTE,
                &note.document_id,
                note.modified_at,
                &checksum,
            ));
        }
        Ok(items)
    }

    fn list_settings(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError> {
        let mut items = Vec::new();
        for group in SettingsGroup::ALL {
            if !self.enabled.contains(group.data_type()) {
                continue;
            }
            let payload = self
                .settings
                .read(*group)
                .map_err(|e| SyncStoreError::Io(e.to_string()))?;
            let _canonical = canonical_json(&payload);
            let checksum = payload_checksum(&payload);
            // 设置类没有 updated_at：内容相对基线变化即视为变更。
            // 未变时把 content_ts 对齐到 last_synced_at，让 planner 判定为已同步。
            let known = self.state_of(group.data_type(), group.type_name());
            let content_ts = match known.as_ref() {
                Some(state) if state.checksum == checksum => {
                    state.last_synced_at.unwrap_or_else(now_seconds)
                }
                _ => now_seconds(),
            };
            items.push(self.snapshot(group.data_type(), group.type_name(), content_ts, &checksum));
        }
        Ok(items)
    }

    // ---------- 应用远端 ----------

    fn apply_quick_command(
        &self,
        record: &CloudSyncData,
        local_key: Option<&str>,
    ) -> Result<(), SyncStoreError> {
        let service = self.lock_service()?;
        let plain: QuickCommandPlainData = parse_record(&service, record).map_err(sync_error)?;
        drop(service);

        let mut command = crate::storage::QuickCommand::new(plain.command.clone());
        command.name = plain.name;
        command.description = plain.description;
        command.shortcut = plain.shortcut;
        command.pinned = plain.pinned;
        command.sort_order = plain.sort_order;
        command.group_name = plain.group_name;
        command.group_color = plain.group_color;
        // 内置命令不参与同步，因此同步下来的必定是用户命令
        command.is_builtin = false;

        let id = match local_key.and_then(|k| k.parse::<i64>().ok()) {
            Some(id) => {
                command.id = Some(id);
                self.quick_commands
                    .update(&command)
                    .map_err(|e| SyncStoreError::Io(e.to_string()))?;
                id
            }
            None => self
                .quick_commands
                .insert(&mut command)
                .map_err(|e| SyncStoreError::Io(e.to_string()))?,
        };

        self.record_synced(data_type::QUICK_COMMAND, &id.to_string(), record, None);
        self.mark_success(data_type::QUICK_COMMAND);
        Ok(())
    }

    fn apply_sftp_favorite(
        &self,
        record: &CloudSyncData,
        local_key: Option<&str>,
    ) -> Result<(), SyncStoreError> {
        let service = self.lock_service()?;
        let plain: SftpFavoritePlainData = parse_record(&service, record).map_err(sync_error)?;
        drop(service);

        let connection_key = plain.connection_name.clone().unwrap_or_default();
        let id = match local_key.and_then(|k| k.parse::<i64>().ok()) {
            Some(id) => {
                if let Some(mut existing) = self
                    .sftp
                    .get_row(id)
                    .map_err(|e| SyncStoreError::Io(e.to_string()))?
                {
                    existing.path = plain.path.clone();
                    existing.sort_order = plain.sort_order;
                    self.sftp
                        .update_row(&existing)
                        .map_err(|e| SyncStoreError::Io(e.to_string()))?;
                }
                id
            }
            None => {
                let row = SftpFavoriteRow {
                    id: 0,
                    connection_id: None,
                    connection_key,
                    path: plain.path.clone(),
                    sort_order: plain.sort_order,
                    created_at: now_seconds(),
                    updated_at: now_seconds(),
                };
                self.sftp
                    .insert_row(&row)
                    .map_err(|e| SyncStoreError::Io(e.to_string()))?
            }
        };

        self.record_synced(data_type::SFTP_FAVORITE, &id.to_string(), record, None);
        self.mark_success(data_type::SFTP_FAVORITE);
        Ok(())
    }

    fn apply_provider(
        &self,
        record: &CloudSyncData,
        local_key: Option<&str>,
    ) -> Result<(), SyncStoreError> {
        let service = self.lock_service()?;
        let plain: LlmProviderPlainData = parse_record(&service, record).map_err(sync_error)?;
        drop(service);

        let mut config = ProviderConfig {
            id: local_key.and_then(|k| k.parse::<i64>().ok()).unwrap_or(0),
            name: plain.name.clone(),
            provider_type: crate::llm::types::ProviderType::from_str(&plain.provider_type)
                .unwrap_or(crate::llm::types::ProviderType::OpenAICompatible),
            api_key: plain.api_key.clone(),
            api_base: plain.api_base.clone(),
            api_version: plain.api_version.clone(),
            model: plain.model.clone(),
            models: plain.models.clone(),
            max_tokens: plain.max_tokens,
            temperature: plain.temperature_milli.map(|milli| milli as f32 / 1000.0),
            enabled: plain.enabled,
            is_default: plain.is_default,
            created_at: now_seconds(),
            updated_at: now_seconds(),
        };

        let id = match local_key.and_then(|k| k.parse::<i64>().ok()) {
            Some(id) => {
                config.id = id;
                self.providers
                    .update(&config)
                    .map_err(|e| SyncStoreError::Io(e.to_string()))?;
                id
            }
            None => {
                let mut created = config;
                self.providers
                    .insert(&mut created)
                    .map_err(|e| SyncStoreError::Io(e.to_string()))?
            }
        };

        self.record_synced(data_type::LLM_PROVIDER, &id.to_string(), record, None);
        self.mark_success(data_type::LLM_PROVIDER);
        Ok(())
    }

    fn apply_saved_sql(&self, record: &CloudSyncData) -> Result<(), SyncStoreError> {
        let service = self.lock_service()?;
        let plain: SavedSqlPlainData = parse_record(&service, record).map_err(sync_error)?;
        drop(service);

        write_saved_sql_document(&SavedSqlDocument {
            document_id: plain.document_id.clone(),
            relative_path: plain.name.clone(),
            modified_at: now_seconds(),
            content: plain.content.clone(),
        });
        // 稳定键是 document_id：同一篇 SQL 换了路径时表现为移动，不会产生重复
        self.record_synced(data_type::SAVED_SQL, &plain.document_id, record, None);
        self.mark_success(data_type::SAVED_SQL);
        Ok(())
    }

    fn apply_note(&self, record: &CloudSyncData) -> Result<(), SyncStoreError> {
        let service = self.lock_service()?;
        let plain: NotePlainData = parse_record(&service, record).map_err(sync_error)?;
        drop(service);

        let Some(adapter) = &self.notes else {
            return Ok(());
        };
        adapter.write(&NoteSyncItem {
            document_id: plain.document_id.clone(),
            relative_path: plain.relative_path.clone(),
            modified_at: now_seconds(),
            content: plain.content.clone(),
        });
        // 稳定键是 document_id：同一篇笔记换了路径时，本地表现为重命名而不是新增
        self.record_synced(data_type::NOTE, &plain.document_id, record, None);
        self.mark_success(data_type::NOTE);
        Ok(())
    }

    fn apply_settings(&self, record: &CloudSyncData) -> Result<(), SyncStoreError> {
        let service = self.lock_service()?;
        let plain: SettingsPlainData = parse_record(&service, record).map_err(sync_error)?;
        drop(service);

        let Some(group) = SettingsGroup::from_type_name(&plain.group) else {
            return Err(SyncStoreError::Parse(format!(
                "unknown settings group: {}",
                plain.group
            )));
        };

        let local = self
            .settings
            .read(group)
            .map_err(|e| SyncStoreError::Io(e.to_string()))?;
        let known = self.state_of(group.data_type(), group.type_name());
        let base_value = known
            .as_ref()
            .and_then(|s| s.base_payload.clone())
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());

        // Q8 字段级三方合并；base 缺失时降级为整体采用远端（R14）
        let merge = merge_three_way(group, base_value.as_ref(), &local, &plain.payload);

        if !merge.is_clean() {
            let field = merge
                .conflicts
                .first()
                .map(|c| c.field.clone())
                .unwrap_or_default();
            self.mark_error(
                group.data_type(),
                "settings_conflict",
                &format!("{field} 本地与云端都被修改，暂按本地值保留"),
            );
        }

        self.settings
            .write(group, &merge.merged)
            .map_err(|e| SyncStoreError::Io(e.to_string()))?;

        let mut state = self
            .state_of(group.data_type(), group.type_name())
            .unwrap_or_else(|| self.base(group.data_type(), group.type_name()));
        state.cloud_id = Some(record.id.clone());
        state.checksum = payload_checksum(&merge.merged);
        state.last_synced_at = Some(now_seconds());
        state.content_ts = now_seconds();
        state.base_payload = Some(merge.merged.to_string());
        self.persist_state(&state)?;
        self.mark_success(group.data_type());
        Ok(())
    }
}

fn provider_plain(provider: &ProviderConfig) -> LlmProviderPlainData {
    super::personal::llm_provider_plain(
        provider.name.clone(),
        provider.provider_type.as_str().to_string(),
        provider.api_key.clone(),
        provider.api_base.clone(),
        provider.api_version.clone(),
        provider.model.clone(),
        provider.models.clone(),
        provider.max_tokens,
        provider.temperature,
        provider.enabled,
        provider.is_default,
    )
}

/// 已保存 SQL 的根目录；拿不到配置目录时返回 None
pub fn saved_sql_root() -> Option<std::path::PathBuf> {
    get_config_dir().ok().map(|dir| dir.join(SAVED_SQL_ROOT))
}

// 已保存 SQL 的读写/枚举统一收敛到 `saved_sql` 模块（以稳定 document_id 为身份），
// 这里只保留根目录解析供其复用。

fn quick_command_plain(command: &crate::storage::QuickCommand) -> QuickCommandPlainData {
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

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn split_local_id(local_id: &str) -> Option<(&str, &str)> {
    local_id.split_once(':')
}

/// `SyncError` → `SyncStoreError`（与 `personal/local_source.rs` 同款转换）
fn sync_error(error: crate::cloud_sync::service::SyncError) -> SyncStoreError {
    SyncStoreError::Parse(error.to_string())
}

#[async_trait]
impl PersonalSyncLocalSource for SyncScopeSource {
    async fn list_items(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError> {
        let mut items = Vec::new();
        if self.enabled.contains(data_type::QUICK_COMMAND) {
            items.extend(self.list_quick_commands()?);
        }
        if self.enabled.contains(data_type::SFTP_FAVORITE) {
            items.extend(self.list_sftp_favorites()?);
        }
        if self.enabled.contains(data_type::LLM_PROVIDER) {
            items.extend(self.list_providers()?);
        }
        if self.enabled.contains(data_type::SAVED_SQL) {
            items.extend(self.list_saved_sql()?);
        }
        if self.enabled.contains(data_type::NOTE) {
            items.extend(self.list_notes()?);
        }
        items.extend(self.list_settings()?);
        Ok(items)
    }

    async fn export_item(
        &self,
        item: &PersonalSyncItemSnapshot,
    ) -> Result<CloudSyncData, SyncStoreError> {
        let local_key = split_local_id(&item.local_id)
            .map(|(_, key)| key.to_string())
            .unwrap_or_default();
        let service = self.lock_service()?;
        let known = self.state_of(&item.data_type, &local_key);
        let cloud_id = known.and_then(|s| s.cloud_id).unwrap_or_default();

        match item.data_type.as_str() {
            data_type::QUICK_COMMAND => {
                let id: i64 = local_key.parse().unwrap_or_default();
                let command = self
                    .quick_commands
                    .get(id)
                    .map_err(|e| SyncStoreError::Io(e.to_string()))?
                    .ok_or_else(|| SyncStoreError::Parse(format!("quick command {id} missing")))?;
                let record = build_record_with_id(
                    &service,
                    &item.data_type,
                    &quick_command_plain(&command),
                    cloud_id,
                )
                .map_err(sync_error)?;
                Ok(record)
            }
            data_type::SFTP_FAVORITE => {
                let id: i64 = local_key.parse().unwrap_or_default();
                let favorite = self
                    .sftp
                    .get_row(id)
                    .map_err(|e| SyncStoreError::Io(e.to_string()))?
                    .ok_or_else(|| SyncStoreError::Parse(format!("sftp favorite {id} missing")))?;
                let plain = SftpFavoritePlainData {
                    format_version: PAYLOAD_FORMAT_VERSION,
                    path: favorite.path,
                    sort_order: favorite.sort_order,
                    connection_cloud_id: None,
                    connection_name: Some(favorite.connection_key),
                };
                let record = build_record_with_id(&service, &item.data_type, &plain, cloud_id)
                    .map_err(sync_error)?;
                Ok(record)
            }
            data_type::LLM_PROVIDER => {
                let id: i64 = local_key.parse().unwrap_or_default();
                let provider = self
                    .providers
                    .get(id)
                    .map_err(|e| SyncStoreError::Io(e.to_string()))?
                    .ok_or_else(|| SyncStoreError::Parse(format!("llm provider {id} missing")))?;
                let record = build_record_with_id(
                    &service,
                    &item.data_type,
                    &provider_plain(&provider),
                    cloud_id,
                )
                .map_err(sync_error)?;
                Ok(record)
            }
            data_type::SAVED_SQL => {
                let document = saved_sql_documents()
                    .into_iter()
                    .find(|document| document.document_id == local_key)
                    .ok_or_else(|| {
                        SyncStoreError::Parse(format!("saved sql {local_key} missing"))
                    })?;
                let plain = SavedSqlPlainData {
                    format_version: PAYLOAD_FORMAT_VERSION,
                    document_id: document.document_id,
                    db_type: "default".to_string(),
                    connection_cloud_id: None,
                    database: None,
                    name: document.relative_path,
                    content: document.content,
                };
                let record = build_record_with_id(&service, &item.data_type, &plain, cloud_id)
                    .map_err(sync_error)?;
                Ok(record)
            }
            data_type::NOTE => {
                let plain = self
                    .notes
                    .as_ref()
                    .and_then(|adapter| {
                        adapter
                            .list()
                            .into_iter()
                            .find(|note| note.document_id == local_key)
                    })
                    .map(|note| NotePlainData {
                        format_version: PAYLOAD_FORMAT_VERSION,
                        notebook_id: String::new(),
                        notebook_name: String::new(),
                        document_id: note.document_id.clone(),
                        relative_path: note.relative_path.clone(),
                        title: note.title(),
                        content: note.content,
                    })
                    .ok_or_else(|| SyncStoreError::Parse(format!("note {local_key} missing")))?;
                let record = build_record_with_id(&service, &item.data_type, &plain, cloud_id)
                    .map_err(sync_error)?;
                Ok(record)
            }
            other if SettingsGroup::from_data_type(other).is_some() => {
                let group = SettingsGroup::from_data_type(other).expect("checked above");
                let payload = self
                    .settings
                    .read(group)
                    .map_err(|e| SyncStoreError::Io(e.to_string()))?;
                let plain = SettingsPlainData {
                    format_version: PAYLOAD_FORMAT_VERSION,
                    group: group.type_name().to_string(),
                    payload,
                };
                let record =
                    build_record_with_id(&service, other, &plain, cloud_id).map_err(sync_error)?;
                Ok(record)
            }
            other => Err(SyncStoreError::Parse(format!(
                "unsupported sync scope data type: {other}"
            ))),
        }
    }

    async fn apply_remote(
        &self,
        record: &CloudSyncData,
        local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError> {
        if !self.enabled.contains(&record.data_type) {
            // 未勾选：静默跳过，不算成功也不算失败
            return Ok(());
        }
        let local_key = local
            .and_then(|item| split_local_id(&item.local_id).map(|(_, key)| key.to_string()))
            .or_else(|| self.find_local_key(&record.data_type, &record.id));

        match record.data_type.as_str() {
            data_type::QUICK_COMMAND => self.apply_quick_command(record, local_key.as_deref()),
            data_type::SFTP_FAVORITE => self.apply_sftp_favorite(record, local_key.as_deref()),
            data_type::LLM_PROVIDER => self.apply_provider(record, local_key.as_deref()),
            data_type::SAVED_SQL => self.apply_saved_sql(record),
            data_type::NOTE => self.apply_note(record),
            other if SettingsGroup::from_data_type(other).is_some() => self.apply_settings(record),
            // 未知类型：记 warning 跳过，绝不报错（R2 的未来兼容部分）
            other => {
                tracing::warn!("忽略未知的同步数据类型: {other}");
                Ok(())
            }
        }
    }

    async fn mark_synced(
        &self,
        local_id: &str,
        cloud_id: &str,
        synced_at: i64,
    ) -> Result<(), SyncStoreError> {
        let Some((record_type, local_key)) = split_local_id(local_id) else {
            return Ok(());
        };
        let mut state = self
            .state_of(record_type, local_key)
            .unwrap_or_else(|| self.base(record_type, local_key));
        state.cloud_id = Some(cloud_id.to_string());
        state.last_synced_at = Some(synced_at);
        self.persist_state(&state)?;
        self.mark_success(record_type);
        Ok(())
    }

    async fn delete_item(&self, item: &PersonalSyncItemSnapshot) -> Result<(), SyncStoreError> {
        let Some((record_type, local_key)) = split_local_id(&item.local_id) else {
            return Ok(());
        };
        match record_type {
            data_type::QUICK_COMMAND => {
                if let Ok(id) = local_key.parse::<i64>() {
                    self.quick_commands
                        .delete(id)
                        .map_err(|e| SyncStoreError::Io(e.to_string()))?;
                }
            }
            data_type::SFTP_FAVORITE => {
                if let Ok(id) = local_key.parse::<i64>() {
                    self.sftp
                        .delete_row(id)
                        .map_err(|e| SyncStoreError::Io(e.to_string()))?;
                }
            }
            data_type::LLM_PROVIDER => {
                if let Ok(id) = local_key.parse::<i64>() {
                    self.providers
                        .delete(id)
                        .map_err(|e| SyncStoreError::Io(e.to_string()))?;
                }
            }
            data_type::SAVED_SQL => {
                delete_saved_sql_document(local_key);
            }
            data_type::NOTE => {
                if let Some(adapter) = &self.notes {
                    adapter.delete(local_key);
                }
            }
            // 设置类 Singleton 没有「删除」语义：整组清空没有意义
            _ => {}
        }
        let _ = self
            .state
            .delete(&self.scope_env, &self.user_id, record_type, local_key);
        Ok(())
    }
}

/// `apply_payload` 在本模块未直接使用，但它是 `FileSettingsStore` 写回路径的一环，
/// 这里显式引用以避免被误认为死代码。
#[allow(dead_code)]
fn _apply_payload_is_used() -> bool {
    apply_payload as *const () as usize != 0
}

/// LLM 提供商载荷构造函数（供 Personal 与 OnetCloud 两条通道共用）
pub fn llm_provider_plain(
    name: String,
    provider_type: String,
    api_key: Option<String>,
    api_base: Option<String>,
    api_version: Option<String>,
    model: String,
    models: Vec<String>,
    max_tokens: Option<i32>,
    temperature: Option<f32>,
    enabled: bool,
    is_default: bool,
) -> LlmProviderPlainData {
    LlmProviderPlainData {
        format_version: PAYLOAD_FORMAT_VERSION,
        name,
        provider_type,
        api_key,
        api_base,
        api_version,
        model,
        models,
        max_tokens,
        // f32 无法直接参与 Eq 比较，转成毫值存整数
        temperature_milli: temperature.map(|v| (v * 1000.0).round() as i64),
        enabled,
        is_default,
    }
}
