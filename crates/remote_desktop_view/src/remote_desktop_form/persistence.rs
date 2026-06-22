use gpui::Context;
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, StoredConnection};

use super::RemoteDesktopFormWindow;

pub fn persist_connection(
    mut connection: StoredConnection,
    is_editing: bool,
    cx: &mut Context<RemoteDesktopFormWindow>,
) -> anyhow::Result<StoredConnection> {
    let storage = cx
        .global::<one_core::storage::GlobalStorageState>()
        .storage
        .clone();
    let repository = storage
        .get::<ConnectionRepository>()
        .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;
    if is_editing {
        repository.update(&connection)?;
    } else {
        repository.insert(&mut connection)?;
    }
    Ok(connection)
}

pub fn emit_saved_connection(
    connection: StoredConnection,
    is_editing: bool,
    cx: &mut Context<RemoteDesktopFormWindow>,
) {
    let Some(notifier) = get_notifier(cx) else {
        return;
    };
    let event = if is_editing {
        ConnectionDataEvent::ConnectionUpdated { connection }
    } else {
        ConnectionDataEvent::ConnectionCreated { connection }
    };
    notifier.update(cx, |_, cx| cx.emit(event));
}
