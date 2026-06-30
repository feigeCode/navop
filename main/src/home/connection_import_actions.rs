use super::connection_import_draft::{
    EditableImportDraft, selected_import_count, selected_import_drafts_to_connections,
};
use crate::setting_tab::GlobalCurrentUser;
use connection_importer::{
    ImportOptions, ImportSourceKind, preview_connections, preview_ssh_connections,
};
use gpui::App;
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::storage::{
    ConnectionRepository, GlobalStorageState, StoredConnection, traits::Repository,
};

fn is_ssh_source(kind: ImportSourceKind) -> bool {
    matches!(
        kind,
        ImportSourceKind::Xshell | ImportSourceKind::FinalShell | ImportSourceKind::Termius
    )
}

pub(crate) fn preview_import_drafts(
    sources: &[ImportSourceKind],
) -> Result<Vec<EditableImportDraft>, String> {
    let mut drafts = Vec::new();
    for source in sources {
        drafts.extend(preview_source_import_drafts(*source)?);
    }
    Ok(drafts)
}

pub(crate) fn save_selected_import_drafts(
    drafts: &[EditableImportDraft],
    cx: &mut App,
) -> Result<usize, String> {
    if selected_import_count(drafts) == 0 {
        return Ok(0);
    }
    let connections = selected_import_drafts_to_connections(drafts)?;
    save_imported_connections(connections, cx)
}

fn preview_source_import_drafts(
    kind: ImportSourceKind,
) -> Result<Vec<EditableImportDraft>, String> {
    if is_ssh_source(kind) {
        return preview_ssh_import_drafts(kind);
    }
    preview_database_import_drafts(kind)
}

fn preview_database_import_drafts(
    kind: ImportSourceKind,
) -> Result<Vec<EditableImportDraft>, String> {
    let imported = preview_connections(
        kind,
        ImportOptions {
            include_passwords: true,
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(imported
        .into_iter()
        .map(EditableImportDraft::database)
        .collect())
}

fn preview_ssh_import_drafts(kind: ImportSourceKind) -> Result<Vec<EditableImportDraft>, String> {
    let imported = preview_ssh_connections(
        kind,
        ImportOptions {
            include_passwords: true,
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(imported.into_iter().map(EditableImportDraft::ssh).collect())
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
