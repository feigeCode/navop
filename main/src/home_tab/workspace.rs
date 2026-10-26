use super::data::sort_workspaces;
use super::*;

impl HomePage {
    pub(crate) fn set_workspace_sidebar_collapsed(
        &mut self,
        workspace_id: i64,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let result = storage
            .get::<WorkspaceRepository>()
            .ok_or_else(|| anyhow::anyhow!("WorkspaceRepository not found"))
            .and_then(|repo| repo.update_sidebar_collapsed(workspace_id, collapsed));

        match result {
            Ok(()) => {
                if let Some(workspace) = self
                    .workspaces
                    .iter_mut()
                    .find(|workspace| workspace.id == Some(workspace_id))
                {
                    workspace.sidebar_collapsed = collapsed;
                    cx.notify();
                }
            }
            Err(error) => tracing::error!("更新工作区侧栏折叠状态失败: {error}"),
        }
    }

    pub(crate) fn set_all_workspaces_sidebar_collapsed(
        &mut self,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let result = storage
            .get::<WorkspaceRepository>()
            .ok_or_else(|| anyhow::anyhow!("WorkspaceRepository not found"))
            .and_then(|repo| repo.set_all_sidebar_collapsed(collapsed));

        match result {
            Ok(()) => {
                self.workspaces
                    .iter_mut()
                    .for_each(|workspace| workspace.sidebar_collapsed = collapsed);
                cx.notify();
            }
            Err(error) => tracing::error!("批量更新工作区侧栏折叠状态失败: {error}"),
        }
    }

    pub(crate) fn handle_save_workspace(
        &mut self,
        workspace_id: Option<i64>,
        parent_id: Option<i64>,
        name: String,
        sort_order: Option<i32>,
        cx: &mut Context<Self>,
    ) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let editing_id = workspace_id;

        let mut workspace = if let Some(id) = editing_id {
            // 编辑模式：从现有工作区更新
            let mut ws = self
                .workspaces
                .iter()
                .find(|w| w.id == Some(id))
                .cloned()
                .unwrap_or_else(|| Workspace::new(name.clone()));
            ws.name = name;
            ws.parent_id = parent_id.or(ws.parent_id);
            ws.sort_order = sort_order.or(ws.sort_order);
            ws
        } else {
            // 新建模式
            let mut workspace = Workspace::new(name);
            workspace.parent_id = parent_id;
            workspace.sort_order = sort_order;
            workspace
        };

        let result: anyhow::Result<Workspace> = (|| {
            let repo = storage
                .get::<WorkspaceRepository>()
                .ok_or_else(|| anyhow::anyhow!("WorkspaceRepository not found"))?;

            if editing_id.is_some() {
                repo.update(&mut workspace)?;
            } else {
                repo.insert(&mut workspace)?;
            }

            Ok(workspace)
        })();

        cx.spawn(async move |this, cx| match result {
            Ok(workspace) => {
                _ = this.update(cx, |this, cx| {
                    let workspace_id = workspace.id.unwrap_or(0);
                    if let Some(editing_id) = editing_id {
                        if let Some(pos) = this
                            .workspaces
                            .iter()
                            .position(|w| w.id == Some(editing_id))
                        {
                            this.workspaces[pos] = workspace;
                        }
                        emit_connection_event(
                            ConnectionDataEvent::WorkspaceUpdated { workspace_id },
                            cx,
                        );
                    } else {
                        this.workspaces.push(workspace);
                        // 与 `load_workspaces` 复用同一套排序，保证展开后的同级顺序
                        // 和重新加载后读到的 DB `sort_order` 顺序一致。
                        sort_workspaces(&mut this.workspaces);
                        emit_connection_event(
                            ConnectionDataEvent::WorkspaceCreated { workspace_id },
                            cx,
                        );
                    }
                    // 兜底触发一次自动同步，避免当前页对自身工作区事件未回流时漏同步。
                    if should_auto_navop_cloud_sync(cx, this.current_user.is_some()) {
                        tracing::info!("本地工作区保存成功，自动触发云同步");
                        this.trigger_sync(cx);
                    }
                    cx.notify();
                });
            }
            Err(e) => {
                tracing::error!("Failed to save workspace: {}", e);
            }
        })
        .detach();
    }

    pub(crate) fn delete_workspace(
        &mut self,
        workspace_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace_name = self
            .workspaces
            .iter()
            .find(|w| w.id == Some(workspace_id))
            .map(|w| w.name.clone())
            .unwrap_or_default();

        let view = cx.entity().clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_clone = view.clone();
            dialog
                .title(t!("Workspace.delete").to_string().into_any_element())
                .child(
                    t!("Workspace.delete_confirm", workspace_name = workspace_name)
                        .to_string()
                        .into_any_element(),
                )
                .confirm()
                .on_ok(move |_, _window, cx: &mut App| {
                    let _ = view_clone.update(cx, |this, cx| {
                        this.handle_delete_workspace(workspace_id, cx);
                    });
                    true
                })
        });
    }

    pub(super) fn handle_delete_workspace(&mut self, workspace_id: i64, cx: &mut Context<Self>) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();

        // 获取工作空间的 cloud_id（用于删除云端数据），并顺带取出删除前的同步基线：
        // 个人同步要靠它判断远端是否在本地删除之后又被别的设备改过
        // （详见 `ConnectionDataEvent::ConnectionDeleted`）。行删掉之后就取不到了。
        let (cloud_id, deleted_last_synced_at) = self
            .workspaces
            .iter()
            .find(|w| w.id == Some(workspace_id))
            .map(|w| (w.cloud_id.clone(), w.last_synced_at))
            .unwrap_or_default();

        // 如果用户已登录且工作空间有 cloud_id，需要同时删除云端
        let cloud_client = if cloud_id.is_some() && self.current_user.is_some() {
            Some(self.auth_service.cloud_client())
        } else {
            None
        };

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            // 1. 先删除云端工作空间（如果有）
            if let (Some(cloud_id), Some(client)) = (&cloud_id, cloud_client) {
                match client.delete_sync_data(cloud_id).await {
                    Ok(_) => {
                        tracing::info!("[删除] 云端工作空间删除成功: {}", cloud_id);
                    }
                    Err(e) => {
                        // 云端删除失败，记录到待删除表，下次同步时重试
                        tracing::warn!(
                            "[删除] 云端工作空间删除失败: {} - {}（记录到待删除列表）",
                            cloud_id,
                            e
                        );
                        if let Some(pending_repo) = storage.get::<PendingCloudDeletionRepository>()
                        {
                            if let Err(e) = pending_repo.add(cloud_id, "workspace") {
                                tracing::error!("[删除] 记录待删除失败: {}", e);
                            }
                        }
                    }
                }
            } else if let Some(cloud_id) = &cloud_id {
                // 用户未登录但工作空间有 cloud_id，也记录到待删除表
                tracing::info!("[删除] 用户离线，记录到待删除列表: {}", cloud_id);
                if let Some(pending_repo) = storage.get::<PendingCloudDeletionRepository>() {
                    if let Err(e) = pending_repo.add(cloud_id, "workspace") {
                        tracing::error!("[删除] 记录待删除失败: {}", e);
                    }
                }
            }

            // 2. 删除本地工作空间
            let result = (|| {
                let repo = storage
                    .get::<WorkspaceRepository>()
                    .ok_or_else(|| anyhow::anyhow!("WorkspaceRepository not found"))?;
                repo.delete(workspace_id)
            })();

            match result {
                Ok(_) => {
                    _ = this.update(cx, |this, cx| {
                        promote_workspace_children_in_memory(
                            &mut this.workspaces,
                            &mut this.connections,
                            workspace_id,
                        );
                        this.filtered_workspace_ids.remove(&workspace_id);
                        if let Some(cloud_id) = &cloud_id {
                            // 本地分组没了，遗留的冲突也就解不开了，删除时一并丢掉。
                            crate::personal_sync_runtime::forget_personal_conflict(
                                &storage,
                                one_core::cloud_sync::data_type::WORKSPACE,
                                cloud_id,
                            );
                        }
                        emit_connection_event(
                            ConnectionDataEvent::WorkspaceDeleted {
                                workspace_id,
                                cloud_id: cloud_id.clone(),
                                last_synced_at: deleted_last_synced_at,
                            },
                            cx,
                        );
                        // 兜底触发一次自动同步，避免当前页对自身工作区事件未回流时漏同步。
                        if should_auto_navop_cloud_sync(cx, this.current_user.is_some()) {
                            tracing::info!("本地工作区删除成功，自动触发云同步");
                            this.trigger_sync(cx);
                        }
                        cx.notify();
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to delete workspace: {}", e);
                }
            }
        })
        .detach();
    }
}

/// 把 `workspace_id` 的子分组和直属连接提升到它的父级，并移除该分组。
///
/// 与 `WorkspaceRepository::delete` 的 SQL 迁移保持一致，避免内存里的树出现
/// "连接消失、子分组跳到根级"。根分组被删时提升到的父级为 `None`。
pub(super) fn promote_workspace_children_in_memory(
    workspaces: &mut Vec<Workspace>,
    connections: &mut [StoredConnection],
    workspace_id: i64,
) -> Option<i64> {
    let parent_id = workspaces
        .iter()
        .find(|workspace| workspace.id == Some(workspace_id))
        .and_then(|workspace| workspace.parent_id);
    for workspace in workspaces.iter_mut() {
        if workspace.parent_id == Some(workspace_id) {
            workspace.parent_id = parent_id;
        }
    }
    for connection in connections.iter_mut() {
        if connection.workspace_id == Some(workspace_id) {
            connection.workspace_id = parent_id;
        }
    }
    workspaces.retain(|workspace| workspace.id != Some(workspace_id));
    parent_id
}
