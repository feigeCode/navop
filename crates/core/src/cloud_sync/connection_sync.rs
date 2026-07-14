use crate::cloud_sync::engine::{SyncEngine, SyncFuture, SyncHandler};
use crate::cloud_sync::models::{
    CloudSyncData, ConflictResolution, SyncConflict, SyncPlan, SyncResult, data_type,
};
use crate::cloud_sync::queue::SyncOperation;
use crate::cloud_sync::service::SyncError;
use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionRepository, PendingCloudDeletionRepository, StoredConnection, WorkspaceRepository,
};
use std::collections::{HashMap, HashSet};

const CONNECTION_QUEUE_KEY: &str = "connection";

pub struct ConnectionSyncHandler;

impl SyncHandler for ConnectionSyncHandler {
    fn name(&self) -> &'static str {
        "连接"
    }

    fn sync<'a>(&'a self, engine: &'a SyncEngine) -> SyncFuture<'a> {
        Box::pin(async move { engine.sync_connections().await })
    }
}

impl SyncEngine {
    async fn sync_connections(&self) -> Result<SyncResult, SyncError> {
        let mut result = SyncResult::default();

        let pending_deletions = self.process_pending_deletions().await;
        tracing::info!("[同步] 处理待删除列表完成: {} 个", pending_deletions.len());

        let local_connections = self.get_local_connections()?;
        let sync_enabled_count = local_connections.iter().filter(|c| c.sync_enabled).count();
        tracing::info!(
            "[同步] 本地连接: {} 个，其中 {} 个启用同步",
            local_connections.len(),
            sync_enabled_count
        );

        let decrypt_failures = self.get_local_decrypt_failures()?;
        let failure_ids: HashSet<i64> = decrypt_failures.iter().map(|(id, _)| *id).collect();
        if !decrypt_failures.is_empty() {
            let preview = decrypt_failures
                .iter()
                .take(5)
                .map(|(id, name)| format!("{}:{}", id, name))
                .collect::<Vec<_>>()
                .join(", ");
            tracing::warn!(
                "[同步] 检测到 {} 个本地连接解密失败，将跳过其上传与拉取更新: {}",
                decrypt_failures.len(),
                preview
            );
        }

        tracing::info!("[同步] 正在获取云端同步数据列表...");
        let cloud_sync_data = self
            .cloud_client
            .list_sync_data(Some(data_type::CONNECTION), None, None)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        tracing::info!("[同步] 云端连接同步数据: {} 个", cloud_sync_data.len());

        // 过滤掉团队密钥未解锁的团队数据，避免解密失败中断同步
        let cloud_sync_data: Vec<_> = cloud_sync_data
            .into_iter()
            .filter(|d| match &d.team_id {
                Some(tid) => {
                    let unlocked = self.is_team_unlocked(tid);
                    if !unlocked {
                        tracing::info!("[同步] 跳过未解锁团队 {} 的云端连接数据 {}", tid, d.id);
                    }
                    unlocked
                }
                None => true,
            })
            .collect();
        tracing::info!("[同步] 可处理的云端连接数据: {} 个", cloud_sync_data.len());

        // 解密一次建立 cloud_id → name 映射
        let cloud_name_map = self.build_cloud_name_map(&cloud_sync_data);

        let soft_deleted_cloud_ids =
            self.process_cloud_soft_deleted_sync_data(&cloud_sync_data, &local_connections)?;
        if !soft_deleted_cloud_ids.is_empty() {
            tracing::info!(
                "[同步] 处理云端软删除: 删除了 {} 个本地连接",
                soft_deleted_cloud_ids.len()
            );
            result.deleted += soft_deleted_cloud_ids.len();
        }
        let local_connections =
            Self::filter_soft_deleted_local_connections(local_connections, &soft_deleted_cloud_ids);

        let active_cloud_data: Vec<_> = cloud_sync_data
            .into_iter()
            .filter(|d| d.deleted_at.is_none())
            .collect();
        tracing::info!("[同步] 活跃云端连接数据: {} 个", active_cloud_data.len());

        let readable_team_ids = self.readable_team_ids();
        let plan = self.calculate_sync_plan(
            &local_connections,
            &active_cloud_data,
            &cloud_name_map,
            &readable_team_ids,
        )?;
        tracing::info!(
            "[同步计划] 上传: {}, 更新云端: {}, 下载: {}, 更新本地: {}, 冲突: {}",
            plan.to_upload.len(),
            plan.to_update_cloud.len(),
            plan.to_download.len(),
            plan.to_update_local.len(),
            plan.conflicts.len()
        );

        let active_conflicts: Vec<_> = plan
            .conflicts
            .iter()
            .filter(|conflict| match conflict.local.id {
                Some(id) => !failure_ids.contains(&id),
                None => true,
            })
            .cloned()
            .collect();
        let skipped_conflicts = plan.conflicts.len().saturating_sub(active_conflicts.len());
        if skipped_conflicts > 0 {
            tracing::warn!(
                "[同步] 跳过 {} 个与解密失败连接相关的冲突",
                skipped_conflicts
            );
        }

        let resolved_conflicts = self.resolve_conflicts(&active_conflicts)?;
        result.conflicts = active_conflicts;

        let local_connection_map: HashMap<i64, StoredConnection> = local_connections
            .iter()
            .filter_map(|conn| conn.id.map(|id| (id, conn.clone())))
            .collect();
        let cloud_data_map: HashMap<String, CloudSyncData> = active_cloud_data
            .iter()
            .map(|d| (d.id.clone(), d.clone()))
            .collect();

        let mut operations = Vec::new();
        for local_conn in &plan.to_upload {
            if let Some(local_id) = local_conn.id {
                if failure_ids.contains(&local_id) {
                    tracing::warn!(
                        "[同步] 跳过上传（解密失败）: {} ({})",
                        local_conn.name,
                        local_id
                    );
                    continue;
                }
                operations.push(SyncOperation::Upload { local_id });
            } else {
                result
                    .errors
                    .push(format!("上传失败 {}: 缺少本地 ID", local_conn.name));
            }
        }

        for (local_conn, cloud_data) in &plan.to_update_cloud {
            if let Some(local_id) = local_conn.id {
                if failure_ids.contains(&local_id) {
                    tracing::warn!(
                        "[同步] 跳过更新云端（解密失败）: {} ({})",
                        local_conn.name,
                        local_id
                    );
                    continue;
                }
                operations.push(SyncOperation::UpdateCloud {
                    local_id,
                    cloud_id: cloud_data.id.clone(),
                });
            } else {
                result
                    .errors
                    .push(format!("更新云端失败 {}: 缺少本地 ID", local_conn.name));
            }
        }

        for cloud_data in &plan.to_download {
            operations.push(SyncOperation::Download(cloud_data.id.clone()));
        }

        for (cloud_data, local_conn) in &plan.to_update_local {
            if let Some(local_id) = local_conn.id {
                if failure_ids.contains(&local_id) {
                    tracing::warn!(
                        "[同步] 跳过拉取更新（解密失败）: {} ({})",
                        local_conn.name,
                        local_id
                    );
                    continue;
                }
                operations.push(SyncOperation::UpdateLocal {
                    local_id,
                    cloud_id: cloud_data.id.clone(),
                });
            } else {
                let name = cloud_name_map
                    .get(&cloud_data.id)
                    .cloned()
                    .unwrap_or_else(|| cloud_data.id.clone());
                result
                    .errors
                    .push(format!("更新本地失败 {}: 缺少本地 ID", name));
            }
        }

        let mut queue = self.take_operation_queue(CONNECTION_QUEUE_KEY)?;
        queue.enqueue_all(operations);
        queue.retry_failed();

        while let Some(queued_operation) = queue.dequeue() {
            let operation = queued_operation.operation.clone();
            match operation {
                SyncOperation::Upload { local_id } => {
                    let Some(local_conn) = local_connection_map.get(&local_id) else {
                        result
                            .errors
                            .push(format!("上传失败 {}: 本地数据不存在", local_id));
                        continue;
                    };
                    match self.upload_connection(local_conn).await {
                        Ok(created_cloud_data) => {
                            match self.update_sync_status_after_cloud_write(
                                local_conn,
                                &created_cloud_data,
                                None,
                            ) {
                                Ok(()) => {
                                    result.uploaded += 1;
                                    tracing::info!("[上传] 成功: {}", local_conn.name);
                                }
                                Err(e) => {
                                    let error_message =
                                        format!("上传失败 {}: {}", local_conn.name, e);
                                    result.errors.push(error_message.clone());
                                    queue.mark_failed(queued_operation, error_message);
                                }
                            }
                        }
                        Err(e) => {
                            let error_message = format!("上传失败 {}: {}", local_conn.name, e);
                            result.errors.push(error_message.clone());
                            queue.mark_failed(queued_operation, error_message);
                        }
                    }
                }
                SyncOperation::UpdateCloud { local_id, cloud_id } => {
                    let Some(local_conn) = local_connection_map.get(&local_id) else {
                        result
                            .errors
                            .push(format!("更新云端失败 {}: 本地数据不存在", local_id));
                        continue;
                    };
                    let Some(cloud_data) = cloud_data_map.get(&cloud_id) else {
                        result
                            .errors
                            .push(format!("更新云端失败 {}: 云端数据不存在", cloud_id));
                        continue;
                    };
                    match self.update_cloud_connection(local_conn, cloud_data).await {
                        Ok(updated_cloud_data) => {
                            match self.update_sync_status_after_cloud_write(
                                local_conn,
                                &updated_cloud_data,
                                None,
                            ) {
                                Ok(()) => {
                                    result.uploaded += 1;
                                    tracing::info!("[更新云端] 成功: {}", local_conn.name);
                                }
                                Err(e) => {
                                    let error_message =
                                        format!("更新云端失败 {}: {}", local_conn.name, e);
                                    result.errors.push(error_message.clone());
                                    queue.mark_failed(queued_operation, error_message);
                                }
                            }
                        }
                        Err(e) => {
                            let error_message = format!("更新云端失败 {}: {}", local_conn.name, e);
                            result.errors.push(error_message.clone());
                            queue.mark_failed(queued_operation, error_message);
                        }
                    }
                }
                SyncOperation::UpdateLocal { local_id, cloud_id } => {
                    let Some(local_conn) = local_connection_map.get(&local_id) else {
                        result
                            .errors
                            .push(format!("更新本地失败 {}: 本地数据不存在", local_id));
                        continue;
                    };
                    let Some(cloud_data) = cloud_data_map.get(&cloud_id) else {
                        result
                            .errors
                            .push(format!("更新本地失败 {}: 云端数据不存在", cloud_id));
                        continue;
                    };
                    let name = cloud_name_map
                        .get(&cloud_data.id)
                        .cloned()
                        .unwrap_or_else(|| cloud_data.id.clone());
                    match self.update_local_connection(cloud_data, local_conn).await {
                        Ok(()) => {
                            result.downloaded += 1;
                            tracing::info!("[更新本地] 成功: {}", name);
                        }
                        Err(e) => {
                            let error_message = format!("更新本地失败 {}: {}", name, e);
                            result.errors.push(error_message.clone());
                            queue.mark_failed(queued_operation, error_message);
                        }
                    }
                }
                SyncOperation::Download(cloud_id) => {
                    let Some(cloud_data) = cloud_data_map.get(&cloud_id) else {
                        result
                            .errors
                            .push(format!("下载失败 {}: 云端数据不存在", cloud_id));
                        continue;
                    };
                    let name = cloud_name_map
                        .get(&cloud_data.id)
                        .cloned()
                        .unwrap_or_else(|| cloud_data.id.clone());
                    match self.download_connection(cloud_data).await {
                        Ok(()) => {
                            result.downloaded += 1;
                            tracing::info!("[下载] 成功: {}", name);
                        }
                        Err(e) => {
                            let error_message = format!("下载失败 {}: {}", name, e);
                            result.errors.push(error_message.clone());
                            queue.mark_failed(queued_operation, error_message);
                        }
                    }
                }
                SyncOperation::DeleteCloud(cloud_id) => {
                    match self.delete_cloud_connection(&cloud_id).await {
                        Ok(()) => {
                            result.deleted += 1;
                            tracing::info!("[删除云端] 成功: {}", cloud_id);
                        }
                        Err(e) => {
                            let error_message = format!("删除云端失败 {}: {}", cloud_id, e);
                            result.errors.push(error_message.clone());
                            queue.mark_failed(queued_operation, error_message);
                        }
                    }
                }
                SyncOperation::DeleteLocal(local_id) => {
                    match self.delete_local_connection(local_id) {
                        Ok(()) => {
                            result.deleted += 1;
                            tracing::info!("[删除本地] 成功: {}", local_id);
                        }
                        Err(e) => {
                            let error_message = format!("删除本地失败 {}: {}", local_id, e);
                            result.errors.push(error_message.clone());
                            queue.mark_failed(queued_operation, error_message);
                        }
                    }
                }
            }
        }

        for resolved in &resolved_conflicts {
            match self.apply_resolved_conflict(resolved).await {
                Ok(()) => {
                    tracing::info!("[冲突解决] 成功应用");
                }
                Err(e) => {
                    result.errors.push(format!("应用冲突解决失败: {}", e));
                }
            }
        }

        self.store_operation_queue(CONNECTION_QUEUE_KEY, queue)?;

        Ok(result)
    }

    /// 解密云端数据建立 cloud_id → name 映射
    fn build_cloud_name_map(&self, cloud_data_list: &[CloudSyncData]) -> HashMap<String, String> {
        let mut map = HashMap::new();
        let service = match self.crypto_service.read() {
            Ok(s) => s,
            Err(_) => return map,
        };

        for data in cloud_data_list {
            if let Ok(conn) = service.decrypt_sync_data_connection(data) {
                map.insert(data.id.clone(), conn.name);
            }
        }
        map
    }

    fn get_local_connections(&self) -> Result<Vec<StoredConnection>, SyncError> {
        let repo = self
            .storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| SyncError::StorageError("ConnectionRepository not found".to_string()))?;

        repo.list()
            .map_err(|e| SyncError::StorageError(e.to_string()))
    }

    fn get_local_decrypt_failures(&self) -> Result<Vec<(i64, String)>, SyncError> {
        let repo = self
            .storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| SyncError::StorageError("ConnectionRepository not found".to_string()))?;

        repo.list_sync_decrypt_failures()
            .map_err(|e| SyncError::StorageError(e.to_string()))
    }

    async fn process_pending_deletions(&self) -> Vec<String> {
        let mut deleted = Vec::new();

        let pending_repo = match self.storage.get::<PendingCloudDeletionRepository>() {
            Some(repo) => repo,
            None => {
                tracing::warn!("[同步] PendingCloudDeletionRepository not found");
                return deleted;
            }
        };

        let pending_list = match pending_repo.list_connections() {
            Ok(list) => list,
            Err(e) => {
                tracing::error!("[同步] 获取待删除列表失败: {}", e);
                return deleted;
            }
        };

        for pending in pending_list {
            tracing::info!("[同步] 处理待删除云端连接: {}", pending.cloud_id);
            match self.cloud_client.delete_sync_data(&pending.cloud_id).await {
                Ok(_) => {
                    tracing::info!("[同步] 云端连接删除成功: {}", pending.cloud_id);
                    if let Err(e) = pending_repo.remove(&pending.cloud_id) {
                        tracing::error!("[同步] 移除待删除记录失败: {}", e);
                    }
                    deleted.push(pending.cloud_id);
                }
                Err(e) => {
                    let error_str = e.to_string();
                    if error_str.contains("404") || error_str.contains("not found") {
                        tracing::info!(
                            "[同步] 云端连接已不存在，移除待删除记录: {}",
                            pending.cloud_id
                        );
                        if let Err(e) = pending_repo.remove(&pending.cloud_id) {
                            tracing::error!("[同步] 移除待删除记录失败: {}", e);
                        }
                        deleted.push(pending.cloud_id);
                    } else {
                        tracing::warn!(
                            "[同步] 删除云端连接失败: {} - {}（保留在待删除列表）",
                            pending.cloud_id,
                            e
                        );
                    }
                }
            }
        }

        deleted
    }

    fn process_cloud_soft_deleted_sync_data(
        &self,
        cloud_data_list: &[CloudSyncData],
        local_connections: &[StoredConnection],
    ) -> Result<Vec<String>, SyncError> {
        let repo = self
            .storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| SyncError::StorageError("ConnectionRepository not found".to_string()))?;

        let mut deleted_cloud_ids = Vec::new();

        for cloud_data in cloud_data_list {
            if cloud_data.deleted_at.is_some() {
                if let Some(local_conn) = local_connections
                    .iter()
                    .find(|c| c.cloud_id.as_ref() == Some(&cloud_data.id))
                {
                    if let Some(local_id) = local_conn.id {
                        tracing::info!(
                            "[软删除] 云端数据 {} 已被删除，删除对应的本地连接 {}",
                            cloud_data.id,
                            local_id
                        );
                        if let Err(e) = repo.delete(local_id) {
                            tracing::error!("[软删除] 删除本地连接失败: {} - {}", local_id, e);
                        } else {
                            deleted_cloud_ids.push(cloud_data.id.clone());
                        }
                    }
                }
            }
        }

        Ok(deleted_cloud_ids)
    }

    fn filter_soft_deleted_local_connections(
        local_connections: Vec<StoredConnection>,
        soft_deleted_cloud_ids: &[String],
    ) -> Vec<StoredConnection> {
        if soft_deleted_cloud_ids.is_empty() {
            return local_connections;
        }
        let soft_deleted: HashSet<&str> =
            soft_deleted_cloud_ids.iter().map(String::as_str).collect();
        local_connections
            .into_iter()
            .filter(|connection| {
                connection
                    .cloud_id
                    .as_deref()
                    .is_none_or(|cloud_id| !soft_deleted.contains(cloud_id))
            })
            .collect()
    }

    fn calculate_sync_plan(
        &self,
        local_connections: &[StoredConnection],
        cloud_data_list: &[CloudSyncData],
        cloud_name_map: &HashMap<String, String>,
        readable_team_ids: &HashSet<String>,
    ) -> Result<SyncPlan, SyncError> {
        let mut plan = SyncPlan::default();

        let cloud_map: HashMap<&str, &CloudSyncData> =
            cloud_data_list.iter().map(|d| (d.id.as_str(), d)).collect();

        let local_cloud_ids: HashSet<String> = local_connections
            .iter()
            .filter_map(|c| c.cloud_id.clone())
            .collect();

        let local_unlinked_by_name: HashMap<&str, &StoredConnection> = local_connections
            .iter()
            .filter(|c| c.cloud_id.is_none() && c.sync_enabled)
            .map(|c| (c.name.as_str(), c))
            .collect();

        for local_conn in local_connections {
            if !local_conn.sync_enabled {
                continue;
            }

            match &local_conn.cloud_id {
                Some(cloud_id) => {
                    if let Some(cloud_data) = cloud_map.get(cloud_id.as_str()) {
                        let last_synced = local_conn.last_synced_at.unwrap_or(0);
                        let cloud_updated = cloud_data.updated_at / 1000;

                        let local_changed =
                            Self::connection_has_local_changes(local_conn, cloud_data);
                        let cloud_changed = cloud_updated > last_synced;

                        let cloud_name = cloud_name_map
                            .get(&cloud_data.id)
                            .cloned()
                            .unwrap_or_else(|| cloud_data.id.clone());

                        match (local_changed, cloud_changed) {
                            (true, true) => {
                                plan.conflicts.push(SyncConflict {
                                    local: local_conn.clone(),
                                    cloud: (*cloud_data).clone(),
                                    cloud_name,
                                    conflict_type:
                                        crate::cloud_sync::models::ConflictType::BothModified,
                                });
                            }
                            (true, false) => {
                                plan.to_update_cloud
                                    .push((local_conn.clone(), (*cloud_data).clone()));
                            }
                            (false, true) => {
                                plan.to_update_local
                                    .push(((*cloud_data).clone(), local_conn.clone()));
                            }
                            (false, false) => {}
                        }
                    } else if Self::cloud_absence_is_authoritative(local_conn, readable_team_ids) {
                        plan.conflicts.push(
                            crate::cloud_sync::conflict::ConflictResolver::detect_local_modified_cloud_deleted(
                                local_conn,
                                cloud_id,
                            ),
                        );
                    } else {
                        tracing::info!(
                            "[同步计划] 团队 {} 当前不可读，保留本地连接 {}",
                            local_conn.team_id.as_deref().unwrap_or_default(),
                            local_conn.name
                        );
                    }
                }
                None => {
                    let has_cloud_match =
                        cloud_name_map.values().any(|name| name == &local_conn.name);
                    if !has_cloud_match {
                        plan.to_upload.push(local_conn.clone());
                    }
                }
            }
        }

        let pending_cloud_ids = self.get_pending_deletion_cloud_ids();
        for cloud_data in cloud_data_list {
            if !local_cloud_ids.contains(&cloud_data.id) {
                if pending_cloud_ids.contains(&cloud_data.id) {
                    let name = cloud_name_map
                        .get(&cloud_data.id)
                        .cloned()
                        .unwrap_or_else(|| cloud_data.id.clone());
                    tracing::info!("[同步计划] 跳过待删除的云端连接: {}", name);
                    continue;
                }

                let cloud_name = cloud_name_map
                    .get(&cloud_data.id)
                    .cloned()
                    .unwrap_or_else(|| cloud_data.id.clone());

                if let Some(local_conn) = local_unlinked_by_name.get(cloud_name.as_str()) {
                    tracing::info!(
                        "[同步计划] 按名称匹配连接: {} (云端 {} -> 本地 {:?})",
                        cloud_name,
                        cloud_data.id,
                        local_conn.id
                    );
                    plan.to_update_local
                        .push((cloud_data.clone(), (*local_conn).clone()));
                } else {
                    plan.to_download.push(cloud_data.clone());
                }
            }
        }

        Ok(plan)
    }

    fn readable_team_ids(&self) -> HashSet<String> {
        self.get_cached_teams()
            .into_iter()
            .filter(|team| self.is_team_unlocked(&team.id))
            .map(|team| team.id)
            .collect()
    }

    fn cloud_absence_is_authoritative(
        local_conn: &StoredConnection,
        readable_team_ids: &HashSet<String>,
    ) -> bool {
        local_conn
            .team_id
            .as_ref()
            .is_none_or(|team_id| readable_team_ids.contains(team_id))
    }

    fn resolve_conflicts(
        &self,
        conflicts: &[SyncConflict],
    ) -> Result<Vec<ResolvedConflictAction>, SyncError> {
        let mut resolved = Vec::new();

        for conflict in conflicts {
            match self.conflict_strategy {
                ConflictResolution::UseCloud => {
                    resolved.push(ResolvedConflictAction {
                        conflict: conflict.clone(),
                        resolution: ConflictResolution::UseCloud,
                        result_connection: None,
                    });
                }
                ConflictResolution::UseLocal => {
                    resolved.push(ResolvedConflictAction {
                        conflict: conflict.clone(),
                        resolution: ConflictResolution::UseLocal,
                        result_connection: Some(conflict.local.clone()),
                    });
                }
                ConflictResolution::KeepBoth => {
                    let mut copy = conflict.local.clone();
                    copy.id = None;
                    copy.cloud_id = None;
                    copy.last_synced_at = None;
                    let timestamp = Self::current_timestamp();
                    copy.name = format!("{} (冲突副本 {})", copy.name, timestamp);

                    resolved.push(ResolvedConflictAction {
                        conflict: conflict.clone(),
                        resolution: ConflictResolution::KeepBoth,
                        result_connection: Some(copy),
                    });
                }
            }
        }

        Ok(resolved)
    }

    async fn upload_connection(&self, conn: &StoredConnection) -> Result<CloudSyncData, SyncError> {
        let teams = self.get_cached_teams();
        let workspace_cloud_id = self.workspace_cloud_id_for_local_id(conn.workspace_id)?;
        let cloud_data = {
            let service = self
                .crypto_service
                .read()
                .map_err(|_| SyncError::StorageError("同步服务锁获取失败".to_string()))?;
            service.prepare_sync_data_upload_with_workspace_cloud_id(
                conn,
                conn.team_id.as_deref(),
                &teams,
                workspace_cloud_id,
            )?
        };
        let created = self
            .cloud_client
            .create_sync_data(&cloud_data)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        Ok(created)
    }

    async fn update_cloud_connection(
        &self,
        local_conn: &StoredConnection,
        cloud_data: &CloudSyncData,
    ) -> Result<CloudSyncData, SyncError> {
        let teams = self.get_cached_teams();
        let workspace_cloud_id = self.workspace_cloud_id_for_local_id(local_conn.workspace_id)?;
        let updated_data = {
            let service = self
                .crypto_service
                .read()
                .map_err(|_| SyncError::StorageError("同步服务锁获取失败".to_string()))?;
            let mut data = service.prepare_sync_data_upload_with_workspace_cloud_id(
                local_conn,
                local_conn.team_id.as_deref(),
                &teams,
                workspace_cloud_id,
            )?;
            data.id = cloud_data.id.clone();
            data.version = cloud_data.version;
            data
        };

        let updated = self
            .cloud_client
            .update_sync_data(&updated_data)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        Ok(updated)
    }

    async fn download_connection(&self, cloud_data: &CloudSyncData) -> Result<(), SyncError> {
        let service = self
            .crypto_service
            .read()
            .map_err(|_| SyncError::StorageError("同步服务锁获取失败".to_string()))?;

        let (mut local_conn, workspace_cloud_id) =
            service.decrypt_sync_data_connection_with_workspace_cloud_id(cloud_data)?;
        drop(service);
        local_conn.id = None;
        local_conn.cloud_id = Some(cloud_data.id.clone());
        local_conn.workspace_id =
            self.local_workspace_id_for_cloud_id(workspace_cloud_id.as_deref())?;
        local_conn.last_synced_at = Some(Self::current_timestamp());

        let repo = self
            .storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| SyncError::StorageError("ConnectionRepository not found".to_string()))?;

        Self::persist_downloaded_connection(repo.as_ref(), &mut local_conn)?;

        Ok(())
    }

    fn persist_downloaded_connection(
        repo: &ConnectionRepository,
        local_conn: &mut StoredConnection,
    ) -> Result<(), SyncError> {
        repo.upsert_cloud_connection(local_conn)
            .map_err(|error| SyncError::StorageError(error.to_string()))
    }

    async fn update_local_connection(
        &self,
        cloud_data: &CloudSyncData,
        local_conn: &StoredConnection,
    ) -> Result<(), SyncError> {
        let service = self
            .crypto_service
            .read()
            .map_err(|_| SyncError::StorageError("同步服务锁获取失败".to_string()))?;

        let (mut updated, workspace_cloud_id) =
            service.decrypt_sync_data_connection_with_workspace_cloud_id(cloud_data)?;
        drop(service);
        updated.id = local_conn.id;
        updated.cloud_id = Some(cloud_data.id.clone());
        updated.workspace_id =
            self.local_workspace_id_for_cloud_id(workspace_cloud_id.as_deref())?;
        updated.last_synced_at = Some(Self::current_timestamp());

        let repo = self
            .storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| SyncError::StorageError("ConnectionRepository not found".to_string()))?;

        repo.update(&updated)
            .map_err(|e| SyncError::StorageError(e.to_string()))?;

        Ok(())
    }

    fn update_sync_status(
        &self,
        local_id: i64,
        cloud_id: Option<String>,
        last_synced_at: Option<i64>,
        error: Option<String>,
    ) -> Result<(), SyncError> {
        let repo = self
            .storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| SyncError::StorageError("ConnectionRepository not found".to_string()))?;

        let synced_at = last_synced_at.unwrap_or_else(Self::current_timestamp);
        repo.update_sync_status(local_id, cloud_id, Some(synced_at))
            .map_err(|e| SyncError::StorageError(e.to_string()))?;

        if let Some(err) = error {
            tracing::warn!("[同步状态] 连接 {} 同步出错: {}", local_id, err);
        }

        Ok(())
    }

    fn update_sync_status_after_cloud_write(
        &self,
        snapshot: &StoredConnection,
        cloud_data: &CloudSyncData,
        error: Option<String>,
    ) -> Result<(), SyncError> {
        let repo = self
            .storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| SyncError::StorageError("ConnectionRepository not found".to_string()))?;
        let local_id = snapshot
            .id
            .ok_or_else(|| SyncError::StorageError("连接缺少本地 ID".to_string()))?;
        let current = repo
            .get(local_id)
            .map_err(|e| SyncError::StorageError(e.to_string()))?
            .unwrap_or_else(|| snapshot.clone());
        let synced_at = Self::connection_synced_at_after_cloud_write(snapshot, cloud_data);
        let pending_updated_at =
            Self::pending_updated_at_after_cloud_write(snapshot, &current, synced_at);

        if let Some(updated_at) = pending_updated_at {
            repo.update_sync_status_with_updated_at(
                local_id,
                Some(cloud_data.id.clone()),
                Some(synced_at),
                updated_at,
            )
        } else {
            repo.update_sync_status(local_id, Some(cloud_data.id.clone()), Some(synced_at))
        }
        .map_err(|e| SyncError::StorageError(e.to_string()))?;

        if let Some(err) = error {
            tracing::warn!("[同步状态] 连接 {} 同步出错: {}", local_id, err);
        }

        Ok(())
    }

    pub(crate) fn connection_synced_at_after_cloud_write(
        local_conn: &StoredConnection,
        cloud_data: &CloudSyncData,
    ) -> i64 {
        let local_updated = local_conn.updated_at.unwrap_or(0);
        let cloud_updated = cloud_data.updated_at / 1000;
        Self::current_timestamp()
            .max(local_updated)
            .max(cloud_updated)
    }

    pub(crate) fn pending_updated_at_after_cloud_write(
        snapshot: &StoredConnection,
        current: &StoredConnection,
        synced_at: i64,
    ) -> Option<i64> {
        if !Self::connection_content_changed(snapshot, current) {
            return None;
        }

        let current_updated_at = current.updated_at.unwrap_or(0);
        if current_updated_at > synced_at {
            None
        } else {
            Some(synced_at + 1)
        }
    }

    fn connection_content_changed(snapshot: &StoredConnection, current: &StoredConnection) -> bool {
        snapshot.name != current.name
            || snapshot.connection_type != current.connection_type
            || snapshot.params != current.params
            || snapshot.workspace_id != current.workspace_id
            || snapshot.selected_databases != current.selected_databases
            || snapshot.remark != current.remark
            || snapshot.sync_enabled != current.sync_enabled
            || snapshot.team_id != current.team_id
            || snapshot.owner_id != current.owner_id
    }

    fn connection_has_local_changes(
        local_conn: &StoredConnection,
        cloud_data: &CloudSyncData,
    ) -> bool {
        local_conn.updated_at.unwrap_or(0) > local_conn.last_synced_at.unwrap_or(0)
            || local_conn.team_id != cloud_data.team_id
    }

    pub(crate) fn connection_cloud_write_target(
        stale: &CloudSyncData,
        latest: Option<&CloudSyncData>,
    ) -> Option<CloudSyncData> {
        if let Some(latest) = latest {
            if latest.deleted_at.is_none() {
                return Some(latest.clone());
            }
            return None;
        }
        if stale.deleted_at.is_none() && !stale.encrypted_data.is_empty() {
            Some(stale.clone())
        } else {
            None
        }
    }

    pub(crate) async fn apply_use_local_connection_conflict(
        &self,
        conflict: &SyncConflict,
    ) -> Result<(), SyncError> {
        let local_id = conflict
            .local
            .id
            .ok_or_else(|| SyncError::StorageError("冲突本地连接缺少 ID".to_string()))?;
        let latest = self
            .fetch_latest_connection_cloud_data(&conflict.cloud.id)
            .await?;

        if let Some(target) = Self::connection_cloud_write_target(&conflict.cloud, latest.as_ref())
        {
            let updated_cloud_data = self
                .update_cloud_connection(&conflict.local, &target)
                .await?;
            let synced_at =
                Self::connection_synced_at_after_cloud_write(&conflict.local, &updated_cloud_data);
            return self.update_sync_status(
                local_id,
                Some(updated_cloud_data.id),
                Some(synced_at),
                None,
            );
        }

        let created_cloud_data = self.upload_connection(&conflict.local).await?;
        let synced_at =
            Self::connection_synced_at_after_cloud_write(&conflict.local, &created_cloud_data);
        self.update_sync_status(local_id, Some(created_cloud_data.id), Some(synced_at), None)
    }

    pub(crate) fn apply_cloud_deleted_connection_conflict(
        &self,
        conflict: &SyncConflict,
        keep_local_copy: bool,
    ) -> Result<(), SyncError> {
        if keep_local_copy {
            let repo = self.storage.get::<ConnectionRepository>().ok_or_else(|| {
                SyncError::StorageError("ConnectionRepository not found".to_string())
            })?;
            let mut copy = conflict.local.clone();
            copy.id = None;
            copy.cloud_id = None;
            copy.last_synced_at = None;
            copy.name = format!("{} (冲突副本 {})", copy.name, Self::current_timestamp());
            repo.insert(&mut copy)
                .map_err(|e| SyncError::StorageError(e.to_string()))?;
        }
        let local_id = conflict
            .local
            .id
            .ok_or_else(|| SyncError::StorageError("冲突本地连接缺少 ID".to_string()))?;
        self.delete_local_connection(local_id)
    }

    pub(crate) async fn fetch_latest_connection_cloud_data(
        &self,
        cloud_id: &str,
    ) -> Result<Option<CloudSyncData>, SyncError> {
        if cloud_id.is_empty() {
            return Ok(None);
        }
        let cloud_data = self
            .cloud_client
            .list_sync_data(Some(data_type::CONNECTION), None, None)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;
        Ok(cloud_data.into_iter().find(|data| data.id == cloud_id))
    }

    async fn apply_resolved_conflict(
        &self,
        resolved: &ResolvedConflictAction,
    ) -> Result<(), SyncError> {
        match resolved.resolution {
            ConflictResolution::UseCloud => {
                if resolved.conflict.conflict_type
                    == crate::cloud_sync::models::ConflictType::LocalModifiedCloudDeleted
                {
                    return self.apply_cloud_deleted_connection_conflict(&resolved.conflict, false);
                }
                self.update_local_connection(&resolved.conflict.cloud, &resolved.conflict.local)
                    .await
            }
            ConflictResolution::UseLocal => {
                self.apply_use_local_connection_conflict(&resolved.conflict)
                    .await
            }
            ConflictResolution::KeepBoth => {
                if resolved.conflict.conflict_type
                    == crate::cloud_sync::models::ConflictType::LocalModifiedCloudDeleted
                {
                    return self.apply_cloud_deleted_connection_conflict(&resolved.conflict, true);
                }
                if let Some(copy) = &resolved.result_connection {
                    let repo = self.storage.get::<ConnectionRepository>().ok_or_else(|| {
                        SyncError::StorageError("ConnectionRepository not found".to_string())
                    })?;

                    let mut new_conn = copy.clone();
                    repo.insert(&mut new_conn)
                        .map_err(|e| SyncError::StorageError(e.to_string()))?;
                }
                self.update_local_connection(&resolved.conflict.cloud, &resolved.conflict.local)
                    .await
            }
        }
    }

    pub async fn delete_cloud_connection(&self, cloud_id: &str) -> Result<(), SyncError> {
        self.cloud_client
            .delete_sync_data(cloud_id)
            .await
            .map_err(|e| SyncError::NetworkError(e.to_string()))?;

        Ok(())
    }

    pub fn delete_local_connection(&self, local_id: i64) -> Result<(), SyncError> {
        let repo = self
            .storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| SyncError::StorageError("ConnectionRepository not found".to_string()))?;

        repo.delete(local_id)
            .map_err(|e| SyncError::StorageError(e.to_string()))?;

        Ok(())
    }

    pub async fn delete_connection(&self, local_id: i64) -> Result<(), SyncError> {
        let repo = self
            .storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| SyncError::StorageError("ConnectionRepository not found".to_string()))?;

        let connections = repo
            .list()
            .map_err(|e| SyncError::StorageError(e.to_string()))?;

        let conn = connections
            .iter()
            .find(|c| c.id == Some(local_id))
            .ok_or_else(|| SyncError::StorageError(format!("连接 {} 不存在", local_id)))?;

        if let Some(cloud_id) = &conn.cloud_id {
            if let Err(e) = self.delete_cloud_connection(cloud_id).await {
                tracing::warn!("[删除] 云端删除失败: {} - {}（继续删除本地）", cloud_id, e);
            }
        }

        self.delete_local_connection(local_id)
    }

    fn get_pending_deletion_cloud_ids(&self) -> HashSet<String> {
        let pending_repo = match self.storage.get::<PendingCloudDeletionRepository>() {
            Some(repo) => repo,
            None => return HashSet::new(),
        };

        match pending_repo.list_connections() {
            Ok(list) => list.into_iter().map(|p| p.cloud_id).collect(),
            Err(_) => HashSet::new(),
        }
    }

    pub(crate) fn workspace_cloud_id_for_local_id(
        &self,
        workspace_id: Option<i64>,
    ) -> Result<Option<String>, SyncError> {
        let Some(workspace_id) = workspace_id else {
            return Ok(None);
        };
        let repo = self
            .storage
            .get::<WorkspaceRepository>()
            .ok_or_else(|| SyncError::StorageError("WorkspaceRepository not found".to_string()))?;

        let workspace = repo
            .get(workspace_id)
            .map_err(|e| SyncError::StorageError(e.to_string()))?;
        Ok(workspace.and_then(|workspace| workspace.cloud_id))
    }

    pub(crate) fn local_workspace_id_for_cloud_id(
        &self,
        workspace_cloud_id: Option<&str>,
    ) -> Result<Option<i64>, SyncError> {
        let Some(workspace_cloud_id) = workspace_cloud_id else {
            return Ok(None);
        };
        let repo = self
            .storage
            .get::<WorkspaceRepository>()
            .ok_or_else(|| SyncError::StorageError("WorkspaceRepository not found".to_string()))?;

        let workspaces = repo
            .list()
            .map_err(|e| SyncError::StorageError(e.to_string()))?;
        Ok(workspaces
            .into_iter()
            .find(|workspace| workspace.cloud_id.as_deref() == Some(workspace_cloud_id))
            .and_then(|workspace| workspace.id))
    }
}

/// 已解决的冲突操作
pub struct ResolvedConflictAction {
    pub conflict: SyncConflict,
    pub resolution: ConflictResolution,
    pub result_connection: Option<StoredConnection>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::connection::SqliteConnection;
    use crate::storage::migration::run_migrations;
    use crate::storage::{ConnectionRepository, ConnectionType};

    fn stored_connection(updated_at: i64) -> StoredConnection {
        StoredConnection {
            id: Some(1),
            name: "local".to_string(),
            connection_type: ConnectionType::Database,
            params: "{}".to_string(),
            workspace_id: None,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: Some("cloud-1".to_string()),
            last_synced_at: Some(updated_at - 10),
            last_used_at: None,
            sort_order: None,
            created_at: Some(updated_at - 20),
            updated_at: Some(updated_at),
            team_id: None,
            owner_id: None,
        }
    }

    fn cloud_sync_data(updated_at: i64) -> CloudSyncData {
        CloudSyncData {
            id: "cloud-1".to_string(),
            owner_id: "user-1".to_string(),
            team_id: None,
            data_type: data_type::CONNECTION.to_string(),
            encrypted_data: String::new(),
            key_version: 1,
            checksum: String::new(),
            version: 1,
            updated_at: updated_at * 1000,
            deleted_at: None,
        }
    }

    #[test]
    fn conflict_sync_timestamp_covers_local_update_time() {
        let local_updated = SyncEngine::current_timestamp() + 120;
        let cloud_updated = local_updated - 30;
        let synced_at = SyncEngine::connection_synced_at_after_cloud_write(
            &stored_connection(local_updated),
            &cloud_sync_data(cloud_updated),
        );

        assert_eq!(local_updated, synced_at);
    }

    #[test]
    fn conflict_sync_timestamp_covers_cloud_update_time() {
        let cloud_updated = SyncEngine::current_timestamp() + 120;
        let local_updated = cloud_updated - 30;
        let synced_at = SyncEngine::connection_synced_at_after_cloud_write(
            &stored_connection(local_updated),
            &cloud_sync_data(cloud_updated),
        );

        assert_eq!(cloud_updated, synced_at);
    }

    #[test]
    fn use_local_conflict_write_target_prefers_latest_cloud_version() {
        let stale = CloudSyncData {
            version: 2,
            ..cloud_sync_data(100)
        };
        let latest = CloudSyncData {
            version: 7,
            updated_at: 150_000,
            ..cloud_sync_data(150)
        };

        let target = SyncEngine::connection_cloud_write_target(&stale, Some(&latest))
            .expect("latest active cloud data should be selected");

        assert_eq!(7, target.version);
        assert_eq!(150_000, target.updated_at);
    }

    #[test]
    fn cloud_deleted_placeholder_is_not_a_write_target() {
        let placeholder = CloudSyncData {
            id: "deleted-cloud".to_string(),
            encrypted_data: String::new(),
            version: 0,
            ..cloud_sync_data(0)
        };
        let deleted = CloudSyncData {
            deleted_at: Some(200_000),
            ..cloud_sync_data(200)
        };

        let target = SyncEngine::connection_cloud_write_target(&placeholder, Some(&deleted));

        assert!(target.is_none());
    }

    #[test]
    fn soft_deleted_local_connections_are_removed_from_same_pass_snapshot() {
        let mut deleted = stored_connection(100);
        deleted.cloud_id = Some("deleted-cloud".to_string());

        let mut active = stored_connection(100);
        active.id = Some(2);
        active.cloud_id = Some("active-cloud".to_string());

        let filtered = SyncEngine::filter_soft_deleted_local_connections(
            vec![deleted, active],
            &[String::from("deleted-cloud")],
        );

        assert_eq!(1, filtered.len());
        assert_eq!(Some("active-cloud"), filtered[0].cloud_id.as_deref());
    }

    #[test]
    fn cloud_write_marks_mid_sync_local_edit_pending_even_in_same_second() {
        let snapshot = stored_connection(100);
        let mut current = snapshot.clone();
        current.name = "edited".to_string();
        current.updated_at = snapshot.updated_at;
        let synced_at = 120;

        let pending_updated_at =
            SyncEngine::pending_updated_at_after_cloud_write(&snapshot, &current, synced_at);

        assert_eq!(Some(121), pending_updated_at);
    }

    #[test]
    fn cloud_write_does_not_mark_unchanged_connection_pending() {
        let snapshot = stored_connection(100);
        let current = snapshot.clone();

        let pending_updated_at =
            SyncEngine::pending_updated_at_after_cloud_write(&snapshot, &current, 120);

        assert_eq!(None, pending_updated_at);
    }

    #[test]
    fn team_assignment_mismatch_remains_pending_after_false_success() {
        let mut local = stored_connection(100);
        local.updated_at = Some(100);
        local.last_synced_at = Some(120);
        local.team_id = Some("team-1".to_string());
        let cloud = cloud_sync_data(120);

        assert!(SyncEngine::connection_has_local_changes(&local, &cloud));
    }

    #[test]
    fn missing_unreadable_team_record_is_not_treated_as_cloud_deletion() {
        let mut local = stored_connection(100);
        local.team_id = Some("departed-team".to_string());

        assert!(!SyncEngine::cloud_absence_is_authoritative(
            &local,
            &HashSet::new()
        ));
    }

    #[test]
    fn missing_readable_team_record_can_be_treated_as_cloud_deletion() {
        let mut local = stored_connection(100);
        local.team_id = Some("team-1".to_string());
        let readable_team_ids = HashSet::from(["team-1".to_string()]);

        assert!(SyncEngine::cloud_absence_is_authoritative(
            &local,
            &readable_team_ids
        ));
    }

    #[test]
    fn missing_personal_record_can_be_treated_as_cloud_deletion() {
        let local = stored_connection(100);

        assert!(SyncEngine::cloud_absence_is_authoritative(
            &local,
            &HashSet::new()
        ));
    }

    #[test]
    fn repeated_download_updates_existing_cloud_connection() {
        let temp = tempfile::tempdir().expect("temp directory");
        let database =
            SqliteConnection::open(temp.path().join("connections.db")).expect("database opens");
        database
            .with_connection(|connection| {
                run_migrations(connection)?;
                Ok(())
            })
            .expect("migrations run");
        let repo = ConnectionRepository::new(database);
        let mut first = stored_connection(100);
        first.id = None;
        first.name = "first".to_string();
        repo.insert(&mut first).expect("first download inserts");
        let mut repeated = stored_connection(120);
        repeated.id = None;
        repeated.name = "updated".to_string();

        SyncEngine::persist_downloaded_connection(&repo, &mut repeated)
            .expect("repeated download persists");

        let connections = repo.list().expect("connections list");
        assert_eq!(1, connections.len());
        assert_eq!("updated", connections[0].name);
        assert_eq!(first.id, repeated.id);
    }
}
