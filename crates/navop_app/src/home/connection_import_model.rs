use std::collections::{BTreeMap, BTreeSet};

use connection_import_protocol::{
    ImportRecord, ImportRecordKind, ImportScanReport, ImporterAvailability, ImporterDescriptor,
    Platform,
};
use extension_runtime::connection_import_provider::ImportPreviewError;

use super::connection_import_draft::{
    EditableImportDraft, ImportDraftKind, normalized_ssh_group_path, normalized_workspace_path,
};

pub(crate) struct ImportCenterState {
    sources: Vec<ImportSourceState>,
    rows: Vec<ImportPreviewRow>,
}

pub(crate) struct ImportSourceState {
    pub(crate) descriptor: ImporterDescriptor,
    pub(crate) selected: bool,
    pub(crate) selectable: bool,
    pub(crate) availability: ImporterAvailability,
    pub(crate) scan_error: Option<String>,
    pub(crate) preview_error: Option<String>,
    pub(crate) discovered_workspace_paths: Vec<String>,
}

pub(crate) struct ImportPreviewRow {
    pub(crate) draft: EditableImportDraft,
    pub(crate) selected: bool,
    pub(crate) save_status: ImportRowSaveStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImportRowSaveStatus {
    Pending,
    Saving,
    Saved { connection_id: Option<i64> },
    Failed { message: String },
    SkippedDuplicate { existing_name: String },
}

impl ImportCenterState {
    pub(crate) fn new(descriptors: Vec<ImporterDescriptor>, platform: Platform) -> Self {
        let sources = descriptors
            .into_iter()
            .map(|descriptor| ImportSourceState::new(descriptor, platform))
            .collect();
        Self {
            sources,
            rows: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_tests() -> Self {
        Self {
            sources: Vec::new(),
            rows: Vec::new(),
        }
    }

    pub(crate) fn selected_source_ids(&self) -> Vec<String> {
        self.sources
            .iter()
            .filter(|source| source.selected && source.selectable)
            .map(|source| source.descriptor.id.clone())
            .collect()
    }

    pub(crate) fn sources(&self) -> &[ImportSourceState] {
        &self.sources
    }

    #[cfg(test)]
    pub(crate) fn source(&self, importer_id: &str) -> Option<&ImportSourceState> {
        self.sources
            .iter()
            .find(|source| source.descriptor.id == importer_id)
    }

    pub(crate) fn rows(&self) -> &[ImportPreviewRow] {
        &self.rows
    }

    pub(crate) fn workspace_group_paths(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|row| row.selected)
            .filter_map(|row| match row.draft.kind() {
                ImportDraftKind::Ssh => normalized_ssh_group_path(&row.draft.ssh_group_path),
                ImportDraftKind::Workspace => row.draft.workspace_path(),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub(crate) fn row(&self, record_id: &str) -> Option<&ImportPreviewRow> {
        self.rows.iter().find(|row| row.record_id() == record_id)
    }

    pub(crate) fn next_save_candidate_row_id_after(&self, record_id: &str) -> Option<String> {
        let current_index = self
            .rows
            .iter()
            .position(|row| row.record_id() == record_id)?;
        self.rows
            .iter()
            .skip(current_index + 1)
            .find(|row| row.selected && is_save_candidate(&row.save_status))
            .map(|row| row.record_id().to_string())
    }

    pub(crate) fn apply_scan_reports(&mut self, reports: Vec<ImportScanReport>) {
        for report in reports {
            if let Some(source) = self
                .sources
                .iter_mut()
                .find(|source| source.descriptor.id == report.importer_id)
            {
                source.availability = report.availability;
                source.discovered_workspace_paths = report
                    .discovered_workspace_paths
                    .into_iter()
                    .filter_map(|path| normalized_workspace_path(&path))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                source.scan_error = match &source.availability {
                    ImporterAvailability::Error { message } => Some(message.clone()),
                    _ => None,
                };
            }
        }
    }

    pub(crate) fn apply_preview_errors(
        &mut self,
        importer_ids: &[String],
        errors: Vec<ImportPreviewError>,
    ) {
        for source in self
            .sources
            .iter_mut()
            .filter(|source| importer_ids.contains(&source.descriptor.id))
        {
            source.preview_error = None;
        }
        for error in errors {
            if let Some(source) = self
                .sources
                .iter_mut()
                .find(|source| source.descriptor.id == error.importer_id)
            {
                source.preview_error = Some(error.message);
            }
        }
    }

    pub(crate) fn apply_preview_records(&mut self, records: Vec<ImportRecord>) {
        let mut workspace_paths_by_importer = BTreeMap::<String, BTreeSet<String>>::new();
        for record in &records {
            let workspace_path = match record.kind {
                ImportRecordKind::Ssh => record
                    .ssh
                    .as_ref()
                    .and_then(|ssh| ssh.group_path.as_deref())
                    .and_then(normalized_ssh_group_path),
                ImportRecordKind::Workspace => record
                    .workspace
                    .as_ref()
                    .and_then(|workspace| normalized_workspace_path(&workspace.path)),
                _ => None,
            };
            if let Some(workspace_path) = workspace_path {
                workspace_paths_by_importer
                    .entry(record.importer_id.clone())
                    .or_default()
                    .insert(workspace_path);
            }
        }
        for source in &mut self.sources {
            if let Some(workspace_paths) = workspace_paths_by_importer.remove(&source.descriptor.id)
                && !workspace_paths.is_empty()
            {
                source.discovered_workspace_paths = workspace_paths.into_iter().collect();
            }
        }

        self.rows = records
            .into_iter()
            .map(|record| ImportPreviewRow {
                draft: EditableImportDraft::new(record),
                selected: true,
                save_status: ImportRowSaveStatus::Pending,
            })
            .collect();
    }

    pub(crate) fn toggle_source(&mut self, importer_id: &str) {
        if let Some(source) = self
            .sources
            .iter_mut()
            .find(|source| source.descriptor.id == importer_id && source.selectable)
        {
            source.selected = !source.selected;
        }
    }

    pub(crate) fn toggle_row(&mut self, record_id: &str) {
        if let Some(row) = self
            .rows
            .iter_mut()
            .find(|row| row.record_id() == record_id)
        {
            row.selected = !row.selected;
        }
    }

    pub(crate) fn mark_saving(&mut self, record_id: &str) {
        if let Some(row) = self.row_mut(record_id) {
            row.save_status = ImportRowSaveStatus::Saving;
        }
    }

    pub(crate) fn mark_saved(&mut self, record_id: &str, connection_id: Option<i64>) {
        if let Some(row) = self.row_mut(record_id) {
            row.save_status = ImportRowSaveStatus::Saved { connection_id };
        }
    }

    pub(crate) fn mark_failed(&mut self, record_id: &str, message: String) {
        if let Some(row) = self.row_mut(record_id) {
            row.save_status = ImportRowSaveStatus::Failed { message };
        }
    }

    pub(crate) fn mark_duplicate(&mut self, record_id: &str, existing_name: String) {
        if let Some(row) = self.row_mut(record_id) {
            row.save_status = ImportRowSaveStatus::SkippedDuplicate { existing_name };
        }
    }

    fn row_mut(&mut self, record_id: &str) -> Option<&mut ImportPreviewRow> {
        self.rows
            .iter_mut()
            .find(|row| row.record_id() == record_id)
    }
}

impl ImportSourceState {
    fn new(descriptor: ImporterDescriptor, platform: Platform) -> Self {
        let selectable = descriptor.supported_platforms.is_empty()
            || descriptor.supported_platforms.contains(&platform);
        let availability = if selectable {
            ImporterAvailability::Installed
        } else {
            ImporterAvailability::UnsupportedPlatform
        };
        Self {
            descriptor,
            selected: selectable,
            selectable,
            availability,
            scan_error: None,
            preview_error: None,
            discovered_workspace_paths: Vec::new(),
        }
    }
}

impl ImportPreviewRow {
    pub(crate) fn record_id(&self) -> &str {
        self.draft.source_id()
    }
}

pub(crate) fn previewable_source_ids_after_scan(
    selected_ids: &[String],
    reports: &[ImportScanReport],
) -> Vec<String> {
    selected_ids
        .iter()
        .filter(|id| {
            reports
                .iter()
                .find(|report| report.importer_id.as_str() == id.as_str())
                .map(|report| is_previewable_availability(&report.availability))
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

fn is_previewable_availability(availability: &ImporterAvailability) -> bool {
    matches!(
        availability,
        ImporterAvailability::Available { .. } | ImporterAvailability::Installed
    )
}

fn is_save_candidate(status: &ImportRowSaveStatus) -> bool {
    matches!(
        status,
        ImportRowSaveStatus::Pending | ImportRowSaveStatus::Failed { .. }
    )
}
