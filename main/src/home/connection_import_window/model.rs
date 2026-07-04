use connection_import_protocol::{ImportRecord, ImportScanReport, ImporterDescriptor, Platform};

use super::is_save_candidate;
use crate::home::connection_import_draft::EditableImportDraft;
use crate::home::connection_import_model::{
    ImportCenterState, ImportPreviewRow, ImportSourceState,
};

pub(crate) struct ConnectionImportWindowModel {
    state: ImportCenterState,
}

impl ConnectionImportWindowModel {
    pub(crate) fn new(descriptors: Vec<ImporterDescriptor>) -> Self {
        Self {
            state: ImportCenterState::new(descriptors, current_platform()),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests(descriptors: Vec<ImporterDescriptor>) -> Self {
        Self {
            state: ImportCenterState::new(descriptors, Platform::Macos),
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_tests() -> Self {
        Self {
            state: ImportCenterState::empty_for_tests(),
        }
    }

    pub(crate) fn can_scan(&self) -> bool {
        !self.state.selected_source_ids().is_empty()
    }

    pub(crate) fn selected_source_ids(&self) -> Vec<String> {
        self.state.selected_source_ids()
    }

    pub(crate) fn sources(&self) -> &[ImportSourceState] {
        self.state.sources()
    }

    pub(crate) fn rows(&self) -> &[ImportPreviewRow] {
        self.state.rows()
    }

    pub(crate) fn toggle_source(&mut self, importer_id: &str) {
        self.state.toggle_source(importer_id);
    }

    pub(crate) fn toggle_row(&mut self, record_id: &str) {
        self.state.toggle_row(record_id);
    }

    pub(crate) fn apply_scan_reports(&mut self, reports: Vec<ImportScanReport>) {
        self.state.apply_scan_reports(reports);
    }

    pub(crate) fn apply_preview_records(&mut self, records: Vec<ImportRecord>) {
        self.state.apply_preview_records(records);
    }

    pub(crate) fn mark_saving(&mut self, record_id: &str) {
        self.state.mark_saving(record_id);
    }

    pub(crate) fn mark_saved(&mut self, record_id: &str, connection_id: Option<i64>) {
        self.state.mark_saved(record_id, connection_id);
    }

    pub(crate) fn mark_failed(&mut self, record_id: &str, message: String) {
        self.state.mark_failed(record_id, message);
    }

    pub(crate) fn mark_duplicate(&mut self, record_id: &str, existing_name: String) {
        self.state.mark_duplicate(record_id, existing_name);
    }

    pub(crate) fn draft(&self, record_id: &str) -> Option<EditableImportDraft> {
        self.state.row(record_id).map(|row| row.draft.clone())
    }

    pub(crate) fn batch_save_row_ids(&self) -> Vec<String> {
        self.state
            .rows()
            .iter()
            .filter(|row| row.selected && is_save_candidate(&row.save_status))
            .map(|row| row.record_id().to_string())
            .collect()
    }

    pub(crate) fn next_save_candidate_row_id_after(&self, record_id: &str) -> Option<String> {
        self.state.next_save_candidate_row_id_after(record_id)
    }
}

fn current_platform() -> Platform {
    if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::Macos
    }
}
