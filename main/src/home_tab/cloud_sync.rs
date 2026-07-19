use super::*;

impl HomePage {
    pub(super) fn trigger_sync(&mut self, cx: &mut Context<Self>) {
        if sync_route(cx) == HomeSyncRoute::Personal {
            crate::personal_sync_runtime::sync_now(cx);
            return;
        }

        // 检查 License
        if !is_feature_enabled(Feature::CloudSync, cx) {
            tracing::debug!("云同步功能需要 Pro 订阅");
            return;
        }

        if self.current_user.is_none() {
            self.cloud_error = Some(t!("Home.cloud_need_login").to_string());
            cx.notify();
            return;
        }

        if self.syncing {
            self.sync_requested = true;
            return;
        }

        self.syncing = true;
        self.sync_requested = false;
        self.cloud_error = None;
        cx.notify();

        let cloud_client = self.auth_service.cloud_client();
        let sync_service = self.cloud_sync_service.clone();
        let storage = cx.global::<GlobalStorageState>().storage.clone();

        if let Some(user) = &self.current_user {
            if let Ok(mut service) = sync_service.write() {
                service.set_logged_in(user.id.clone());
            } else {
                tracing::warn!("同步前设置用户ID失败：无法获取云同步服务写锁");
            }
        }

        // 创建同步引擎
        let engine = SyncEngine::new(cloud_client, sync_service, storage);
        let sync_task = Tokio::spawn(cx, async move { engine.sync().await });

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = match sync_task.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(format!("云同步任务执行失败: {error}")),
            };

            _ = this.update(cx, |this, cx| {
                this.syncing = false;
                let sync_requested = this.sync_requested;
                match result {
                    Ok(stats) => {
                        tracing::info!(
                            "同步完成：上传 {} 个，下载 {} 个，冲突 {} 个",
                            stats.uploaded,
                            stats.downloaded,
                            stats.conflicts.len()
                        );
                        this.cloud_error = None;

                        if !stats.conflicts.is_empty() {
                            tracing::warn!("同步存在 {} 个冲突需要处理", stats.conflicts.len());
                        }
                        this.pending_conflicts = refreshed_pending_conflicts(
                            std::mem::take(&mut this.pending_conflicts),
                            stats.conflicts,
                            &stats.errors,
                        );

                        // 如果有错误，显示第一个错误
                        if !stats.errors.is_empty() {
                            this.cloud_error = Some(stats.errors.join("; "));
                        }

                        // 刷新首页本地数据，确保部分失败时界面仍与已落库数据一致
                        this.refresh_local_home_data(cx);
                        emit_connection_event(ConnectionDataEvent::TeamCacheUpdated, cx);
                    }
                    Err(e) => {
                        tracing::error!("同步失败: {}", e);
                        this.cloud_error = Some(e);
                    }
                }
                if sync_requested && this.pending_conflicts.is_empty() && this.cloud_error.is_none()
                {
                    this.sync_requested = false;
                    this.trigger_sync(cx);
                } else {
                    this.sync_requested = false;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 显示冲突解决对话框
    pub(super) fn show_conflict_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_conflicts.is_empty() {
            return;
        }

        let conflicts = self.pending_conflicts.clone();
        let view = cx.entity().clone();
        let conflict_count = conflicts.len();
        let options = vec![
            crate::sync_conflict_dialog::SyncConflictResolutionOption {
                strategy: ConflictResolution::UseCloud,
                label: SharedString::from(t!("Home.sync_conflict_use_cloud").to_string()),
            },
            crate::sync_conflict_dialog::SyncConflictResolutionOption {
                strategy: ConflictResolution::UseLocal,
                label: SharedString::from(t!("Home.sync_conflict_use_local").to_string()),
            },
            crate::sync_conflict_dialog::SyncConflictResolutionOption {
                strategy: ConflictResolution::KeepBoth,
                label: SharedString::from(t!("Home.sync_conflict_keep_both").to_string()),
            },
        ];
        let items = conflicts
            .iter()
            .map(|conflict| {
                let suggested = match conflict.conflict_type {
                    one_core::cloud_sync::ConflictType::BothModified => {
                        ConflictResolution::KeepBoth
                    }
                    one_core::cloud_sync::ConflictType::LocalDeletedCloudModified => {
                        ConflictResolution::UseCloud
                    }
                    one_core::cloud_sync::ConflictType::LocalModifiedCloudDeleted => {
                        ConflictResolution::UseLocal
                    }
                };
                crate::sync_conflict_dialog::SyncConflictDialogItem {
                    id: conflict.cloud.id.clone(),
                    title: conflict.local.name.clone(),
                    detail: t!(
                        "Home.sync_conflict_type",
                        conflict_type = format!("{}", conflict.conflict_type)
                    )
                    .to_string(),
                    default_strategy: suggested,
                    options: options.clone(),
                }
            })
            .collect();

        crate::sync_conflict_dialog::show_sync_conflict_dialog(
            window,
            cx,
            t!("Home.sync_conflict_dialog_title", count = conflict_count).to_string(),
            t!("Home.sync_conflict_apply").to_string(),
            items,
            move |selected, _window, cx| {
                let selected_strategies = selected.into_iter().collect();
                view.update(cx, |this, cx| {
                    this.resolve_conflicts_individually(selected_strategies, cx);
                });
            },
        );
    }

    /// 使用单独的策略解决每个冲突
    pub(super) fn resolve_conflicts_individually(
        &mut self,
        strategies: std::collections::HashMap<String, ConflictResolution>,
        cx: &mut Context<Self>,
    ) {
        if self.pending_conflicts.is_empty() {
            return;
        }

        tracing::info!("使用单独策略解决 {} 个冲突", self.pending_conflicts.len());

        if self.syncing {
            self.sync_requested = true;
            return;
        }

        let conflicts = self.pending_conflicts.clone();
        let cloud_client = self.auth_service.cloud_client();
        let sync_service = self.cloud_sync_service.clone();

        if let Some(user) = &self.current_user {
            if let Ok(mut service) = sync_service.write() {
                service.set_logged_in(user.id.clone());
            } else {
                tracing::warn!("冲突解决前设置用户ID失败：无法获取云同步服务写锁");
            }
        }

        let storage = cx.global::<GlobalStorageState>().storage.clone();
        self.syncing = true;
        self.sync_requested = false;
        self.cloud_error = None;
        cx.notify();

        // 创建同步引擎
        let engine = SyncEngine::new(cloud_client, sync_service, storage);
        let resolution_task = Tokio::spawn(cx, async move {
            engine
                .apply_conflict_resolutions(conflicts, strategies)
                .await
        });

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = match resolution_task.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(format!("冲突解决任务执行失败: {error}")),
            };

            _ = this.update(cx, |this, cx| {
                this.syncing = false;
                let sync_requested = this.sync_requested;
                match result {
                    Ok(stats) => {
                        if stats.errors.is_empty() {
                            tracing::info!("冲突解决完成");
                            this.pending_conflicts.clear();
                            this.refresh_local_home_data(cx);
                        } else {
                            tracing::error!("冲突解决存在错误: {}", stats.errors.join("; "));
                            this.cloud_error = Some(stats.errors.join("; "));
                        }
                    }
                    Err(e) => {
                        tracing::error!("冲突解决失败: {}", e);
                        this.cloud_error = Some(e);
                    }
                }
                if sync_requested && this.pending_conflicts.is_empty() && this.cloud_error.is_none()
                {
                    this.sync_requested = false;
                    this.trigger_sync(cx);
                } else {
                    this.sync_requested = false;
                }
                cx.notify();
            });
        })
        .detach();
    }
}
