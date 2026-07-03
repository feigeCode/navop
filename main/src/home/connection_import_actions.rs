use super::connection_import_draft::{
    EditableImportDraft, selected_import_count, selected_import_drafts_to_connections,
};
use crate::setting_tab::GlobalCurrentUser;
use extension_runtime::{
    connection_import_provider::preview_manifest_connection_importers,
    extension::{ExtensionKind, extensions_root},
};
use gpui::App;
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::storage::{
    ConnectionRepository, GlobalStorageState, StoredConnection, traits::Repository,
};

pub(crate) fn preview_import_drafts(
    importer_ids: &[String],
) -> Result<Vec<EditableImportDraft>, String> {
    if importer_ids.is_empty() {
        return Ok(Vec::new());
    }
    let root = extensions_root().ok_or_else(|| "扩展目录不可用".to_string())?;
    let composite_root = root.join(ExtensionKind::Composite.dir_name());
    let records = futures::executor::block_on(preview_manifest_connection_importers(
        &composite_root,
        importer_ids,
        true,
    ))
    .map_err(|error| error.to_string())?;
    Ok(records.into_iter().map(EditableImportDraft::new).collect())
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
