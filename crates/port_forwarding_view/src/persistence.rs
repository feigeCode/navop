use gpui::{AppContext as _, Context};
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::storage::StoredConnection;
use one_core::storage::traits::Repository;

use crate::form_window::PortForwardingFormWindow;

pub(super) fn save_connection(
    mut conn: StoredConnection,
    is_editing: bool,
    window_handle: gpui::AnyWindowHandle,
    cx: &mut Context<PortForwardingFormWindow>,
) {
    let storage = cx
        .global::<one_core::storage::GlobalStorageState>()
        .storage
        .clone();
    cx.spawn(async move |this, cx| {
        let result = (|| -> Result<StoredConnection, anyhow::Error> {
            let repo = storage
                .get::<one_core::storage::ConnectionRepository>()
                .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;
            if is_editing {
                repo.update(&mut conn)?;
            } else {
                repo.insert(&mut conn)?;
            };
            Ok(conn)
        })();
        match result {
            Ok(saved) => {
                // 保存已经落地：把表单切到「已保存」，并**再关一次窗**。
                // 前面那次关窗是同步发生的（在 `on_save` 里），窗口正常已经消失；只有隐藏失败时
                // 它会留在屏幕上（关闭漏斗返回 `Retained`）—— 那一次拿不到保存结果，所以提示
                // 要从这里发：到这个点才知道「已保存」，也只有到这里下一次保存才会变成更新。
                _ = cx.update_window(window_handle, |_, window, cx| {
                    let _ = this.update(cx, |form, cx| form.mark_saved(&saved, cx));
                    let _ = one_core::window_close::close_window_after_save(window, cx);
                });
                notify_connection_saved(saved, is_editing, cx)
            }
            Err(error) => tracing::error!("保存端口转发连接失败: {}", error),
        }
    })
    .detach();
}

fn notify_connection_saved(
    connection: StoredConnection,
    is_editing: bool,
    cx: &mut gpui::AsyncApp,
) {
    let _ = cx.update(|cx| {
        if let Some(notifier) = get_notifier(cx) {
            let event = if is_editing {
                ConnectionDataEvent::ConnectionUpdated { connection }
            } else {
                ConnectionDataEvent::ConnectionCreated { connection }
            };
            notifier.update(cx, |_, cx| cx.emit(event));
        }
    });
}
