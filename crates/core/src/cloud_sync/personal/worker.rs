use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::cloud_sync::models::CloudSyncData;

use super::{
    PersonalConflictType, PersonalSyncCloudKey, PersonalSyncItemSnapshot, PersonalSyncPlan,
    PersonalSyncPlanner, PersonalSyncRecordConflict, PersonalSyncStore, SyncDeviceId,
    SyncStoreError,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PersonalSyncEvent {
    FullScan,
    LocalChanged {
        data_type: String,
        local_id: String,
    },
    /// 本地条目被删除。
    ///
    /// `last_synced_at` 是删除前那条本地行记录的同步基线（远端 `updated_at` 的秒值，
    /// 与 planner 的 `last_synced_at` 同口径）。远端记录在那之后又变过，就说明有别的
    /// 设备刚改过它 —— 此时直接推墓碑会把对方的改动一起抹掉，改为挂起
    /// `LocalDeletedRemoteModified` 交给用户决定；基线与远端都不新则照旧推墓碑。
    /// 传 `None` 表示拿不到基线，此时一律按「删除胜出」处理。
    LocalDeleted {
        data_type: String,
        cloud_id: String,
        last_synced_at: Option<i64>,
    },
    RemoteChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    pub backend_profile_id: String,
    pub device_id: SyncDeviceId,
}

impl WorkerConfig {
    #[cfg(test)]
    pub fn test() -> Self {
        Self {
            backend_profile_id: "personal-test".to_string(),
            device_id: SyncDeviceId("test-device".to_string()),
        }
    }
}

#[async_trait]
pub trait PersonalSyncLocalSource: Send + Sync {
    async fn list_items(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError>;

    async fn export_item(
        &self,
        item: &PersonalSyncItemSnapshot,
    ) -> Result<CloudSyncData, SyncStoreError>;

    async fn apply_remote(
        &self,
        record: &CloudSyncData,
        local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError>;

    /// 一轮同步的收尾（在本地写入全部完成后调用）
    ///
    /// 用于解析跨记录的云端引用，例如按本次拉取到的云端记录对齐分组层级。
    async fn finalize_pass(&self, _records: &[CloudSyncData]) -> Result<(), SyncStoreError> {
        Ok(())
    }

    async fn mark_synced(
        &self,
        local_id: &str,
        cloud_id: &str,
        synced_at: i64,
    ) -> Result<(), SyncStoreError>;

    async fn delete_item(&self, item: &PersonalSyncItemSnapshot) -> Result<(), SyncStoreError>;
}

#[async_trait]
pub trait PersonalSyncConflictSink: Send + Sync {
    async fn paused_record_keys(&self) -> Result<HashSet<PersonalSyncCloudKey>, SyncStoreError> {
        Ok(HashSet::new())
    }

    async fn pause_record(
        &self,
        conflict: &PersonalSyncRecordConflict,
        local: Option<&PersonalSyncItemSnapshot>,
        remote: Option<&CloudSyncData>,
    ) -> Result<(), SyncStoreError>;

    /// 本地条目被删除后调用：这条记录上遗留的冲突已经失去意义，直接丢掉。
    ///
    /// 冲突解决要按云端记录反查本地条目（`local.list_items()`），本地条目消失后
    /// 冲突就再也解不开了，所以不能让它留在表里。
    async fn forget_record(&self, _data_type: &str, _cloud_id: &str) -> Result<(), SyncStoreError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct NoopConflictSink;

#[async_trait]
impl PersonalSyncConflictSink for NoopConflictSink {
    async fn pause_record(
        &self,
        _conflict: &PersonalSyncRecordConflict,
        _local: Option<&PersonalSyncItemSnapshot>,
        _remote: Option<&CloudSyncData>,
    ) -> Result<(), SyncStoreError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct WorkerState {
    pending: HashSet<PersonalSyncEvent>,
    active: bool,
    dirty: bool,
    /// 最近一轮同步结束的时刻。文件监听用它区分「自己刚写下的记录」和
    /// 「别的设备写进来的记录」——否则每轮同步都会再触发一轮空转。
    last_pass_end: Option<Instant>,
}

#[derive(Clone)]
pub struct PersonalSyncWorker<S, L, C = NoopConflictSink> {
    store: S,
    local: L,
    conflicts: C,
    config: WorkerConfig,
    planner: PersonalSyncPlanner,
    state: Arc<Mutex<WorkerState>>,
}

impl<S, L> PersonalSyncWorker<S, L, NoopConflictSink>
where
    S: PersonalSyncStore + Clone + Send + Sync,
    L: PersonalSyncLocalSource + Clone + Send + Sync,
{
    pub fn new(store: S, local: L, config: WorkerConfig) -> Self {
        Self::with_conflict_sink(store, local, NoopConflictSink, config)
    }
}

impl<S, L, C> PersonalSyncWorker<S, L, C>
where
    S: PersonalSyncStore + Clone + Send + Sync,
    L: PersonalSyncLocalSource + Clone + Send + Sync,
    C: PersonalSyncConflictSink + Clone + Send + Sync,
{
    pub fn with_conflict_sink(store: S, local: L, conflicts: C, config: WorkerConfig) -> Self {
        Self {
            store,
            local,
            conflicts,
            config,
            planner: PersonalSyncPlanner::new(),
            state: Arc::new(Mutex::new(WorkerState::default())),
        }
    }

    pub fn enqueue(&self, event: PersonalSyncEvent) {
        let mut state = self.state.lock().expect("personal sync worker state");
        if state.active {
            state.dirty = true;
        }
        state.pending.insert(event);
    }

    /// 是否还有没处理的事件（供同步运行时的调度器判断要不要跑一轮）。
    pub fn has_pending(&self) -> bool {
        !self
            .state
            .lock()
            .expect("personal sync worker state")
            .pending
            .is_empty()
    }

    /// 正在同步，或刚刚结束不久。
    ///
    /// 同步目录的文件监听会收到**自己写下去**的记录：那些事件不该再触发一轮
    /// 同步（否则每轮真实同步都跟着一轮空转）。这里用「进行中 + 宽限期」覆盖
    /// 写入到事件送达之间的延迟；代价是宽限期内别的设备的改动会晚一轮被发现。
    pub fn is_syncing_or_just_finished(&self) -> bool {
        const SELF_WRITE_GRACE: Duration = Duration::from_secs(5);
        let state = self.state.lock().expect("personal sync worker state");
        if state.active {
            return true;
        }
        state
            .last_pass_end
            .is_some_and(|finished_at| finished_at.elapsed() < SELF_WRITE_GRACE)
    }

    pub async fn drain_once(&self) -> Result<(), SyncStoreError> {
        let Some(events) = self.begin_drain() else {
            return Ok(());
        };

        let result = self.run_pass(events).await;
        self.finish_drain();
        result
    }

    fn begin_drain(&self) -> Option<HashSet<PersonalSyncEvent>> {
        let mut state = self.state.lock().expect("personal sync worker state");
        if state.active || state.pending.is_empty() {
            return None;
        }

        let events = std::mem::take(&mut state.pending);
        state.active = true;
        Some(events)
    }

    fn finish_drain(&self) {
        let mut state = self.state.lock().expect("personal sync worker state");
        state.active = false;
        state.last_pass_end = Some(Instant::now());
        if state.dirty {
            state.pending.insert(PersonalSyncEvent::FullScan);
            state.dirty = false;
        }
    }

    async fn run_pass(&self, events: HashSet<PersonalSyncEvent>) -> Result<(), SyncStoreError> {
        self.store.probe().await?;
        // 互斥：同一个同步包（同一台机器上的两个实例，或两台机器共享的目录 / WebDAV
        // 目标）同一时刻只允许一个 pass 在做「读记录 → 改 → 写回」。
        let lock = self.store.acquire_lock(&self.config.device_id).await?;
        let result = self.run_locked_pass(events).await;
        // 本地锁在 `Drop` 里就释放了；远端锁（WebDAV）必须显式释放，`Drop` 不能 await。
        // 释放失败不影响本轮结果 —— 远端锁还有 TTL 兜底。
        if let Err(error) = self.store.release_lock(&lock).await {
            tracing::warn!(error = %error, "failed to release personal sync lock");
        }
        result
    }

    async fn run_locked_pass(
        &self,
        events: HashSet<PersonalSyncEvent>,
    ) -> Result<(), SyncStoreError> {
        let local_items = self.local.list_items().await?;
        let remote_records = self.store.list_records(None, None).await?;
        // 挂起状态必须在处理删除之前取出来：墓碑处理要跳过已挂起的记录，
        // 否则会把冲突指向的那条本地行删掉，留下再也解不开的悬挂冲突。
        let mut paused = self.conflicts.paused_record_keys().await?;
        let local_delete_conflicts = self
            .apply_local_delete_events(&events, &remote_records)
            .await?;
        let tombstone_paused = self
            .apply_remote_tombstones(&local_items, &remote_records, &paused)
            .await?;
        // 本轮刚挂起的记录也要排除出 planner：否则同一次 pass 会把它重新上传，
        // 与刚记下的冲突自相矛盾（远端删了、本地又推回去）。
        paused.extend(tombstone_paused.iter().cloned());
        let tombstone_conflicts = tombstone_paused.len();
        let deleted = local_deleted_keys(&events)
            .into_iter()
            .map(|(key, _)| key)
            .collect::<HashSet<_>>();
        let active_remote_records = remote_records
            .into_iter()
            .filter(|record| {
                record.deleted_at.is_none()
                    && !deleted.contains(&PersonalSyncCloudKey {
                        data_type: record.data_type.clone(),
                        cloud_id: record.id.clone(),
                    })
            })
            .collect::<Vec<_>>();
        let plan = self
            .planner
            .plan(&local_items, &active_remote_records, &paused);

        self.apply_plan(&plan, &local_items, &active_remote_records)
            .await?;
        self.local.finalize_pass(&active_remote_records).await?;
        let deletion_conflicts = local_delete_conflicts + tombstone_conflicts;
        if deletion_conflicts > 0 {
            return Err(SyncStoreError::Conflict(format!(
                "{deletion_conflicts} deletion conflict(s) paused"
            )));
        }
        Ok(())
    }

    /// 处理「本地条目被删除」事件；返回本轮因此新挂起冲突的条数。
    async fn apply_local_delete_events(
        &self,
        events: &HashSet<PersonalSyncEvent>,
        records: &[CloudSyncData],
    ) -> Result<usize, SyncStoreError> {
        let mut conflicts = 0;
        for (key, last_synced_at) in local_deleted_keys(events) {
            // 用户显式删掉了本地条目 ⇒ 这条记录上遗留的冲突已经没有任何意义，
            // 顺手清掉，免得对话框里留下一个点哪个按钮都报错的冲突。
            self.conflicts
                .forget_record(&key.data_type, &key.cloud_id)
                .await?;
            let Some(record) = find_remote_by_cloud_key(records, &key) else {
                continue;
            };
            if record.deleted_at.is_some() {
                // 远端也已经是墓碑：两边都删了，没有需要同步的东西。
                continue;
            }
            if remote_changed_since_local_delete(last_synced_at, record) {
                // 远端在本地删除之后又被改过 ⇒ 直接推墓碑会把对方的改动一起抹掉。
                // 挂起冲突交给用户选：「使用远程版本」把记录重新拉回来，
                // 「使用本地版本」保留本地的删除意图。
                self.pause_deleted_local_conflict(record).await?;
                conflicts += 1;
                continue;
            }
            self.store
                .tombstone_record(&key.data_type, &key.cloud_id, Some(record.version))
                .await?;
        }
        Ok(conflicts)
    }

    /// 处理远端墓碑；返回本轮因此新挂起冲突的记录键。
    async fn apply_remote_tombstones(
        &self,
        items: &[PersonalSyncItemSnapshot],
        records: &[CloudSyncData],
        paused: &HashSet<PersonalSyncCloudKey>,
    ) -> Result<HashSet<PersonalSyncCloudKey>, SyncStoreError> {
        let mut conflicts = HashSet::new();
        for record in records.iter().filter(|record| record.deleted_at.is_some()) {
            let key = PersonalSyncCloudKey {
                data_type: record.data_type.clone(),
                cloud_id: record.id.clone(),
            };
            // 已经挂起冲突的记录保持原样：再删本地行会让冲突里存档的
            // `local_snapshot` 指向一个不存在的本地条目，冲突就永远解不开了。
            if paused.contains(&key) {
                continue;
            }
            let Some(item) = find_local_by_cloud_key(items, &key) else {
                continue;
            };
            if local_changed_since_sync(item) {
                // 本地自上次同步之后改过 ⇒ 不能跟着远端墓碑静默丢掉用户的改动，
                // 记一条 `LocalModifiedRemoteDeleted` 交给用户决定。
                self.pause_tombstone_conflict(
                    item,
                    record,
                    PersonalConflictType::LocalModifiedRemoteDeleted,
                )
                .await?;
                conflicts.insert(key);
                continue;
            }
            match self.local.delete_item(item).await {
                Ok(()) => {}
                Err(SyncStoreError::Conflict(_)) => {
                    // 本地拒绝删除（例如凭据仍被连接引用）⇒ 同样交给用户决定。
                    self.pause_tombstone_conflict(
                        item,
                        record,
                        PersonalConflictType::LocalModifiedRemoteDeleted,
                    )
                    .await?;
                    conflicts.insert(key);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(conflicts)
    }

    async fn pause_tombstone_conflict(
        &self,
        item: &PersonalSyncItemSnapshot,
        record: &CloudSyncData,
        conflict_type: PersonalConflictType,
    ) -> Result<(), SyncStoreError> {
        let conflict = PersonalSyncRecordConflict {
            local_id: Some(item.local_id.clone()),
            cloud_id: record.id.clone(),
            data_type: record.data_type.clone(),
            conflict_type,
        };
        self.conflicts
            .pause_record(&conflict, Some(item), Some(record))
            .await
    }

    /// 挂起「本地已删除、远端被别的设备改过」的冲突。
    ///
    /// 本地行此时已经不在了，所以没有 `local_snapshot` 可存：冲突只带远端快照，
    /// 解决时按 `(data_type, cloud_id)` 现场反查本地条目（见 `PersonalSyncConflictResolver`）。
    async fn pause_deleted_local_conflict(
        &self,
        record: &CloudSyncData,
    ) -> Result<(), SyncStoreError> {
        let conflict = PersonalSyncRecordConflict {
            local_id: None,
            cloud_id: record.id.clone(),
            data_type: record.data_type.clone(),
            conflict_type: PersonalConflictType::LocalDeletedRemoteModified,
        };
        self.conflicts
            .pause_record(&conflict, None, Some(record))
            .await
    }

    async fn apply_plan(
        &self,
        plan: &PersonalSyncPlan,
        local_items: &[PersonalSyncItemSnapshot],
        remote_records: &[CloudSyncData],
    ) -> Result<(), SyncStoreError> {
        self.apply_uploads(plan).await?;
        self.apply_cloud_updates(plan).await?;
        self.apply_local_updates(plan).await?;
        self.apply_downloads(plan).await?;
        self.apply_synced_marks(plan, local_items, remote_records)
            .await?;
        self.apply_conflicts(plan, local_items, remote_records)
            .await
    }

    async fn apply_uploads(&self, plan: &PersonalSyncPlan) -> Result<(), SyncStoreError> {
        for item in &plan.to_upload {
            let record = self.local.export_item(item).await?;
            let stored = self.store.upsert_record(&record, None).await?;
            self.mark_synced(item, &stored).await?;
        }
        Ok(())
    }

    async fn apply_cloud_updates(&self, plan: &PersonalSyncPlan) -> Result<(), SyncStoreError> {
        for (item, remote) in &plan.to_update_cloud {
            let record = self.local.export_item(item).await?;
            let stored = self
                .store
                .upsert_record(&record, Some(remote.version))
                .await?;
            self.mark_synced(item, &stored).await?;
        }
        Ok(())
    }

    async fn apply_local_updates(&self, plan: &PersonalSyncPlan) -> Result<(), SyncStoreError> {
        for (record, item) in &plan.to_update_local {
            self.local.apply_remote(record, Some(item)).await?;
            self.mark_synced(item, record).await?;
        }
        Ok(())
    }

    async fn apply_downloads(&self, plan: &PersonalSyncPlan) -> Result<(), SyncStoreError> {
        for record in &plan.to_download {
            self.local.apply_remote(record, None).await?;
        }
        Ok(())
    }

    async fn apply_synced_marks(
        &self,
        plan: &PersonalSyncPlan,
        items: &[PersonalSyncItemSnapshot],
        records: &[CloudSyncData],
    ) -> Result<(), SyncStoreError> {
        for key in &plan.to_mark_synced {
            let Some(item) = find_local_by_cloud_key(items, key) else {
                continue;
            };
            let Some(record) = find_remote_by_cloud_key(records, key) else {
                continue;
            };
            self.mark_synced(item, record).await?;
        }
        Ok(())
    }

    async fn apply_conflicts(
        &self,
        plan: &PersonalSyncPlan,
        items: &[PersonalSyncItemSnapshot],
        records: &[CloudSyncData],
    ) -> Result<(), SyncStoreError> {
        for conflict in &plan.conflicts {
            let local = conflict
                .local_id
                .as_deref()
                .and_then(|local_id| find_local_by_id(items, local_id));
            let remote = find_remote_by_cloud_key(
                records,
                &PersonalSyncCloudKey {
                    data_type: conflict.data_type.clone(),
                    cloud_id: conflict.cloud_id.clone(),
                },
            );
            self.conflicts.pause_record(conflict, local, remote).await?;
        }
        if !plan.conflicts.is_empty() {
            return Err(SyncStoreError::Conflict(format!(
                "{} personal sync conflict(s) paused",
                plan.conflicts.len()
            )));
        }
        Ok(())
    }

    async fn mark_synced(
        &self,
        item: &PersonalSyncItemSnapshot,
        record: &CloudSyncData,
    ) -> Result<(), SyncStoreError> {
        self.local
            .mark_synced(&item.local_id, &record.id, record.updated_at / 1000)
            .await
    }
}

fn find_local_by_cloud_key<'a>(
    items: &'a [PersonalSyncItemSnapshot],
    key: &PersonalSyncCloudKey,
) -> Option<&'a PersonalSyncItemSnapshot> {
    items.iter().find(|item| {
        item.data_type == key.data_type && item.cloud_id.as_deref() == Some(key.cloud_id.as_str())
    })
}

fn find_local_by_id<'a>(
    items: &'a [PersonalSyncItemSnapshot],
    local_id: &str,
) -> Option<&'a PersonalSyncItemSnapshot> {
    items.iter().find(|item| item.local_id == local_id)
}

/// 本地条目自上次同步之后是否被改过。
///
/// 与 planner 判定 `local_changed` 的口径一致：`updated_at` 比 `last_synced_at` 新。
fn local_changed_since_sync(item: &PersonalSyncItemSnapshot) -> bool {
    item.updated_at > item.last_synced_at.unwrap_or(0)
}

/// 远端记录是否在本地删除之后又被改过。
///
/// 基线（`last_synced_at`）与 planner 判定 `remote_changed` 的口径一致，都是秒。
/// 基线未知时不下结论，维持「删除胜出」的既有行为，避免给用户弹一个无从判断的冲突。
fn remote_changed_since_local_delete(baseline: Option<i64>, record: &CloudSyncData) -> bool {
    baseline.is_some_and(|baseline| record.updated_at / 1000 > baseline)
}

fn find_remote_by_cloud_key<'a>(
    records: &'a [CloudSyncData],
    key: &PersonalSyncCloudKey,
) -> Option<&'a CloudSyncData> {
    records
        .iter()
        .find(|record| record.data_type == key.data_type && record.id == key.cloud_id)
}

/// 本轮里「本地条目被删除」的事件，连同删除前记录的同步基线。
fn local_deleted_keys(
    events: &HashSet<PersonalSyncEvent>,
) -> Vec<(PersonalSyncCloudKey, Option<i64>)> {
    events
        .iter()
        .filter_map(|event| match event {
            PersonalSyncEvent::LocalDeleted {
                data_type,
                cloud_id,
                last_synced_at,
            } => Some((
                PersonalSyncCloudKey {
                    data_type: data_type.clone(),
                    cloud_id: cloud_id.clone(),
                },
                *last_synced_at,
            )),
            _ => None,
        })
        .collect()
}
