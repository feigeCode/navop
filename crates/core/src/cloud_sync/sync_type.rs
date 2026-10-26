//! 同步类型抽象
//!
//! 提供 `SyncableItem` 和 `SyncTypeHandler` 两个核心 trait，
//! 将不同数据类型的同步逻辑从流程控制中解耦。
//! 新增同步数据类型只需实现 trait 并通过 `SyncEngine::register_type` 注册即可。

use crate::cloud_sync::engine::SyncEngine;
use crate::cloud_sync::models::{CloudSyncData, Team};
use crate::cloud_sync::service::{CloudSyncService, SyncError};
use crate::storage::PendingCloudDeletion;

/// 可同步数据项的统一抽象
///
/// 为各类本地数据（如 `StoredConnection`、`Workspace`）提供统一的字段访问接口，
/// 使通用同步流程无需关心具体数据结构。
pub trait SyncableItem: Clone + Send + Sync + 'static {
    /// 本地数据库 ID
    fn local_id(&self) -> Option<i64>;
    /// 设置本地 ID
    fn set_local_id(&mut self, id: Option<i64>);
    /// 数据项名称（用于日志和错误信息）
    fn item_name(&self) -> &str;
    /// 云端同步 ID
    fn cloud_id(&self) -> Option<&str>;
    /// 设置云端 ID
    fn set_cloud_id(&mut self, cloud_id: Option<String>);
    /// 更新时间戳（秒）
    fn updated_at(&self) -> Option<i64>;

    /// 是否启用同步（默认 true）
    fn is_sync_enabled(&self) -> bool {
        true
    }

    /// 最后同步时间戳（默认 None，Connection 有此字段）
    fn last_synced_at(&self) -> Option<i64> {
        None
    }

    /// 团队归属 ID（默认 None）
    fn team_id(&self) -> Option<&str> {
        None
    }

    /// 稳定业务键（scope 类型的 local_key）：键控回调用它回写基线/删除本地行。
    ///
    /// 默认取 local_id 的字符串形式（rowid 类型天然稳定）；无主键类型
    /// （设置组 / 已保存 SQL / 笔记）覆盖为各自的业务键。空串表示
    /// 该类型不支持键控（回落到按 id 的旧回调）。
    fn sync_key(&self) -> &str {
        ""
    }

    /// 本地内容校验和（与云端 record.checksum 同口径时可用于短路比较；
    /// 默认 None 表示该类型不提供，计划器跳过短路逻辑）
    fn local_checksum(&self) -> Option<String> {
        None
    }
}

/// 同步计划（通用版）
///
/// 替代原有的 `WorkspaceSyncPlan`，适用于所有简单同步场景。
#[derive(Debug)]
pub struct GenericSyncPlan<T: SyncableItem> {
    /// 需要上传的数据项（本地新增）
    pub to_upload: Vec<T>,
    /// 需要更新到云端的数据项 (本地数据, 对应的云端同步数据)
    pub to_update_cloud: Vec<(T, CloudSyncData)>,
    /// 需要下载的数据项（云端新增）
    pub to_download: Vec<CloudSyncData>,
    /// 需要更新到本地的数据项 (云端同步数据, 对应的本地数据)
    pub to_update_local: Vec<(CloudSyncData, T)>,
    /// checksum 一致的无变化项（只刷新 last_synced_at，不产生网络操作）
    pub unchanged: Vec<T>,
}

impl<T: SyncableItem> Default for GenericSyncPlan<T> {
    fn default() -> Self {
        Self {
            to_upload: Vec::new(),
            to_update_cloud: Vec::new(),
            to_download: Vec::new(),
            to_update_local: Vec::new(),
            unchanged: Vec::new(),
        }
    }
}

/// 数据类型同步处理器
///
/// 封装特定数据类型的存储操作、加解密逻辑和同步回调。
/// 实现此 trait 后，通过 `SyncEngine::register_type` 注册即可自动接入通用同步流程。
pub trait SyncTypeHandler: Send + Sync + 'static {
    type Item: SyncableItem;

    // --- 标识 ---

    /// 数据类型标识（对应 `CloudSyncData.data_type`，如 "connection" / "workspace"）
    fn data_type(&self) -> &'static str;

    /// 显示名称（用于日志，如 "连接" / "工作空间"）
    fn display_name(&self) -> &'static str;

    /// 操作队列 key
    fn queue_key(&self) -> &'static str;

    // --- 本地存储操作 ---

    /// 获取所有本地数据项
    fn list_local(&self, engine: &SyncEngine) -> Result<Vec<Self::Item>, SyncError>;

    /// 插入新的本地数据项
    fn insert_local(&self, engine: &SyncEngine, item: &mut Self::Item) -> Result<(), SyncError>;

    /// 更新本地数据项（从云端同步下来后的更新）
    fn update_local_item(&self, engine: &SyncEngine, item: &Self::Item) -> Result<(), SyncError>;

    /// 删除本地数据项
    fn delete_local(&self, engine: &SyncEngine, id: i64) -> Result<(), SyncError>;

    /// 删除本地数据项的键控版本：按稳定 key 定位业务行。
    ///
    /// 返回 `Err` 表示「按 key 找不到业务行」等异常，由调用方记录；
    /// 不再像旧行为那样静默 `Ok(())` 吞掉删除失败。
    fn delete_local_with_key(&self, _engine: &SyncEngine, _key: &str) -> Result<(), SyncError> {
        Ok(())
    }

    // --- 同步后回调 ---

    /// 上传成功后的回调（通常更新 cloud_id）
    fn on_uploaded(
        &self,
        engine: &SyncEngine,
        local_id: i64,
        cloud_id: &str,
    ) -> Result<(), SyncError>;

    /// 上传成功后的键控回调：按稳定 key（local_key）回写 cloud_id / checksum。
    ///
    /// 无主键类型（设置 / 已保存 SQL / 笔记）的 `local_id` 是哈希合成 ID，
    /// 反查业务行会失败；计划器手里同时有 key，键控版本直接 upsert 基线即可。
    /// 默认实现为空操作，未实现键控语义的旧 handler 不受影响（仍走 `on_uploaded`）。
    fn on_uploaded_with_key(
        &self,
        _key: &str,
        _cloud_id: &str,
        _checksum: &str,
    ) -> Result<(), SyncError> {
        Ok(())
    }

    /// checksum 短路时仅刷新同步时间（不写 cloud_id/checksum/水位）。
    /// 默认空操作：没有基线表的旧 handler 无事可做。
    fn touch_synced(&self, _key: &str) -> Result<(), SyncError> {
        Ok(())
    }

    /// 更新云端成功后回写水位（content_ts 取远端 updated_at 毫秒值）。
    ///
    /// 默认实现回落到 on_uploaded_with_key（无水位语义的类型至少保住 cloud_id）。
    fn persist_watermark(
        &self,
        key: &str,
        cloud_id: &str,
        _remote_updated_ms: i64,
    ) -> Result<(), SyncError> {
        self.on_uploaded_with_key(key, cloud_id, "")
    }

    // --- 加解密 ---

    /// 尝试解密云端数据获取名称（用于构建 name_map，解密失败返回 None）
    fn decrypt_name(&self, service: &CloudSyncService, data: &CloudSyncData) -> Option<String>;

    /// 解密云端数据为本地数据项
    fn decrypt(
        &self,
        service: &CloudSyncService,
        data: &CloudSyncData,
    ) -> Result<Self::Item, SyncError>;

    /// 加密本地数据项为云端数据
    ///
    /// 传入 `engine` 是因为部分类型（如工作空间）在加密前需要
    /// 把本地引用解析为云端 ID，而不是上传仅在本机有效的整数 ID。
    fn encrypt(
        &self,
        engine: &SyncEngine,
        service: &CloudSyncService,
        item: &Self::Item,
        teams: &[Team],
    ) -> Result<CloudSyncData, SyncError>;

    // --- 同步计划修正与收尾（默认无操作） ---

    /// 修正同步计划（在计划计算完成后、执行前调用）
    ///
    /// 用于处理数据类型特有的补偿：例如工作空间需要把「云端载荷尚未携带
    /// 层级信息」的本地分组重新上传，让旧数据也能补上父分组引用。
    fn adjust_plan(
        &self,
        _engine: &SyncEngine,
        _plan: &mut GenericSyncPlan<Self::Item>,
        _cloud_data: &[CloudSyncData],
    ) {
    }

    /// 同步收尾（在所有操作执行完成后调用）
    ///
    /// 用于解析跨记录的云端引用：例如把所有工作空间的本地父分组 ID
    /// 按本次拉动到的云端记录重新对齐。
    fn finalize_sync(
        &self,
        _engine: &SyncEngine,
        _cloud_data: &[CloudSyncData],
    ) -> Result<(), SyncError> {
        Ok(())
    }

    // --- 待删除处理（有默认实现） ---

    /// 待删除实体类型标识（默认与 data_type 相同）
    fn pending_deletion_entity_type(&self) -> &'static str {
        self.data_type()
    }

    /// 获取待删除列表
    fn list_pending_deletions(&self, engine: &SyncEngine) -> Vec<PendingCloudDeletion> {
        let Some(repo) = engine
            .storage
            .get::<crate::storage::PendingCloudDeletionRepository>()
        else {
            return Vec::new();
        };

        let entity_type = self.pending_deletion_entity_type();
        // 旧类型保留原行为；新类型走通用 entity_type 查询
        let result = match entity_type {
            "connection" => repo.list_connections(),
            "workspace" => repo.list_workspaces(),
            _ => repo.list_by_entity_type(entity_type),
        };

        result.unwrap_or_default()
    }

    /// 删除一条待删除记录
    fn remove_pending_deletion(
        &self,
        engine: &SyncEngine,
        cloud_id: &str,
    ) -> Result<(), SyncError> {
        let repo = engine
            .storage
            .get::<crate::storage::PendingCloudDeletionRepository>()
            .ok_or_else(|| {
                SyncError::StorageError("PendingCloudDeletionRepository not found".to_string())
            })?;

        // 旧类型 remove 只按 cloud_id；新类型带 entity_type 防误删他类记录
        let entity_type = self.pending_deletion_entity_type();
        let result = match entity_type {
            "connection" | "workspace" => repo.remove(cloud_id),
            _ => repo.remove_by_entity_type(entity_type, cloud_id),
        };
        result.map_err(|e| SyncError::StorageError(e.to_string()))
    }
}
