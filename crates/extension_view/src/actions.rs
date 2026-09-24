use std::path::PathBuf;

use gpui::{App, AppContext, AsyncApp, Context, PathPromptOptions, WeakEntity, Window};
use gpui_component::{WindowExt, notification::Notification};
use rust_i18n::t;

use crate::state::{
    MarketplaceLoadState, apply_installed_reload_success, apply_marketplace_load_result,
    marketplace_manifest_url_from_query, should_auto_load_marketplace,
};
use crate::status_message::{format_notification_error, format_status_error};
use crate::{
    ExtensionKind, ExtensionManagerView, ExtensionSummary, MarketplaceEntry,
    MarketplaceInstallOutcome,
};

impl ExtensionManagerView {
    pub(crate) fn refresh_installed(&mut self, cx: &mut Context<Self>) {
        let started = std::time::Instant::now();
        tracing::info!(target: "extension_perf", "refresh_installed: begin");
        match self.host.list_installed() {
            Ok(installed) => {
                tracing::info!(
                    target: "extension_perf",
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    count = installed.len(),
                    "refresh_installed: host scan done (blocking main thread)"
                );
                self.set_installed(installed);
            }
            Err(err) => {
                tracing::warn!(
                    target: "extension_perf",
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "refresh_installed: failed"
                );
                self.status = t!("Extension.read_installed_failed", error = err.to_string())
                    .to_string()
                    .into();
            }
        }
        cx.notify();
    }

    pub(crate) fn load_marketplace(&mut self, cx: &mut Context<Self>) {
        if self.marketplace_load_state.is_loading() {
            return;
        }
        let started = std::time::Instant::now();
        tracing::info!(target: "extension_perf", "marketplace load: begin");
        self.marketplace_load_attempted = true;
        self.marketplace_load_state = MarketplaceLoadState::Loading;
        self.status = t!("Extension.loading_marketplace").to_string().into();
        let http_client = cx.http_client();
        let manifest_url = self.marketplace_manifest_url(cx);
        let entity = cx.entity().downgrade();
        let task = match manifest_url {
            Some(url) => cx.background_spawn(
                self.host
                    .load_marketplace_entries_from_url(http_client, url),
            ),
            None => cx.background_spawn(self.host.load_marketplace_entries(http_client)),
        };
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let fetch_started = std::time::Instant::now();
            let result = task.await;
            tracing::info!(
                target: "extension_perf",
                fetch_ms = fetch_started.elapsed().as_millis() as u64,
                ok = result.is_ok(),
                "marketplace load: background fetch finished"
            );
            finish_marketplace_load(entity, result, cx);
            tracing::info!(
                target: "extension_perf",
                total_ms = started.elapsed().as_millis() as u64,
                "marketplace load: complete (incl. foreground apply)"
            );
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn select_local_tarball(&mut self, cx: &mut Context<Self>) {
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t!("Extension.select_archive").to_string().into()),
        });
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = future.await
                && let Some(path) = paths.into_iter().next()
            {
                install_local_on_active_window(entity, path, cx);
            }
        })
        .detach();
    }

    pub(crate) fn install_marketplace_entry(
        &mut self,
        entry: MarketplaceEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(entry.id.clone());
        self.status = t!("Extension.installing", name = entry.name.clone())
            .to_string()
            .into();
        let http_client = cx.http_client();
        let entity = cx.entity().downgrade();
        // 触发安装的窗口必须在这里捕获：异步回调里 `cx.active_window()` 拿到的
        // 可能是详情弹窗、也可能为 `None`（应用不在前台），结果就会无处投递。
        let window_handle = window.window_handle();
        let task = cx.background_spawn(self.host.review_marketplace_entry(http_client, entry));
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            finish_extension_action(entity, window_handle, task.await, cx);
        })
        .detach();
        window.push_notification(
            Notification::info(t!("Extension.install_started").to_string()).autohide(true),
            cx,
        );
        cx.notify();
    }

    pub(crate) fn uninstall_extension(
        &mut self,
        summary: ExtensionSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }

        let name = summary.name.clone();
        let kind = summary.kind;
        self.busy = Some(format!("uninstall:{name}"));
        self.status = t!("Extension.uninstalling", name = name).to_string().into();

        let host = self.host.clone();
        let refresh_host = host.clone();
        let close_task = crate::shell::close_shell_extension(&name, window, cx);
        let entity = cx.entity().downgrade();
        let window_handle = window.window_handle();
        let gate_id = name.clone();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            if !close_task.await {
                let _ = cx.update(|cx| {
                    crate::shell::finish_shell_extension(&gate_id, cx);
                    let _ = entity.update(cx, |view, cx| {
                        view.busy = None;
                        view.status = t!("Extension.tabs_close_failed").to_string().into();
                        cx.notify();
                    });
                });
                return;
            }
            let uninstall_task = cx.background_spawn(async move { host.uninstall(&summary) });
            let outcome = uninstall_task.await;
            let mut view_alive = false;
            let updated = cx.update_window(window_handle, |_, window, cx| {
                let Some(entity) = entity.upgrade() else {
                    return;
                };
                view_alive = true;
                entity.update(cx, |view, cx| {
                    match outcome {
                        Ok(name) => {
                            view.status = t!("Extension.uninstalled", name = name.clone())
                                .to_string()
                                .into();
                            view.refresh_after_extension_change(kind, cx);
                            window.push_notification(
                                Notification::success(
                                    t!("Extension.uninstalled", name = name).to_string(),
                                ),
                                cx,
                            );
                        }
                        Err(err) => {
                            view.busy = None;
                            view.status = format_status_error(
                                &t!("Extension.uninstall_failed").to_string(),
                                &err,
                            )
                            .into();
                            window.push_notification(
                                Notification::error(t!("Extension.uninstall_failed").to_string()),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                });
            });
            if updated.is_err() {
                let _ = cx.update(|cx| {
                    refresh_host.refresh_after_extension_change(kind, cx);
                    crate::shell::finish_shell_extension(&gate_id, cx);
                });
            } else {
                let _ = cx.update(|cx| {
                    if !view_alive {
                        refresh_host.refresh_after_extension_change(kind, cx);
                    }
                    crate::shell::finish_shell_extension(&gate_id, cx);
                });
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn reload_extension(
        &mut self,
        summary: ExtensionSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        let name = summary.name.clone();
        self.busy = Some(format!("reload:{name}"));
        self.status = t!("Extension.reloading", name = name.clone())
            .to_string()
            .into();
        let close_task = crate::shell::close_shell_extension(&name, window, cx);
        let host = self.host.clone();
        let refresh_host = host.clone();
        let entity = cx.entity().downgrade();
        let window_handle = window.window_handle();
        let gate_id = name.clone();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            if !close_task.await {
                let _ = cx.update(|cx| {
                    crate::shell::finish_shell_extension(&gate_id, cx);
                    let _ = entity.update(cx, |view, cx| {
                        view.busy = None;
                        view.status = t!("Extension.tabs_close_failed").to_string().into();
                        cx.notify();
                    });
                });
                return;
            }
            let mut view_alive = false;
            let updated = cx.update_window(window_handle, |_, window, cx| {
                let Some(entity) = entity.upgrade() else {
                    return;
                };
                view_alive = true;
                entity.update(cx, |view, cx| {
                    match host.reload(&summary, cx) {
                        Ok(installed) => {
                            let reloaded_name = installed
                                .iter()
                                .find(|installed| installed.path == summary.path)
                                .map(|installed| installed.name.clone())
                                .unwrap_or(name);
                            apply_installed_reload_success(
                                &mut view.installed,
                                &mut view.busy,
                                &mut view.status,
                                &reloaded_name,
                                installed,
                            );
                            window.push_notification(
                                Notification::success(
                                    t!("Extension.reloaded", name = reloaded_name).to_string(),
                                ),
                                cx,
                            );
                        }
                        Err(err) => {
                            view.busy = None;
                            view.status = format_status_error(
                                &t!("Extension.reload_failed").to_string(),
                                &err,
                            )
                            .into();
                            let message = format_notification_error(
                                &t!("Extension.reload_failed").to_string(),
                                &err,
                            );
                            window.push_notification(
                                Notification::error(message).autohide(false),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                });
            });
            if updated.is_err() {
                let _ = cx.update(|cx| {
                    refresh_host.refresh_after_extension_change(summary.kind, cx);
                    crate::shell::finish_shell_extension(&gate_id, cx);
                });
            } else {
                let _ = cx.update(|cx| {
                    if !view_alive {
                        refresh_host.refresh_after_extension_change(summary.kind, cx);
                    }
                    crate::shell::finish_shell_extension(&gate_id, cx);
                });
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn refresh_after_extension_change(&mut self, kind: ExtensionKind, cx: &mut App) {
        self.host.refresh_after_extension_change(kind, cx);
        self.refresh_installed_from_host();
    }

    pub(crate) fn ensure_marketplace_loaded(&mut self, cx: &mut Context<Self>) {
        if should_auto_load_marketplace(
            self.mode,
            self.marketplace_entries.is_empty(),
            self.marketplace_load_attempted,
            self.marketplace_load_state.is_loading(),
        ) {
            self.load_marketplace(cx);
        }
    }

    pub(crate) fn load_marketplace_from_search_if_manifest_url(&mut self, cx: &mut Context<Self>) {
        if self.marketplace_manifest_url(cx).is_some() {
            self.mode = crate::ExtensionManagerMode::Marketplace;
            self.load_marketplace(cx);
        }
    }

    fn set_installed(&mut self, installed: Vec<ExtensionSummary>) {
        self.installed = installed;
        self.status = t!("Extension.loaded_installed", count = self.installed.len())
            .to_string()
            .into();
    }

    fn refresh_installed_from_host(&mut self) {
        if let Ok(installed) = self.host.list_installed() {
            self.set_installed(installed);
        }
        self.busy = None;
    }

    fn marketplace_manifest_url(&self, cx: &App) -> Option<String> {
        let query = self.search.read(cx).text().to_string();
        marketplace_manifest_url_from_query(&query)
    }
}

fn install_local_on_active_window(
    entity: gpui::WeakEntity<ExtensionManagerView>,
    path: PathBuf,
    cx: &mut AsyncApp,
) {
    let _ = cx.update(|cx| {
        let Some(window_id) = cx.active_window() else {
            return;
        };
        let _ = cx.update_window(window_id, |_, window, cx| {
            let Some(entity) = entity.upgrade() else {
                return;
            };
            entity.update(cx, |view, cx| {
                view.install_local_tarball(path, window, cx);
            });
        });
    });
}

fn finish_extension_action(
    entity: gpui::WeakEntity<ExtensionManagerView>,
    window_handle: gpui::AnyWindowHandle,
    outcome: anyhow::Result<MarketplaceInstallOutcome>,
    cx: &mut AsyncApp,
) {
    // `update_window` 失败时闭包不会执行，用 `Option` 把结果取回来走无窗口兜底。
    let mut outcome = Some(outcome);
    let updated = cx.update_window(window_handle, |_, window, cx| {
        let Some(outcome) = outcome.take() else {
            return;
        };
        let Some(entity) = entity.upgrade() else {
            return;
        };
        let entity_for_dialog = entity.clone();
        entity.update(cx, |view, cx| match outcome {
            Ok(outcome) => {
                view.finish_marketplace_outcome(outcome, entity_for_dialog, window, cx);
            }
            Err(err) => {
                view.busy = None;
                let message =
                    format_notification_error(&t!("Extension.install_failed").to_string(), &err);
                view.status =
                    format_status_error(&t!("Extension.install_failed_short").to_string(), &err)
                        .into();
                window.push_notification(Notification::error(message).autohide(false), cx);
            }
        });
    });
    let Some(outcome) = outcome.filter(|_| updated.is_err()) else {
        return;
    };
    // 窗口在安装过程中被关闭（关闭了详情弹窗、主窗口收进托盘等）：结果无法再通过
    // 窗口投递，但必须落到视图上，否则 `busy` 会一直卡住，扩展页所有安装/卸载
    // 按钮都会被永久禁用，表现为「安装和卸载都没反应」。
    let _ = cx.update(|cx| finish_extension_action_without_window(entity, outcome, cx));
}

/// 没有可用窗口时的安装收尾：只更新视图状态与磁盘暂存，不弹通知/对话框。
fn finish_extension_action_without_window(
    entity: gpui::WeakEntity<ExtensionManagerView>,
    outcome: anyhow::Result<MarketplaceInstallOutcome>,
    cx: &mut App,
) {
    let Some(entity) = entity.upgrade() else {
        return;
    };
    entity.update(cx, |view, cx| {
        view.busy = None;
        match outcome {
            Ok(MarketplaceInstallOutcome::Installed(summary)) => {
                view.status = t!("Extension.installed_name", name = summary.name.clone())
                    .to_string()
                    .into();
                view.refresh_after_extension_change(summary.kind, cx);
            }
            Ok(MarketplaceInstallOutcome::NeedsPermission(downloaded)) => {
                // 需要权限确认却弹不出对话框：放弃这次安装并清掉暂存目录，
                // 状态回到可重试，而不是把 busy 留在原地。
                crate::permissions::cleanup_staging(downloaded.staging);
                view.status = t!("Extension.install_cancelled").to_string().into();
            }
            Err(err) => {
                view.status =
                    format_status_error(&t!("Extension.install_failed_short").to_string(), &err)
                        .into();
            }
        }
        cx.notify();
    });
}

fn finish_marketplace_load(
    entity: gpui::WeakEntity<ExtensionManagerView>,
    outcome: anyhow::Result<Vec<MarketplaceEntry>>,
    cx: &mut AsyncApp,
) {
    let _ = cx.update(|cx| {
        let Some(window_id) = cx.active_window() else {
            update_marketplace_load_without_notification(entity, outcome, cx);
            return;
        };
        let _ = cx.update_window(window_id, |_, window, cx| {
            let Some(entity) = entity.upgrade() else {
                return;
            };
            entity.update(cx, |view, cx| {
                let notification = apply_marketplace_load_result(
                    &mut view.marketplace_entries,
                    &mut view.marketplace_load_state,
                    &mut view.status,
                    outcome,
                );
                if let Some(message) = notification {
                    window.push_notification(Notification::error(message).autohide(false), cx);
                }
                cx.notify();
            });
        });
    });
}

fn update_marketplace_load_without_notification(
    entity: gpui::WeakEntity<ExtensionManagerView>,
    outcome: anyhow::Result<Vec<MarketplaceEntry>>,
    cx: &mut App,
) {
    let Some(entity) = entity.upgrade() else {
        return;
    };
    entity.update(cx, |view, cx| {
        apply_marketplace_load_result(
            &mut view.marketplace_entries,
            &mut view.marketplace_load_state,
            &mut view.status,
            outcome,
        );
        cx.notify();
    });
}

impl ExtensionManagerView {
    fn install_local_tarball(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.host.review_local_tarball(path) {
            Ok(outcome) => {
                let entity = cx.entity().clone();
                self.finish_marketplace_outcome(outcome, entity, window, cx);
            }
            Err(err) => {
                let message =
                    format_notification_error(&t!("Extension.install_failed").to_string(), &err);
                self.status =
                    format_status_error(&t!("Extension.install_failed_short").to_string(), &err)
                        .into();
                window.push_notification(Notification::error(message).autohide(false), cx);
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn uninstall_runs_host_io_on_background_executor() {
        let source = include_str!("actions.rs");
        let uninstall = source
            .split("pub(crate) fn uninstall_extension")
            .nth(1)
            .and_then(|rest| rest.split("pub(crate) fn reload_extension").next())
            .expect("uninstall action should exist");

        assert!(
            uninstall.contains("cx.background_spawn"),
            "uninstall must move host I/O off the UI thread"
        );
        assert!(
            !uninstall.contains("match self.host.uninstall(&summary)"),
            "uninstall must not call the synchronous host method from the click callback"
        );
    }

    #[test]
    fn install_result_survives_the_originating_window_closing() {
        let source = include_str!("actions.rs");
        let install = source
            .split("pub(crate) fn install_marketplace_entry")
            .nth(1)
            .and_then(|rest| rest.split("pub(crate) fn uninstall_extension").next())
            .expect("install action should exist");
        let finish = source
            .split("fn finish_extension_action(")
            .nth(1)
            .and_then(|rest| rest.split("fn finish_marketplace_load").next())
            .expect("install completion should exist");

        assert!(
            install.contains("window.window_handle()"),
            "install must capture the clicking window instead of resolving it later"
        );
        assert!(
            !finish.contains("active_window"),
            "install completion must not depend on `active_window()`, which is None when \
             the app is not frontmost"
        );
        assert!(
            finish.contains("finish_extension_action_without_window"),
            "a closed window must still clear `busy`, otherwise every install/uninstall \
             button stays disabled forever"
        );
        assert!(
            include_str!("permissions.rs").contains("view.busy = None;"),
            "the permission-confirm path must clear `busy` on every exit"
        );
    }
}
