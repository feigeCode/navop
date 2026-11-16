//! `navop.ui` 能力面（模块 `ui`）。
//!
//! 页面侧入口：
//!
//! ```text
//! host.call("navop.ui.notify", { level?, title, message }) -> { ok: true }
//! ```
//!
//! `level` 取 `info`（默认）/ `success` / `warning` / `error`。通知经
//! 主线程派发到当前活动窗口（与 `navop.app.open` 同一套窗口解析规则）。
//!
//! `progress` 资源句柄本轮未提供：面板/页面场景下长任务应走
//! `navop.job`（已有进度面），需要同步进度条时再补
//! `navop.ui.startProgress`。

use serde_json::{Value, json};
/// 主线程上弹通知：复用 `navop.app.open` 的窗口解析（活动窗口 → 窗口栈）。
pub(crate) fn notify_on_main(level: &str, title: &str, message: &str, cx: &mut gpui::App) -> Result<Value, String> {
    let level = level.to_string();
    let title = title.to_string();
    let message = message.to_string();

    cx.defer(move |cx| {
        let Some(window) = cx
            .active_window()
            .or_else(|| cx.window_stack().and_then(|stack| stack.first().copied()))
        else {
            tracing::warn!("no window available for navop.ui.notify");
            return;
        };
        let _ = window.update(cx, |_, window, cx| {
            use gpui_component::{WindowExt as _, notification::Notification};

            let notification = match level.as_str() {
                "success" => Notification::success(title),
                "warning" => Notification::warning(title),
                "error" => Notification::error(title),
                _ => Notification::info(title),
            };
            let notification = if message.is_empty() {
                notification
            } else {
                notification.message(message)
            };
            window.push_notification(notification.autohide(true), cx);
        });
    });
    // 通知是尽力而为：找不到窗口只丢一条提示，不该让页面调用报错。
    Ok(json!({ "ok": true }))
}
