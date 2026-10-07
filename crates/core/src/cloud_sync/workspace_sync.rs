//! 工作空间同步处理器
//!
//! 通过实现 `SyncTypeHandler` trait，将工作空间同步逻辑接入通用同步流程 `generic_sync`。
//!
//! 分组层级（父子分组）跨设备传递只使用云端 ID 引用：
//!
//! - 上传：把本地 `parent_id` 解析成父分组的 `cloud_id` 写进载荷；
//! - 上传顺序：父分组先处理，否则父分组还没有 `cloud_id` 可用；
//! - 下载：载荷里的父分组引用在收尾阶段解析回本地 `parent_id`（见
//!   `crate::cloud_sync::workspace_hierarchy`），因此不受下载顺序影响。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::cloud_sync::engine::SyncEngine;
use crate::cloud_sync::models::{CloudSyncData, Team, WorkspaceParentLink};
use crate::cloud_sync::service::{CloudSyncService, SyncError};
use crate::cloud_sync::sync_type::{GenericSyncPlan, SyncTypeHandler, SyncableItem};
use crate::cloud_sync::workspace_hierarchy::{
    cloud_parent_links, reconcile_parent_links, sort_ancestors_first,
};
use crate::storage::traits::Repository;
use crate::storage::{Workspace, WorkspaceRepository};

/// 工作空间同步类型处理器
pub struct WorkspaceSyncType;

impl WorkspaceSyncType {
    /// 解密本次同步看到的云端分组记录，得到「云端 ID → 父分组关系」
    fn parent_links(
        &self,
        engine: &SyncEngine,
        cloud_data: &[CloudSyncData],
    ) -> Vec<(String, WorkspaceParentLink)> {
        cloud_parent_links(cloud_data, |record| {
            self.decrypt_parent_link(engine, record)
        })
    }

    fn decrypt_parent_link(
        &self,
        engine: &SyncEngine,
        record: &CloudSyncData,
    ) -> Option<WorkspaceParentLink> {
        let service = engine.crypto_service.read().ok()?;
        service
            .decrypt_sync_data_workspace_with_parent_link(record)
            .ok()
            .map(|(_, link)| link)
    }

    /// 本地已有分组但云端载荷还不带层级信息时，补一次上传
    ///
    /// 旧版本上传的载荷没有父分组引用，只靠一次普通同步不会重新上传，
    /// 层级就会永远留在云端之外。这里把这类分组补进「更新云端」计划；
    /// 若本轮云端记录已经决定要覆盖本地（下载 / 更新本地），则留到下一轮，
    /// 避免把云端刚更新的其它字段回退掉。
    fn plan_hierarchy_backfill(
        &self,
        plan: &mut GenericSyncPlan<Workspace>,
        links: &[(String, WorkspaceParentLink)],
        cloud_data: &[CloudSyncData],
        local_items: &[Workspace],
    ) {
        let covered_local_ids: HashSet<i64> = plan
            .to_upload
            .iter()
            .chain(plan.to_update_cloud.iter().map(|(item, _)| item))
            .chain(plan.to_update_local.iter().map(|(_, item)| item))
            .filter_map(|item| item.local_id())
            .collect();
        let uploaded_cloud_ids: HashMap<i64, &str> = local_items
            .iter()
            .filter_map(|item| item.local_id().zip(item.cloud_id.as_deref()))
            .collect();

        for (cloud_id, link) in links {
            if *link != WorkspaceParentLink::Unknown {
                continue;
            }
            let local_item = local_items
                .iter()
                .find(|item| item.cloud_id.as_deref() == Some(cloud_id.as_str()));
            let Some(local_item) = local_item else {
                continue;
            };
            // 只在父分组已经拿到云端 ID 时补传：否则这一轮上传仍会写成
            // 「层级未知」，白白多跑一次同步（父分组一旦上传，下一轮自然补上）。
            let Some(parent_cloud_id) = local_item
                .parent_id
                .and_then(|id| uploaded_cloud_ids.get(&id))
            else {
                continue;
            };
            if local_item
                .local_id()
                .is_some_and(|id| covered_local_ids.contains(&id))
            {
                continue;
            }
            let Some(record) = cloud_data.iter().find(|record| record.id == *cloud_id) else {
                continue;
            };

            tracing::info!(
                "[同步计划] 分组 {} 的父分组引用尚未上传（父分组 {}），补一次上传",
                local_item.name,
                parent_cloud_id
            );
            plan.to_update_cloud
                .push((local_item.clone(), record.clone()));
        }
    }
}

impl SyncTypeHandler for WorkspaceSyncType {
    type Item = Workspace;

    fn data_type(&self) -> &'static str {
        "workspace"
    }

    fn display_name(&self) -> &'static str {
        "工作空间"
    }

    fn queue_key(&self) -> &'static str {
        "workspace"
    }

    fn list_local(&self, engine: &SyncEngine) -> Result<Vec<Workspace>, SyncError> {
        let repo = workspace_repository(engine)?;

        let workspaces = repo
            .list()
            .map_err(|e| SyncError::StorageError(e.to_string()))?;

        // 父分组必须先上传：子分组要引用父分组的云端 ID。
        Ok(sort_ancestors_first(workspaces))
    }

    fn insert_local(&self, engine: &SyncEngine, item: &mut Workspace) -> Result<(), SyncError> {
        let repo = workspace_repository(engine)?;

        repo.insert(item)
            .map_err(|e| SyncError::StorageError(e.to_string()))?;

        Ok(())
    }

    fn update_local_item(&self, engine: &SyncEngine, item: &Workspace) -> Result<(), SyncError> {
        let repo = workspace_repository(engine)?;

        repo.update_from_cloud(item)
            .map_err(|e| SyncError::StorageError(e.to_string()))
    }

    fn delete_local(&self, engine: &SyncEngine, id: i64) -> Result<(), SyncError> {
        let repo = workspace_repository(engine)?;

        repo.delete(id)
            .map_err(|e| SyncError::StorageError(e.to_string()))
    }

    fn on_uploaded(
        &self,
        engine: &SyncEngine,
        local_id: i64,
        cloud_id: &str,
    ) -> Result<(), SyncError> {
        let repo = workspace_repository(engine)?;

        repo.update_cloud_id(local_id, Some(cloud_id.to_string()))
            .map_err(|e| SyncError::StorageError(e.to_string()))
    }

    fn decrypt_name(&self, service: &CloudSyncService, data: &CloudSyncData) -> Option<String> {
        service
            .decrypt_sync_data_workspace(data)
            .ok()
            .map(|ws| ws.name)
    }

    fn decrypt(
        &self,
        service: &CloudSyncService,
        data: &CloudSyncData,
    ) -> Result<Workspace, SyncError> {
        service.decrypt_sync_data_workspace(data)
    }

    fn encrypt(
        &self,
        engine: &SyncEngine,
        service: &CloudSyncService,
        item: &Workspace,
        teams: &[Team],
    ) -> Result<CloudSyncData, SyncError> {
        // 父分组还没有云端 ID（尚未上传或本轮上传失败）时只能按「层级未知」上报，
        // 由 list_local 的祖先优先顺序保证父分组先取得云端 ID。
        let parent_link = WorkspaceParentLink::for_upload(
            item.parent_id,
            engine.workspace_cloud_id_for_local_id(item.parent_id)?,
        );
        service.prepare_workspace_sync_data_upload(item, parent_link, item.team_id(), teams)
    }

    fn adjust_plan(
        &self,
        engine: &SyncEngine,
        plan: &mut GenericSyncPlan<Workspace>,
        cloud_data: &[CloudSyncData],
    ) {
        let Ok(local_items) = self.list_local(engine) else {
            return;
        };
        let links = self.parent_links(engine, cloud_data);

        self.plan_hierarchy_backfill(plan, &links, cloud_data, &local_items);
    }

    fn finalize_sync(
        &self,
        engine: &SyncEngine,
        cloud_data: &[CloudSyncData],
    ) -> Result<(), SyncError> {
        let repo = workspace_repository(engine)?;
        let workspaces = repo
            .list()
            .map_err(|e| SyncError::StorageError(e.to_string()))?;
        let links = self.parent_links(engine, cloud_data);

        // 所有写入都已完成，此时解析父分组引用与顺序无关，也能自愈
        // 「父分组还没落地」的情况。
        let updated = reconcile_parent_links(repo.as_ref(), &workspaces, &links)?;
        if updated > 0 {
            tracing::info!("[同步] 已对齐 {} 个分组的父分组关系", updated);
        }

        Ok(())
    }

    fn pending_deletion_entity_type(&self) -> &'static str {
        "workspace"
    }
}

fn workspace_repository(engine: &SyncEngine) -> Result<Arc<WorkspaceRepository>, SyncError> {
    engine
        .storage
        .get::<WorkspaceRepository>()
        .ok_or_else(|| SyncError::StorageError("WorkspaceRepository not found".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud_sync::models::data_type;

    fn local_workspace(
        id: i64,
        name: &str,
        parent_id: Option<i64>,
        cloud_id: Option<&str>,
    ) -> Workspace {
        let mut workspace = Workspace::new(name.to_string());
        workspace.id = Some(id);
        workspace.parent_id = parent_id;
        workspace.cloud_id = cloud_id.map(str::to_string);
        workspace
    }

    fn cloud_record(id: &str) -> CloudSyncData {
        CloudSyncData {
            id: id.to_string(),
            owner_id: "owner".to_string(),
            team_id: None,
            data_type: data_type::WORKSPACE.to_string(),
            encrypted_data: String::new(),
            key_version: 1,
            checksum: String::new(),
            version: 1,
            updated_at: 1_000_000,
            deleted_at: None,
        }
    }

    fn backfill(
        plan: &mut GenericSyncPlan<Workspace>,
        links: &[(String, WorkspaceParentLink)],
        cloud_data: &[CloudSyncData],
        local_items: &[Workspace],
    ) {
        WorkspaceSyncType.plan_hierarchy_backfill(plan, links, cloud_data, local_items);
    }

    #[test]
    fn backfill_reuploads_child_when_cloud_payload_has_no_hierarchy() {
        let mut plan = GenericSyncPlan::default();
        let links = vec![("cloud-child".to_string(), WorkspaceParentLink::Unknown)];
        let cloud_data = vec![cloud_record("cloud-child")];
        let local_items = vec![
            local_workspace(1, "parent", None, Some("cloud-parent")),
            local_workspace(2, "child", Some(1), Some("cloud-child")),
        ];

        backfill(&mut plan, &links, &cloud_data, &local_items);

        assert_eq!(1, plan.to_update_cloud.len());
        assert_eq!(Some(2), plan.to_update_cloud[0].0.id);
        assert_eq!("cloud-child", plan.to_update_cloud[0].1.id);
    }

    #[test]
    fn backfill_waits_until_the_parent_has_a_cloud_id() {
        let mut plan = GenericSyncPlan::default();
        let links = vec![("cloud-child".to_string(), WorkspaceParentLink::Unknown)];
        let cloud_data = vec![cloud_record("cloud-child")];
        // 父分组还未上传（没有 cloud_id），补传也只能写成「层级未知」。
        let local_items = vec![
            local_workspace(1, "parent", None, None),
            local_workspace(2, "child", Some(1), Some("cloud-child")),
        ];

        backfill(&mut plan, &links, &cloud_data, &local_items);

        assert!(plan.to_update_cloud.is_empty());
    }

    #[test]
    fn backfill_ignores_groups_without_local_parent() {
        let mut plan = GenericSyncPlan::default();
        let links = vec![("cloud-root".to_string(), WorkspaceParentLink::Unknown)];
        let cloud_data = vec![cloud_record("cloud-root")];
        let local_items = vec![local_workspace(1, "root", None, Some("cloud-root"))];

        backfill(&mut plan, &links, &cloud_data, &local_items);

        assert!(plan.to_update_cloud.is_empty());
    }

    #[test]
    fn backfill_skips_groups_already_covered_by_the_plan() {
        let parent = local_workspace(1, "parent", None, Some("cloud-parent"));
        let child = local_workspace(2, "child", Some(1), Some("cloud-child"));
        let links = vec![("cloud-child".to_string(), WorkspaceParentLink::Unknown)];
        let cloud_data = vec![cloud_record("cloud-child")];
        let local_items = [parent, child.clone()];

        let mut upload_plan = GenericSyncPlan::default();
        upload_plan.to_upload.push(child.clone());
        backfill(&mut upload_plan, &links, &cloud_data, &local_items);

        let mut download_plan = GenericSyncPlan::default();
        download_plan
            .to_update_local
            .push((cloud_record("cloud-child"), child.clone()));
        backfill(&mut download_plan, &links, &cloud_data, &local_items);

        assert!(upload_plan.to_update_cloud.is_empty());
        assert!(download_plan.to_update_cloud.is_empty());
    }

    #[test]
    fn backfill_ignores_hierarchy_aware_payloads() {
        let mut plan = GenericSyncPlan::default();
        let links = vec![(
            "cloud-child".to_string(),
            WorkspaceParentLink::Cloud("cloud-parent".to_string()),
        )];
        let cloud_data = vec![cloud_record("cloud-child")];
        let local_items = vec![
            local_workspace(1, "parent", None, Some("cloud-parent")),
            local_workspace(2, "child", Some(1), Some("cloud-child")),
        ];

        backfill(&mut plan, &links, &cloud_data, &local_items);

        assert!(plan.to_update_cloud.is_empty());
    }
}
