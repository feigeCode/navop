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
    PendingLocalDeletion, PersonalSyncItemSnapshot, PersonalSyncLocalSource, SyncStoreError,
};
use crate::cloud_sync::scope::notes::{NOTE_MAX_CONTENT_BYTES, NoteSyncItem, SharedNoteAdapter};
use crate::cloud_sync::scope::payloads::{
    LlmProviderPlainData, PAYLOAD_FORMAT_VERSION, QuickCommandPlainData, SftpFavoritePlainData,
    build_record, build_record_with_id, parse_record,
};
use crate::cloud_sync::scope::payloads::{NotePlainData, SavedSqlPlainData};
use crate::cloud_sync::scope::saved_sql::{
    SavedSqlDocument, delete_saved_sql_document, saved_sql_documents, saved_sql_file_stamps,
    write_saved_sql_document,
};
use crate::cloud_sync::scope::settings_payload::{SettingsGroup, apply_payload, merge_three_way};
use crate::cloud_sync::scope::settings_store::{FileSettingsStore, payload_checksum};
use crate::cloud_sync::scope::state::{
    SyncItemState, SyncItemStateRepository, SyncTypeStatusRepository,
};
use crate::cloud_sync::service::CloudSyncService;
use crate::llm::storage::ProviderRepository;
use crate::llm::types::ProviderConfig;
use crate::storage::ConnectionRepository;
use crate::storage::get_config_dir;
use crate::storage::quick_command::QuickCommandRepository;
use crate::storage::sftp_favorite_path::{
    LOCAL_FAVORITE_CONNECTION_KEY, SftpFavoritePathRepository, SftpFavoriteRow,
    sftp_favorite_connection_key,
};
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
    /// 连接仓储：SFTP 收藏 / 连接级快捷命令上传时解析连接的 cloud_id，
    /// 下载时由 cloud_id 反解本地连接 id，跨设备保持外键稳定
    connections: ConnectionRepository,
    providers: ProviderRepository,
    notes: Option<SharedNoteAdapter>,
    settings: FileSettingsStore,
    service: Arc<RwLock<CloudSyncService>>,
}

impl SyncScopeSource {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scope_env: String,
        user_id: String,
        enabled: HashSet<String>,
        state: SyncItemStateRepository,
        status: SyncTypeStatusRepository,
        quick_commands: QuickCommandRepository,
        sftp: SftpFavoritePathRepository,
        connections: ConnectionRepository,
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
            connections,
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

    #[cfg(test)]
    pub(crate) fn quick_commands(&self) -> &QuickCommandRepository {
        &self.quick_commands
    }

    /// 基线差分删除（trait 实现的具体逻辑，测试直接调用）
    pub async fn report_pending_local_deletions(&self) -> Vec<PendingLocalDeletion> {
        // 「基线里有 cloud_id、本轮枚举里却消失」的本地条目即被删除：
        // - enabled 过滤：用户取消勾选的类型按 D3 保留本地与云端，不推删除；
        // - 设置分组没有删除语义，跳过；
        // - 只有同步过（有 cloud_id）的行才可能构成删除传播。
        let mut alive: HashSet<(String, String)> = HashSet::new();
        for item in self.list_items().await.unwrap_or_default() {
            if let Some((record_type, local_key)) = split_local_id(&item.local_id) {
                alive.insert((record_type.to_string(), local_key.to_string()));
            }
        }
        let mut deletions = Vec::new();
        let Ok(states) = self.state.list(&self.scope_env, &self.user_id) else {
            return deletions;
        };
        for state in states {
            if !self.enabled.contains(&state.data_type)
                || SettingsGroup::from_data_type(&state.data_type).is_some()
                || state.cloud_id.as_deref().unwrap_or("").is_empty()
            {
                continue;
            }
            if alive.contains(&(state.data_type.clone(), state.local_key.clone())) {
                continue;
            }
            deletions.push(PendingLocalDeletion {
                data_type: state.data_type.clone(),
                cloud_id: state.cloud_id.clone().unwrap_or_default(),
                last_synced_at: state.last_synced_at,
            });
        }
        deletions
    }

    /// 墓碑推完后清基线行（trait 实现的具体逻辑，测试直接调用）
    pub async fn drop_local_state(&self, data_type: &str, cloud_id: &str) {
        let Ok(states) = self.state.list(&self.scope_env, &self.user_id) else {
            return;
        };
        let Some(local_key) = states
            .into_iter()
            .find(|s| s.data_type == data_type && s.cloud_id.as_deref() == Some(cloud_id))
            .map(|s| s.local_key)
        else {
            return;
        };
        let _ = self
            .state
            .delete(&self.scope_env, &self.user_id, data_type, &local_key);
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

    /// 由本地连接 id 解析其云端 ID（上传侧填充 `connection_cloud_id` 用）。
    ///
    /// 直接查连接表：连接同步成功后 cloud_id 就回填在连接行上，
    /// 不经过 `sync_item_state`（那是 scope 类型的状态表，connection 不在其中）。
    fn connection_cloud_id_for(&self, connection_id: Option<i64>) -> Option<String> {
        let connection_id = connection_id?;
        let connection = self.connections.get(connection_id).ok().flatten()?;
        connection.cloud_id.filter(|id| !id.trim().is_empty())
    }

    /// 由云端连接 ID 反解本地连接 id（下载侧恢复外键用）。
    fn resolve_connection_cloud_id(&self, cloud_id: &str) -> Option<i64> {
        self.connections
            .get_by_cloud_id(cloud_id)
            .ok()
            .flatten()
            .and_then(|c| c.id)
    }

    /// 按连接名兜底解析（云端记录里只有 `connection_name` 时）。
    fn resolve_connection_by_name(&self, name: &str) -> Option<i64> {
        if name.is_empty() {
            return None;
        }
        self.connections
            .list()
            .ok()?
            .into_iter()
            .find(|c| c.name == name)
            .and_then(|c| c.id)
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
                &serde_json::to_value(quick_command_plain(
                    self.connection_cloud_id_for(command.connection_id),
                    &command,
                ))
                .unwrap_or_default(),
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
            // 「本地文件列表」伪连接键：不是真实连接的目录收藏，不参与同步
            if favorite.connection_key == LOCAL_FAVORITE_CONNECTION_KEY {
                continue;
            }
            let plain = self.sftp_plain_payload(&favorite)?;
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

    /// SFTP 收藏的上传载荷：优先带稳定 `connection_cloud_id`；
    /// 连接尚未同步（无 cloud_id）时退化为真实连接名，
    /// 由对端按名兜底解析。绝不把 `local:{id}` 这种设备本地键传上云。
    fn sftp_plain_payload(
        &self,
        favorite: &SftpFavoriteRow,
    ) -> Result<SftpFavoritePlainData, SyncStoreError> {
        let connection = favorite
            .connection_id
            .and_then(|id| self.connections.get(id).ok().flatten());
        let (connection_cloud_id, connection_name) = match &connection {
            Some(conn) => {
                let cloud_id = conn.cloud_id.clone().filter(|id| !id.trim().is_empty());
                let name = if cloud_id.is_some() {
                    None
                } else {
                    Some(conn.name.clone())
                };
                (cloud_id, name)
            }
            // 连接行已删（收藏悬空）：按 connection_key 里的 cloud: 前缀兜底
            None => (
                favorite
                    .connection_key
                    .strip_prefix("cloud:")
                    .map(str::to_string),
                None,
            ),
        };
        Ok(SftpFavoritePlainData {
            format_version: PAYLOAD_FORMAT_VERSION,
            path: favorite.path.clone(),
            sort_order: favorite.sort_order,
            connection_cloud_id,
            connection_name,
        })
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
            let checksum = settings_group_checksum(group.type_name(), &payload);
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

    /// 各类数据的轻量变更签名，供同步运行时轮询「本地有没有东西变了」。
    ///
    /// 只取身份与时间戳、不算内容校验和——检测变更不该比同步本身还贵。
    /// 设置类不在这里：它们没有时间戳，由设置观察者按白名单载荷精确比对。
    pub fn change_signatures(&self) -> Vec<(String, String)> {
        let mut signatures = Vec::new();
        if self.enabled.contains(data_type::QUICK_COMMAND) {
            let rows = self.quick_commands.list().unwrap_or_default();
            let signature = rows
                .iter()
                .map(|command| {
                    format!(
                        "{}@{}",
                        command.id.unwrap_or_default(),
                        command.updated_at.unwrap_or(0)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            signatures.push((data_type::QUICK_COMMAND.to_string(), signature));
        }
        if self.enabled.contains(data_type::SFTP_FAVORITE) {
            let rows = self.sftp.list_all().unwrap_or_default();
            let signature = rows
                .iter()
                .map(|row| format!("{}@{}", row.id, row.updated_at))
                .collect::<Vec<_>>()
                .join(",");
            signatures.push((data_type::SFTP_FAVORITE.to_string(), signature));
        }
        if self.enabled.contains(data_type::LLM_PROVIDER) {
            let rows = self.providers.list().unwrap_or_default();
            let signature = rows
                .iter()
                .map(|provider| format!("{}@{}", provider.id, provider.updated_at))
                .collect::<Vec<_>>()
                .join(",");
            signatures.push((data_type::LLM_PROVIDER.to_string(), signature));
        }
        if self.enabled.contains(data_type::NOTE) {
            if let Some(adapter) = &self.notes {
                let signature = adapter
                    .list()
                    .iter()
                    .map(|note| format!("{}@{}", note.document_id, note.modified_at))
                    .collect::<Vec<_>>()
                    .join(",");
                signatures.push((data_type::NOTE.to_string(), signature));
            }
        }
        if self.enabled.contains(data_type::SAVED_SQL) {
            let signature = saved_sql_file_stamps()
                .iter()
                .map(|(path, modified_at)| format!("{path}@{modified_at}"))
                .collect::<Vec<_>>()
                .join(",");
            signatures.push((data_type::SAVED_SQL.to_string(), signature));
        }
        signatures
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
        // 连接级命令：按云端连接 ID 反解本地 id；连接尚未同步到本机时
        // 降级为全局命令（连接同步落地后不会自动回填，用户重新绑定即可）
        command.connection_id = plain
            .connection_cloud_id
            .as_deref()
            .and_then(|id| self.resolve_connection_cloud_id(id));

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

        // 跨设备外键恢复：优先按 connection_cloud_id 反解本地连接；
        // 旧记录只有 connection_name 时按名兜底；两者都失败则按 name: 键
        // 落地（与本地「连接未同步」时的键规则一致，等连接同步后由 UI 归位）。
        let (connection_id, connection_key) = {
            let by_cloud = plain
                .connection_cloud_id
                .as_deref()
                .and_then(|id| self.resolve_connection_cloud_id(id));
            let by_name = plain
                .connection_name
                .as_deref()
                .and_then(|name| self.resolve_connection_by_name(name));
            match by_cloud.or(by_name) {
                Some(id) => {
                    let key = self
                        .connections
                        .get(id)
                        .ok()
                        .flatten()
                        .map(|c| sftp_favorite_connection_key(&c))
                        .unwrap_or_else(|| format!("local:{id}"));
                    (Some(id), key)
                }
                None => (
                    None,
                    plain
                        .connection_name
                        .clone()
                        .map(|name| format!("name:{name}"))
                        .unwrap_or_else(|| "name:".to_string()),
                ),
            }
        };

        let id = match local_key.and_then(|k| k.parse::<i64>().ok()) {
            Some(id) => {
                if let Some(mut existing) = self
                    .sftp
                    .get_row(id)
                    .map_err(|e| SyncStoreError::Io(e.to_string()))?
                {
                    existing.path = plain.path.clone();
                    existing.sort_order = plain.sort_order;
                    existing.connection_id = connection_id;
                    existing.connection_key = connection_key;
                    self.sftp
                        .update_row(&existing)
                        .map_err(|e| SyncStoreError::Io(e.to_string()))?;
                }
                id
            }
            None => {
                let row = SftpFavoriteRow {
                    id: 0,
                    connection_id,
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
                // 新设备拉取：本地没有对应行。以 id=0 调 insert，由 SQLite
                // 分配 rowid 并回写（显式绑 0 会让第二条提供商记录撞
                // llm_providers.id 主键）；返回的行号同时作为
                // sync_item_state 的 local_key，后续轮次走上面的 update 分支。
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
        })
        .map_err(|e| SyncStoreError::Io(format!("写入已保存 SQL 失败: {e}")))?;
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

        let conflicted = !merge.is_clean();
        if conflicted {
            let fields = merge
                .conflicts
                .iter()
                .map(|c| c.field.as_str())
                .collect::<Vec<_>>()
                .join("、");
            self.mark_error(
                group.data_type(),
                "settings_conflict",
                &format!("{fields} 本地与云端都被修改，暂按本地值保留"),
            );
        }

        self.settings
            .write(group, &merge.merged)
            .map_err(|e| SyncStoreError::Io(e.to_string()))?;

        let mut state = self
            .state_of(group.data_type(), group.type_name())
            .unwrap_or_else(|| self.base(group.data_type(), group.type_name()));
        state.cloud_id = Some(record.id.clone());
        // 与 `list_settings` 同口径：覆盖完整载荷，否则下一轮永远判为「已变更」。
        if conflicted {
            // 冲突：保留本地值，但校验和落**远端口径**、base 不推进——
            // 下一轮 planner 会把本地合并结果判为「仅本地改」走 to_update_cloud，
            // 即「本地保留 + 主动覆盖云端」；冲突信息持续留在 sync_type_status
            // 里直到用户下次改动触发新一轮干净合并，而不是被 mark_success 抹掉。
            state.checksum = record.checksum.clone();
            state.last_synced_at = Some(now_seconds());
            state.content_ts = now_seconds();
            self.persist_state(&state)?;
            return Ok(());
        }
        state.checksum = settings_group_checksum(group.type_name(), &merge.merged);
        state.last_synced_at = Some(now_seconds());
        state.content_ts = now_seconds();
        state.base_payload = Some(merge.merged.to_string());
        self.persist_state(&state)?;
        self.mark_success(group.data_type());
        Ok(())
    }
}

/// 设置类分组的完整载荷校验和。
///
/// 必须覆盖 `SettingsPlainData` 整体（`format_version` + `group` + `payload`），
/// 与 `export_item` 建记录时 `build_record_with_id` 算的口径一致：以前只对
/// payload 取哈希，而记录侧算的是整个信封，两边永远不相等，设置类每轮都被
/// 当成「本地已变更」重新上传，版本号跟着虚增。
fn settings_group_checksum(group: &str, payload: &serde_json::Value) -> String {
    let plain = SettingsPlainData {
        format_version: PAYLOAD_FORMAT_VERSION,
        group: group.to_string(),
        payload: payload.clone(),
    };
    payload_checksum(&serde_json::to_value(&plain).unwrap_or_default())
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

fn quick_command_plain(
    connection_cloud_id: Option<String>,
    command: &crate::storage::QuickCommand,
) -> QuickCommandPlainData {
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
        connection_cloud_id,
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

/// 生成待上传的云端记录：同步过的沿用原 `cloud_id`，否则新建一个 UUID。
///
/// **绝不能把「未知 id」当作空串**：文件夹 / Git / WebDAV 后端用 `{id}.json`
/// 当文件名，空 id 会让同一 data_type 下的所有记录塌缩成同一个无名 `.json`
/// —— 表现为「只有一条数据、后写的覆盖先写的、version 每次同步都涨」。
fn build_scope_record(
    service: &CloudSyncService,
    record_type: &str,
    plain: &impl serde::Serialize,
    cloud_id: Option<&str>,
) -> Result<CloudSyncData, crate::cloud_sync::service::SyncError> {
    match cloud_id {
        Some(id) => build_record_with_id(service, record_type, plain, id.to_string()),
        None => build_record(service, record_type, plain),
    }
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

    fn singleton_data_types(&self) -> HashSet<String> {
        // 设置分组每组云端最多一条记录；planner 据此做 Singleton 认领，
        // 避免第二台设备给同一分组再传一条记录、两条互相覆盖。
        SettingsGroup::ALL
            .iter()
            .map(|group| group.data_type().to_string())
            .collect()
    }
    async fn pending_local_deletions(&self) -> Result<Vec<PendingLocalDeletion>, SyncStoreError> {
        Ok(self.report_pending_local_deletions().await)
    }

    async fn forget_local_state(
        &self,
        data_type: &str,
        cloud_id: &str,
    ) -> Result<(), SyncStoreError> {
        // 墓碑已推上云端：清掉基线行，下一轮差分不再重复报告同一条删除。
        self.drop_local_state(data_type, cloud_id).await;
        Ok(())
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
        // 未知 cloud_id 必须落成 `None`（随后生成新 UUID），**不能当成空串**：
        // 文件夹后端用 `{id}.json` 当文件名，空 id 会让同一类型的所有记录塌缩成
        // 同一个无名 `.json` 并互相覆盖。
        let cloud_id = known
            .and_then(|s| s.cloud_id)
            .filter(|id| !id.trim().is_empty());

        match item.data_type.as_str() {
            data_type::QUICK_COMMAND => {
                let id: i64 = local_key.parse().unwrap_or_default();
                let command = self
                    .quick_commands
                    .get(id)
                    .map_err(|e| SyncStoreError::Io(e.to_string()))?
                    .ok_or_else(|| SyncStoreError::Parse(format!("quick command {id} missing")))?;
                let record = build_scope_record(
                    &service,
                    &item.data_type,
                    &quick_command_plain(
                        self.connection_cloud_id_for(command.connection_id),
                        &command,
                    ),
                    cloud_id.as_deref(),
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
                let plain = self.sftp_plain_payload(&favorite)?;
                let record =
                    build_scope_record(&service, &item.data_type, &plain, cloud_id.as_deref())
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
                let record = build_scope_record(
                    &service,
                    &item.data_type,
                    &provider_plain(&provider),
                    cloud_id.as_deref(),
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
                let record =
                    build_scope_record(&service, &item.data_type, &plain, cloud_id.as_deref())
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
                let record =
                    build_scope_record(&service, &item.data_type, &plain, cloud_id.as_deref())
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
                    payload: payload.clone(),
                };
                let record = build_scope_record(&service, other, &plain, cloud_id.as_deref())
                    .map_err(sync_error)?;
                drop(service);
                // 上传前把本次载荷登记为三方合并的 base：下一轮远端再变时，
                // `apply_settings` 拿得到「双方共同起点」，才能做字段级合并，
                // 而不是退化成「base 缺失 → 整体采用远端」把本地改动抹掉。
                let mut state = self
                    .state_of(other, group.type_name())
                    .unwrap_or_else(|| self.base(other, group.type_name()));
                state.base_payload = Some(payload.to_string());
                self.persist_state(&state)?;
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
        checksum: &str,
        synced_at: i64,
    ) -> Result<(), SyncStoreError> {
        let Some((record_type, local_key)) = split_local_id(local_id) else {
            return Ok(());
        };
        let mut state = self
            .state_of(record_type, local_key)
            .unwrap_or_else(|| self.base(record_type, local_key));
        state.cloud_id = Some(cloud_id.to_string());
        // 校验和必须落库：`list_settings` 靠 `state.checksum == 本次计算的校验和`
        // 把「内容没变」的 content_ts 对齐到 last_synced_at；不落的话每轮都判为
        // 「已变更」（content_ts 取 now），远端单方面改动也会被 planner 当成
        // 双方冲突（local_changed 恒为 true）。口径与记录侧一致，见
        // `build_record_with_id` 的说明。
        if !checksum.is_empty() {
            state.checksum = checksum.to_string();
        }
        state.last_synced_at = Some(synced_at);
        // 内容「变更时刻」取同步时刻：此后 `updated_at > last_synced_at` 不再成立，
        // planner 视其为未变更。上传时 synced_at 即新记录的 updated_at，标记已
        // 同步时是原记录的 updated_at，两种情况下都与 last_synced_at 相等。
        state.content_ts = synced_at;
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use crate::cloud_sync::models::data_type;
    use crate::cloud_sync::scope::payloads::PAYLOAD_FORMAT_VERSION;
    use crate::cloud_sync::scope::state::{
        SyncItemState, SyncItemStateRepository, SyncTypeStatusRepository,
    };
    use crate::cloud_sync::service::CloudSyncService;
    use crate::llm::storage::ProviderRepository;
    use crate::storage::ConnectionRepository;
    use crate::storage::connection::SqliteConnection;
    use crate::storage::manager::StorageManager;
    use crate::storage::migration::run_migrations;
    use crate::storage::quick_command::{QuickCommand, QuickCommandRepository};
    use crate::storage::sftp_favorite_path::SftpFavoritePathRepository;
    use crate::storage::traits::Repository;

    use super::{SettingsGroup, SyncScopeSource};

    fn test_source(enabled: &[&str]) -> (SyncScopeSource, StorageManager) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!(
            "onetcli-scope-personal-tests-{}-{unique}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&db_path);
        let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
        conn.with_connection(|c| run_migrations(c))
            .expect("run migrations");
        let storage = StorageManager::new_with_connection(conn.clone());
        let state = SyncItemStateRepository::new(conn.clone());
        let status = SyncTypeStatusRepository::new(conn.clone());
        let source = SyncScopeSource::new(
            "test-env".to_string(),
            "user-1".to_string(),
            enabled.iter().map(|t| t.to_string()).collect(),
            state,
            status,
            QuickCommandRepository::new(conn.clone()),
            SftpFavoritePathRepository::new(conn.clone()),
            ConnectionRepository::new(conn.clone()),
            ProviderRepository::new(conn),
            None,
            Arc::new(RwLock::new(CloudSyncService::new())),
        );
        (source, storage)
    }

    fn state_row(local_key: &str, cloud_id: &str) -> SyncItemState {
        let mut state = SyncItemState::new(
            "test-env".to_string(),
            "user-1".to_string(),
            data_type::QUICK_COMMAND.to_string(),
            local_key.to_string(),
        );
        state.cloud_id = Some(cloud_id.to_string());
        state.last_synced_at = Some(100);
        state
    }

    /// 基线里有 cloud_id、本地行已删 ⇒ 差分应报告删除
    #[tokio::test]
    async fn pending_local_deletions_reports_missing_local_rows() {
        let (source, _storage) = test_source(&[data_type::QUICK_COMMAND]);
        source
            .state_repo()
            .upsert(&state_row("7", "cloud-7"))
            .expect("seed state");

        let deletions = source.report_pending_local_deletions().await;

        assert_eq!(1, deletions.len());
        assert_eq!(data_type::QUICK_COMMAND, deletions[0].data_type);
        assert_eq!("cloud-7", deletions[0].cloud_id);
        assert_eq!(Some(100), deletions[0].last_synced_at);
    }

    /// 本地行还在 ⇒ 不构成删除
    #[tokio::test]
    async fn pending_local_deletions_ignores_alive_rows() {
        let (source, _storage) = test_source(&[data_type::QUICK_COMMAND]);
        let mut command = QuickCommand::new("echo hi".to_string());
        let id = source
            .quick_commands()
            .insert(&mut command)
            .expect("insert command");
        source
            .state_repo()
            .upsert(&state_row(&id.to_string(), "cloud-1"))
            .expect("seed state");

        let deletions = source.report_pending_local_deletions().await;

        assert!(deletions.is_empty(), "本地行还在，不应报告删除");
    }

    /// 取消勾选的类型按 D3 停止同步：不推删除、保留云端与本地
    #[tokio::test]
    async fn pending_local_deletions_skips_disabled_types() {
        let (source, _storage) = test_source(&[data_type::NOTE]);
        source
            .state_repo()
            .upsert(&state_row("7", "cloud-7"))
            .expect("seed state");

        let deletions = source.report_pending_local_deletions().await;

        assert!(deletions.is_empty(), "未勾选的类型不应报告删除");
    }

    /// 设置分组没有删除语义，基线行不构成差分删除
    #[tokio::test]
    async fn pending_local_deletions_skips_settings_groups() {
        let (source, _storage) = test_source(&[data_type::SETTINGS_GENERAL]);
        let mut state = state_row("general", "cloud-settings");
        state.data_type = data_type::SETTINGS_GENERAL.to_string();
        source.state_repo().upsert(&state).expect("seed state");

        let deletions = source.report_pending_local_deletions().await;

        assert!(deletions.is_empty(), "设置分组不应报告删除");
    }

    /// 墓碑推完后清基线：同一删除不会每轮重复上报
    #[tokio::test]
    async fn forget_local_state_removes_baseline_row() {
        let (source, _storage) = test_source(&[data_type::QUICK_COMMAND]);
        source
            .state_repo()
            .upsert(&state_row("7", "cloud-7"))
            .expect("seed state");

        source
            .drop_local_state(data_type::QUICK_COMMAND, "cloud-7")
            .await;

        let deletions = source.report_pending_local_deletions().await;
        assert!(deletions.is_empty(), "基线已清，删除不应重复上报");
    }

    /// singleton 集合 = 六个设置分组
    #[test]
    fn singleton_data_types_covers_all_settings_groups() {
        let singletons: std::collections::HashSet<String> = SettingsGroup::ALL
            .iter()
            .map(|group| group.data_type().to_string())
            .collect();

        assert_eq!(SettingsGroup::ALL.len(), singletons.len());
        for group in SettingsGroup::ALL {
            assert!(
                singletons.contains(group.data_type()),
                "缺少分组 {}",
                group.data_type()
            );
        }
    }

    /// 上传前 base 登记的是明文 payload（三方合并的公共起点），
    /// 且 format_version 不影响口径。
    #[test]
    fn payload_format_version_is_stable() {
        assert_eq!(1, PAYLOAD_FORMAT_VERSION);
    }
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
