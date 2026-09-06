use super::*;

impl HomePage {
    pub(super) fn handle_sync_click(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !AppSettings::global(cx).sync_enabled {
            self.show_sync_disabled_dialog(window, cx);
        } else if sync_route(cx) == HomeSyncRoute::OnetCloud
            && !is_feature_enabled(Feature::CloudSync, cx)
        {
            show_upgrade_dialog(window, cx);
        } else {
            self.trigger_sync(cx);
        }
    }

    fn show_sync_disabled_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view = view.clone();
            dialog
                .title(t!("Home.sync_disabled_title").to_string())
                .child(t!("Home.sync_disabled_message").to_string())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Home.open_sync_settings").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    view.update(cx, |home, cx| {
                        home.add_sync_settings_tab(window, cx);
                    });
                    true
                })
        });
    }

    pub(super) fn trigger_sync(&mut self, cx: &mut Context<Self>) {
        if !AppSettings::global(cx).sync_enabled {
            tracing::debug!("同步总开关已关闭，跳过同步请求");
            return;
        }

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
                        crate::personal_sync_status::note_sync_completed();
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
}
