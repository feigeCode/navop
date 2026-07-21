use super::*;

impl HomePage {
    pub(super) fn load_workspaces(&mut self, cx: &mut Context<Self>) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let load_task = cx.background_spawn(async move {
            (|| {
                let repo = storage
                    .get::<WorkspaceRepository>()
                    .ok_or_else(|| anyhow::anyhow!("WorkspaceRepository not found"))?;
                repo.list()
            })()
        });

        cx.spawn(async move |this, cx: &mut AsyncApp| match load_task.await {
            Ok(workspaces) => {
                _ = this.update(cx, |this, cx| {
                    this.workspaces = workspaces;
                    cx.notify();
                });
            }
            Err(e) => {
                tracing::error!("Task join error: {}", e);
            }
        })
        .detach();
    }

    pub(super) fn load_connections(&mut self, cx: &mut Context<Self>) {
        if self.saved_connections_locked() {
            tracing::warn!("主密钥未解锁，暂缓加载本地连接，避免将加密密码解密为空");
            self.connections.clear();
            cx.notify();
            return;
        }

        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let load_task = cx.background_spawn(async move {
            (|| {
                let repo = storage
                    .get::<ConnectionRepository>()
                    .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;
                Ok::<_, anyhow::Error>((repo.list()?, IpcDriverRegistry::load_default()))
            })()
        });

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = load_task.await;
            match result {
                Ok((connections, external_driver_registry)) => {
                    _ = this.update(cx, |this, cx| {
                        this.connections = connections;
                        this.external_driver_registry = external_driver_registry;
                        cx.notify();
                    });
                }
                Err(e) => {
                    tracing::error!("Task join error: {}", e);
                }
            }
        })
        .detach();
    }

    pub(super) fn load_team_options(&mut self, cx: &mut Context<Self>) {
        let Some(user) = self.current_user.as_ref() else {
            self.team_options.clear();
            cx.notify();
            return;
        };
        let requested_user_id = user.id.clone();
        let scope = CloudAccountScope::new(
            self.auth_service.cloud_client().environment_id(),
            requested_user_id.clone(),
        );
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let load_task = cx.background_spawn(async move {
            get_cached_team_display_options_for_scope(&storage, &scope)
        });

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let teams = load_task.await;
            _ = this.update(cx, |this, cx| {
                if this.current_user.as_ref().map(|user| user.id.as_str())
                    != Some(requested_user_id.as_str())
                {
                    return;
                }
                this.team_options = teams;
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn reorder_workspace_by_id(
        &mut self,
        source_id: i64,
        target_id: i64,
        cx: &mut Context<Self>,
    ) {
        if source_id == target_id {
            return;
        }
        let from = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == Some(source_id));
        let to = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == Some(target_id));
        if let (Some(from), Some(to)) = (from, to) {
            self.reorder_workspaces(from, to, cx);
        }
    }

    pub(super) fn reorder_workspaces(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from >= self.workspaces.len() || to >= self.workspaces.len() || from == to {
            return;
        }
        let moved_workspace_id = self.workspaces[from].id;
        let workspace = self.workspaces.remove(from);
        self.workspaces.insert(to, workspace);

        let orders: Vec<(i64, i32)> = self
            .workspaces
            .iter()
            .enumerate()
            .filter_map(|(index, workspace)| workspace.id.map(|id| (id, index as i32)))
            .collect();
        for (index, workspace) in self.workspaces.iter_mut().enumerate() {
            workspace.sort_order = Some(index as i32);
        }

        let storage = cx.global::<GlobalStorageState>().storage.clone();
        cx.spawn(async move |this, cx| {
            let result = storage
                .get::<WorkspaceRepository>()
                .ok_or_else(|| anyhow::anyhow!("WorkspaceRepository not found"))
                .and_then(|repo| repo.update_sort_orders(&orders));

            _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        if let Some(workspace_id) = moved_workspace_id {
                            emit_connection_event(
                                ConnectionDataEvent::WorkspaceUpdated { workspace_id },
                                cx,
                            );
                        }
                        if should_auto_onet_cloud_sync(cx, this.current_user.is_some()) {
                            tracing::info!("工作区排序变化，自动触发云同步");
                            this.trigger_sync(cx);
                        }
                    }
                    Err(error) => {
                        tracing::error!("更新工作区排序失败: {error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn refresh_local_home_data(&mut self, cx: &mut Context<Self>) {
        self.load_workspaces(cx);
        self.load_connections(cx);
    }
}
