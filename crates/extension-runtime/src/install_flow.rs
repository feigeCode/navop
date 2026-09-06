use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use futures::Future;
use gpui::{
    AnyWindowHandle, AppContext, AsyncApp, Context, PromptLevel, WeakEntity, Window,
    http_client::HttpClient,
};
use gpui_component::{WindowExt, notification::Notification};
use one_core::gpui_tokio::Tokio;

use crate::database_driver_install_progress::{
    DriverInstallProgressSnapshot, DriverInstallProgressView, driver_install_progress_callback,
    mark_driver_install_finished, open_driver_install_progress_dialog, watch_driver_install_progress,
};
use crate::extension::ExtensionSummary;
use crate::extension_downloader::DownloadProgressCallback;

/// 统一的“确认提示 → 进度弹窗 → 后台安装 → 收尾回调”GPUI 安装流水线。
///
/// 数据库驱动与远程桌面插件安装共用；`install` 是能力域安装实现，由
/// runner 注入 `http_client` 与进度回调，`on_success` 在安装成功后执行。
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_install_with_progress_prompt<T, Fut, F>(
    window: &mut Window,
    cx: &mut Context<T>,
    progress_label: (String, String),
    prompt_title: &'static str,
    prompt_message: String,
    prompt_buttons: &'static [&str],
    install: F,
    on_success: impl FnOnce(&mut T, &mut Window, &mut Context<T>) + 'static,
    success_message: impl Into<String>,
    failure_prefix: impl Into<String>,
) where
    T: 'static,
    Fut: Future<Output = anyhow::Result<ExtensionSummary>> + Send + 'static,
    F: FnOnce(Arc<dyn HttpClient>, DownloadProgressCallback) -> Fut + Send + 'static,
{
    let answer = window.prompt(
        PromptLevel::Warning,
        prompt_title,
        Some(&prompt_message),
        prompt_buttons,
        cx,
    );
    let http_client = cx.http_client();
    let window_handle = window.window_handle();
    let progress_view =
        cx.new(|_| DriverInstallProgressView::new(&progress_label.0, &progress_label.1));
    let progress_view_weak = progress_view.downgrade();
    let progress_snapshot = Arc::new(Mutex::new(DriverInstallProgressSnapshot::default()));
    let progress_finished = Arc::new(AtomicBool::new(false));
    watch_driver_install_progress(
        progress_view_weak.clone(),
        Arc::clone(&progress_snapshot),
        Arc::clone(&progress_finished),
        cx,
    );
    let success_message = success_message.into();
    let failure_prefix = failure_prefix.into();
    cx.spawn(async move |this: WeakEntity<T>, cx: &mut AsyncApp| {
        if answer.await.ok() != Some(0) {
            progress_finished.store(true, Ordering::Relaxed);
            return;
        }
        open_install_progress_dialog(window_handle, progress_view, cx);
        let progress_callback = driver_install_progress_callback(progress_snapshot);
        let task = Tokio::spawn(cx, async move { install(http_client, progress_callback).await });
        let outcome = match task.await {
            Ok(Ok(_summary)) => Ok(()),
            Ok(Err(error)) => Err(format!("{error:?}")),
            Err(error) => Err(format!("任务执行失败: {error}")),
        };
        progress_finished.store(true, Ordering::Relaxed);
        finish_install_and_open(
            window_handle,
            this,
            progress_view_weak,
            outcome,
            on_success,
            success_message,
            failure_prefix,
            cx,
        );
    })
    .detach();
}

/// 打开带进度弹窗的安装对话框。数据库驱动与远程桌面插件安装共用。
pub(crate) fn open_install_progress_dialog(
    window_handle: AnyWindowHandle,
    progress_view: gpui::Entity<DriverInstallProgressView>,
    cx: &mut AsyncApp,
) {
    let _ = cx.update_window(window_handle, |_, window, cx| {
        open_driver_install_progress_dialog(progress_view, window, cx);
    });
}

/// 收尾一个带进度对话框的安装流程：标记完成、关闭对话框、成功回调或错误通知。
///
/// `success_message` 与 `failure_prefix` 由调用方按能力域构造文案；
/// 失败前缀会在末尾拼接具体 `error`。
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_install_and_open<T: 'static>(
    window_handle: AnyWindowHandle,
    target: WeakEntity<T>,
    progress_view: gpui::WeakEntity<DriverInstallProgressView>,
    outcome: Result<(), String>,
    on_success: impl FnOnce(&mut T, &mut Window, &mut Context<T>) + 'static,
    success_message: impl Into<String>,
    failure_prefix: impl Into<String>,
    cx: &mut AsyncApp,
) {
    if outcome.is_ok() {
        mark_driver_install_finished(&progress_view, cx);
    }
    let success_message = success_message.into();
    let failure_prefix = failure_prefix.into();
    let _ = cx.update_window(window_handle, |_, window, cx| {
        window.close_dialog(cx);
        if let Some(target) = target.upgrade() {
            target.update(cx, |target, cx| match outcome {
                Ok(()) => {
                    notify_success(window, cx, success_message);
                    on_success(target, window, cx);
                }
                Err(error) => notify_error(window, cx, format!("{failure_prefix}: {error}")),
            });
        }
    });
}

pub(crate) fn notify_error<T>(window: &mut Window, cx: &mut Context<T>, message: impl Into<String>) {
    window.push_notification(Notification::error(message.into()), cx);
}

pub(crate) fn notify_success<T>(
    window: &mut Window,
    cx: &mut Context<T>,
    message: impl Into<String>,
) {
    window.push_notification(Notification::success(message.into()), cx);
}