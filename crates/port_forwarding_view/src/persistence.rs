use gpui::Context;
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::storage::StoredConnection;
use one_core::storage::traits::Repository;

use crate::form_window::PortForwardingFormWindow;

pub(super) fn save_connection(
    mut conn: StoredConnection,
    is_editing: bool,
    cx: &mut Context<PortForwardingFormWindow>,
) {
    let storage = cx
        .global::<one_core::storage::GlobalStorageState>()
        .storage
        .clone();
    cx.spawn(async move |_this, cx| {
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
            Ok(saved) => notify_connection_saved(saved, is_editing, cx),
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
