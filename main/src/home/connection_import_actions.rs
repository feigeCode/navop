use std::path::PathBuf;

use super::connection_import_draft::{
    EditableImportDraft, ImportDraftKind, normalized_ssh_group_path, normalized_workspace_path,
};
use super::connection_import_draft_conversion::{
    quick_command_duplicate_identity, stored_connection_duplicate_identity,
};
use crate::setting_tab::GlobalCurrentUser;
use connection_import_protocol::ImportScanReport;
use extension_runtime::{
    connection_import_provider::{
        ImportPreviewReport, ManualConnectionImportFile, preview_manifest_connection_importers,
        preview_manifest_connection_importers_with_files, scan_manifest_connection_importers,
    },
    extension::{ExtensionKind, extensions_root},
};
use gpui::App;
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::storage::StorageManager;
use one_core::storage::{
    ConnectionRepository, GlobalStorageState, QuickCommand, QuickCommandRepository,
    StoredConnection, Workspace, WorkspaceRepository, traits::Repository,
};
use rust_i18n::t;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImportSaveResult {
    Saved { connection_id: Option<i64> },
    SkippedDuplicate { existing_name: String },
}

pub(crate) async fn scan_import_sources(
    importer_ids: Vec<String>,
) -> Result<Vec<ImportScanReport>, String> {
    if importer_ids.is_empty() {
        return Ok(Vec::new());
    }
    let composite_root = composite_extensions_root()?;
    scan_manifest_connection_importers(&composite_root, &importer_ids)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn preview_import_records(
    importer_ids: Vec<String>,
    include_passwords: bool,
) -> Result<ImportPreviewReport, String> {
    if importer_ids.is_empty() {
        return Ok(ImportPreviewReport::default());
    }
    let composite_root = composite_extensions_root()?;
    preview_manifest_connection_importers(&composite_root, &importer_ids, include_passwords)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn preview_import_records_from_files(
    importer_id: String,
    file_paths: Vec<PathBuf>,
    include_passwords: bool,
) -> Result<ImportPreviewReport, String> {
    if file_paths.is_empty() {
        return Ok(ImportPreviewReport::default());
    }
    let composite_root = composite_extensions_root()?;
    let manual_files = file_paths
        .into_iter()
        .map(|path| ManualConnectionImportFile::new(importer_id.clone(), path))
        .collect::<Vec<_>>();
    preview_manifest_connection_importers_with_files(
        &composite_root,
        std::slice::from_ref(&importer_id),
        include_passwords,
        &manual_files,
    )
    .await
    .map_err(|error| error.to_string())
}

pub(crate) fn duplicate_connection_name(
    draft: &EditableImportDraft,
    existing: &[StoredConnection],
) -> Result<Option<String>, String> {
    Ok(duplicate_connection(draft, existing)?.map(|connection| connection.name.clone()))
}

fn duplicate_connection<'a>(
    draft: &EditableImportDraft,
    existing: &'a [StoredConnection],
) -> Result<Option<&'a StoredConnection>, String> {
    let draft_identity = draft.duplicate_identity()?;
    for connection in existing {
        let Some(existing_identity) = stored_connection_duplicate_identity(connection)? else {
            continue;
        };
        if existing_identity == draft_identity {
            return Ok(Some(connection));
        }
    }
    Ok(None)
}

pub(crate) fn duplicate_quick_command_name(
    draft: &EditableImportDraft,
    existing: &[QuickCommand],
) -> Result<Option<String>, String> {
    let identity = quick_command_duplicate_identity(draft)?;
    for command in existing
        .iter()
        .filter(|command| command.connection_id.is_none())
    {
        let existing_identity = format!(
            "quick-command:{}:{}:{}",
            command
                .group_name
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase(),
            command
                .name
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase(),
            command.command.trim()
        );
        if existing_identity == identity {
            return Ok(Some(
                command
                    .name
                    .clone()
                    .unwrap_or_else(|| command.command.clone()),
            ));
        }
    }
    Ok(None)
}

pub(crate) fn save_import_draft(
    draft: &EditableImportDraft,
    cx: &mut App,
) -> Result<ImportSaveResult, String> {
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    if draft.kind() == ImportDraftKind::Workspace {
        let path = draft
            .workspace_path()
            .ok_or_else(|| t!("Home.ConnectionImport.workspace_path_required").to_string())?;
        let resolution = workspace_path_id(&path, &storage)?;
        notify_workspaces_created(&resolution.created_workspace_ids, cx);
        return Ok(ImportSaveResult::Saved {
            connection_id: None,
        });
    }
    if matches!(draft.kind(), ImportDraftKind::QuickCommand) {
        let repo = storage
            .get::<QuickCommandRepository>()
            .ok_or_else(|| t!("Home.ConnectionImport.repository_unavailable").to_string())?;
        let existing = repo
            .list_by_connection(None)
            .map_err(|error| error.to_string())?;
        if let Some(existing_name) = duplicate_quick_command_name(draft, &existing)? {
            return Ok(ImportSaveResult::SkippedDuplicate { existing_name });
        }
        let mut quick_command = draft.to_quick_command()?;
        repo.insert(&mut quick_command)
            .map_err(|error| error.to_string())?;
        return Ok(ImportSaveResult::Saved {
            connection_id: quick_command.id,
        });
    }

    let repo = storage
        .get::<ConnectionRepository>()
        .ok_or_else(|| t!("Home.ConnectionImport.repository_unavailable").to_string())?;
    let existing = repo.list().map_err(|error| error.to_string())?;
    if let Some(existing_connection) = duplicate_connection(draft, &existing)? {
        if draft.kind() == ImportDraftKind::Ssh
            && existing_connection.workspace_id.is_none()
            && normalized_ssh_group_path(&draft.ssh_group_path).is_some()
        {
            let resolution = ssh_group_workspace_id(draft, &storage)?;
            if let Some(workspace_id) = resolution.workspace_id {
                let connection_id = existing_connection
                    .id
                    .ok_or_else(|| "Existing connection ID is missing".to_string())?;
                let mut updated_connection = existing_connection.clone();
                updated_connection.workspace_id = Some(workspace_id);
                updated_connection.updated_at = Some(
                    repo.update_workspace(connection_id, Some(workspace_id))
                        .map_err(|error| error.to_string())?,
                );
                notify_workspaces_created(&resolution.created_workspace_ids, cx);
                notify_connection_updated(updated_connection, cx);
                return Ok(ImportSaveResult::Saved {
                    connection_id: Some(connection_id),
                });
            }
        }
        return Ok(ImportSaveResult::SkippedDuplicate {
            existing_name: existing_connection.name.clone(),
        });
    }

    let workspace_resolution = ssh_group_workspace_id(draft, &storage)?;
    let mut connection = draft.to_stored_connection()?;
    connection.workspace_id = workspace_resolution.workspace_id;
    connection.owner_id = GlobalCurrentUser::get_user(cx).map(|user| user.id);
    repo.insert(&mut connection)
        .map_err(|error| error.to_string())?;
    let connection_id = connection.id;
    notify_workspaces_created(&workspace_resolution.created_workspace_ids, cx);
    notify_connections_created(vec![connection], cx);
    Ok(ImportSaveResult::Saved { connection_id })
}

#[derive(Default)]
struct WorkspaceResolution {
    workspace_id: Option<i64>,
    created_workspace_ids: Vec<i64>,
}

fn workspace_path_id(path: &str, storage: &StorageManager) -> Result<WorkspaceResolution, String> {
    let group_path = normalized_workspace_path(path)
        .ok_or_else(|| t!("Home.ConnectionImport.workspace_path_required").to_string())?;
    let repo = storage
        .get::<WorkspaceRepository>()
        .ok_or_else(|| t!("Home.ConnectionImport.repository_unavailable").to_string())?;
    let mut workspaces = repo.list().map_err(|error| error.to_string())?;
    let mut parent_id = None;
    let mut created_workspace_ids = Vec::new();
    for component in group_path.split('/') {
        let workspace_id = match workspaces
            .iter()
            .find(|workspace| workspace.parent_id == parent_id && workspace.name == component)
        {
            Some(existing) => existing.id,
            None => {
                let mut workspace = Workspace::new(component.to_string());
                workspace.parent_id = parent_id;
                repo.insert(&mut workspace)
                    .map_err(|error| error.to_string())?;
                let workspace_id = workspace
                    .id
                    .ok_or_else(|| "Workspace repository did not assign an ID".to_string())?;
                created_workspace_ids.push(workspace_id);
                workspaces.push(workspace.clone());
                Some(workspace_id)
            }
        };
        parent_id = workspace_id;
    }
    Ok(WorkspaceResolution {
        workspace_id: parent_id,
        created_workspace_ids,
    })
}

fn ssh_group_workspace_id(
    draft: &EditableImportDraft,
    storage: &StorageManager,
) -> Result<WorkspaceResolution, String> {
    let Some(group_path) = normalized_ssh_group_path(&draft.ssh_group_path) else {
        return Ok(WorkspaceResolution::default());
    };
    workspace_path_id(&group_path, storage)
}

fn composite_extensions_root() -> Result<std::path::PathBuf, String> {
    let root = extensions_root()
        .ok_or_else(|| t!("Home.ConnectionImport.extension_directory_unavailable").to_string())?;
    Ok(root.join(ExtensionKind::Composite.dir_name()))
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

fn notify_connection_updated(connection: StoredConnection, cx: &mut App) {
    let Some(notifier) = get_notifier(cx) else {
        return;
    };
    notifier.update(cx, |_, cx| {
        cx.emit(ConnectionDataEvent::ConnectionUpdated { connection });
    });
}

fn notify_workspaces_created(workspace_ids: &[i64], cx: &mut App) {
    let Some(notifier) = get_notifier(cx) else {
        return;
    };
    for workspace_id in workspace_ids {
        notifier.update(cx, |_, cx| {
            cx.emit(ConnectionDataEvent::WorkspaceCreated {
                workspace_id: *workspace_id,
            });
        });
    }
}
