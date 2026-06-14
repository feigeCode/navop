use std::path::PathBuf;

use gpui::{App, AppContext, AsyncApp, Context, PathPromptOptions, WeakEntity, Window};
use gpui_component::{WindowExt, notification::Notification};

use crate::state::{apply_marketplace_load_result, should_auto_load_marketplace};
use crate::status_message::format_status_error;
use crate::{ExtensionManagerView, ExtensionSummary, MarketplaceEntry, MarketplaceInstallOutcome};

impl ExtensionManagerView {
    pub(crate) fn refresh_installed(&mut self, cx: &mut Context<Self>) {
        match self.host.list_installed() {
            Ok(installed) => self.set_installed(installed),
            Err(err) => {
                self.installed.clear();
                self.status = format!("读取扩展列表失败: {err}").into();
            }
        }
        cx.notify();
    }

    pub(crate) fn load_marketplace(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.marketplace_load_attempted = true;
        self.loading = true;
        self.status = "正在加载扩展市场...".into();
        let http_client = cx.http_client();
        let entity = cx.entity().downgrade();
        let task = cx.background_spawn(self.host.load_marketplace_entries(http_client));
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let outcome = task.await;
            let _ = entity.update(cx, |view, cx| {
                apply_marketplace_load_result(
                    &mut view.marketplace_entries,
                    &mut view.loading,
                    &mut view.status,
                    outcome,
                );
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn select_local_tarball(&mut self, cx: &mut Context<Self>) {
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择扩展压缩包".into()),
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
        self.status = format!("正在安装 {}...", entry.name).into();
        let http_client = cx.http_client();
        let entity = cx.entity().downgrade();
        let task = cx.background_spawn(self.host.review_marketplace_entry(http_client, entry));
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            finish_extension_action(entity, task.await, cx);
        })
        .detach();
        window.push_notification(Notification::info("扩展安装已开始").autohide(true), cx);
        cx.notify();
    }

    pub(crate) fn uninstall_extension(
        &mut self,
        summary: ExtensionSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.host.uninstall(&summary) {
            Ok(name) => {
                self.status = format!("已卸载 {name}").into();
                self.refresh_after_extension_change(cx);
                window.push_notification(Notification::success(format!("已卸载 {name}")), cx);
            }
            Err(err) => {
                self.status = format_status_error("卸载失败", &err).into();
                window.push_notification(Notification::error("扩展卸载失败"), cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn refresh_after_extension_change(&mut self, cx: &mut App) {
        self.host.refresh_after_extension_change(cx);
        self.refresh_installed_from_host();
    }

    pub(crate) fn ensure_marketplace_loaded(&mut self, cx: &mut Context<Self>) {
        if should_auto_load_marketplace(
            self.mode,
            self.marketplace_entries.is_empty(),
            self.marketplace_load_attempted,
            self.loading,
        ) {
            self.load_marketplace(cx);
        }
    }

    fn set_installed(&mut self, installed: Vec<ExtensionSummary>) {
        self.installed = installed;
        self.status = format!("已加载 {} 个本地扩展", self.installed.len()).into();
    }

    fn refresh_installed_from_host(&mut self) {
        if let Ok(installed) = self.host.list_installed() {
            self.set_installed(installed);
        }
        self.busy = None;
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
    outcome: anyhow::Result<MarketplaceInstallOutcome>,
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
            let entity_for_dialog = entity.clone();
            entity.update(cx, |view, cx| match outcome {
                Ok(outcome) => {
                    view.finish_marketplace_outcome(outcome, entity_for_dialog, window, cx);
                }
                Err(err) => {
                    view.busy = None;
                    view.status = format_status_error("安装失败", &err).into();
                    window.push_notification(Notification::error("扩展安装失败"), cx);
                }
            });
        });
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
                self.status = format_status_error("安装失败", &err).into();
                window.push_notification(Notification::error("扩展安装失败"), cx);
            }
        }
        cx.notify();
    }
}
