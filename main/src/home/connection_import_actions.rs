use crate::setting_tab::GlobalCurrentUser;
use connection_importer::{
    ImportOptions, ImportSourceKind, preview_connections, preview_ssh_connections,
    to_db_connection_config, to_ssh_params,
};
use gpui::App;
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::storage::{
    ConnectionRepository, GlobalStorageState, StoredConnection, traits::Repository,
};

pub(crate) fn import_connection_sources(
    sources: &[ImportSourceKind],
    cx: &mut App,
) -> Result<usize, String> {
    sources.iter().try_fold(0usize, |count, source| {
        import_connections(*source, cx).map(|n| count + n)
    })
}

fn import_connections(kind: ImportSourceKind, cx: &mut App) -> Result<usize, String> {
    if is_ssh_source(kind) {
        return import_ssh_connections(kind, cx);
    }
    import_database_connections(kind, cx)
}

fn is_ssh_source(kind: ImportSourceKind) -> bool {
    matches!(kind, ImportSourceKind::Xshell)
}

fn import_database_connections(kind: ImportSourceKind, cx: &mut App) -> Result<usize, String> {
    let imported = preview_connections(
        kind,
        ImportOptions {
            include_passwords: true,
        },
    )
    .map_err(|error| error.to_string())?;
    if imported.is_empty() {
        return Ok(0);
    }

    let mut saved = Vec::with_capacity(imported.len());
    for imported_connection in imported {
        let config =
            to_db_connection_config(imported_connection).map_err(|error| error.to_string())?;
        saved.push(StoredConnection::from_db_connection(config));
    }

    save_imported_connections(saved, cx)
}

fn import_ssh_connections(kind: ImportSourceKind, cx: &mut App) -> Result<usize, String> {
    let imported = preview_ssh_connections(
        kind,
        ImportOptions {
            include_passwords: true,
        },
    )
    .map_err(|error| error.to_string())?;
    if imported.is_empty() {
        return Ok(0);
    }

    let mut saved = Vec::with_capacity(imported.len());
    for imported_connection in imported {
        let name = imported_connection.name.clone();
        let params = to_ssh_params(imported_connection).map_err(|error| error.to_string())?;
        saved.push(StoredConnection::new_ssh(name, params, None));
    }

    save_imported_connections(saved, cx)
}

fn save_imported_connections(
    mut connections: Vec<StoredConnection>,
    cx: &mut App,
) -> Result<usize, String> {
    let owner_id = GlobalCurrentUser::get_user(cx).map(|user| user.id);
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let repo = storage
        .get::<ConnectionRepository>()
        .ok_or_else(|| "ConnectionRepository not found".to_string())?;

    for connection in &mut connections {
        connection.owner_id = owner_id.clone();
        repo.insert(connection).map_err(|error| error.to_string())?;
    }

    notify_connections_created(connections.clone(), cx);
    Ok(connections.len())
}

fn notify_connections_created(connections: Vec<StoredConnection>, cx: &mut App) {
    let Some(notifier) = get_notifier(cx) else {
        return;
    };
    for connection in connections {
        notifier.update(cx, |_, cx| {
            cx.emit(ConnectionDataEvent::ConnectionCreated { connection });
        });
    }
}
