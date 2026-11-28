use std::collections::{BTreeMap, BTreeSet};

use connection_import_protocol::{
    ImportRecord, ImportRecordKind, ImportScanReport, ImportWarning, ImporterAvailability,
    ImporterDescriptor, Platform,
};
use extension_runtime::connection_import_provider::ImportPreviewError;

use super::connection_import_defaults::ImportConnectionDefaults;
use super::connection_import_draft::{
    EditableImportDraft, ImportDraftKind, normalized_ssh_group_path, normalized_workspace_path,
};

pub(crate) struct ImportCenterState {
    sources: Vec<ImportSourceState>,
    rows: Vec<ImportPreviewRow>,
    defaults: ImportConnectionDefaults,
    /// 最近一次单点勾选的预览行。
    ///
    /// Shift 区间选择需要一个锚点；没有锚点（或锚点已被重新扫描冲掉）时
    /// 退化成普通单点取反，不会把整段行误勾上。
    selection_anchor: Option<String>,
}

pub(crate) struct ImportSourceState {
    pub(crate) descriptor: ImporterDescriptor,
    pub(crate) selected: bool,
    pub(crate) selectable: bool,
    pub(crate) availability: ImporterAvailability,
    pub(crate) scan_error: Option<String>,
    pub(crate) preview_error: Option<String>,
    pub(crate) discovered_workspace_paths: Vec<String>,
    /// 导入器在扫描阶段报的结构性告警（例如「跳过了多少条非 SSH/Telnet 会话」）。
    ///
    /// 之前这些告警被直接丢弃，用户只能看到「少了 78 条」却看不到原因；
    /// 现在原样保留，供来源行展示并写进日志。
    pub(crate) scan_warnings: Vec<ImportWarning>,
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
            defaults: ImportConnectionDefaults::default(),
            selection_anchor: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_tests() -> Self {
        Self {
            sources: Vec::new(),
            rows: Vec::new(),
            defaults: ImportConnectionDefaults::default(),
            selection_anchor: None,
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

    pub(crate) fn defaults(&self) -> ImportConnectionDefaults {
        self.defaults
    }

    /// 更新导入默认项，并同步到已经解析出来的草稿上（用户可能在预览之后才改勾选）。
    pub(crate) fn set_defaults(&mut self, defaults: ImportConnectionDefaults) {
        self.defaults = defaults;
        for row in &mut self.rows {
            row.draft.set_defaults(defaults);
        }
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
                source.scan_warnings = report.warnings;
                for warning in &source.scan_warnings {
                    // 扫描告警是「为什么少了连接」的唯一线索，必须进日志文件。
                    tracing::warn!(
                        target: "connection_import",
                        importer = %report.importer_id,
                        code = %warning.code,
                        "{}",
                        warning.message
                    );
                }
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
                ImportRecordKind::Telnet => record
                    .telnet
                    .as_ref()
                    .and_then(|telnet| telnet.group_path.as_deref())
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
                draft: EditableImportDraft::new(record).with_defaults(self.defaults),
                selected: true,
                save_status: ImportRowSaveStatus::Pending,
            })
            .collect();
        // 重新扫描后旧的行已经不存在，锚点必须失效。
        self.selection_anchor = None;
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
        self.toggle_row_with_shift(record_id, false);
    }

    /// 勾选/取消单条预览行；`extend` 为 true 时（Shift 点击）从锚点到当前行做区间选择。
    pub(crate) fn toggle_row_with_shift(&mut self, record_id: &str, extend: bool) {
        let Some(index) = self
            .rows
            .iter()
            .position(|row| row.record_id() == record_id)
        else {
            return;
        };
        if !is_save_candidate(&self.rows[index].save_status) {
            return;
        }

        if !extend {
            self.rows[index].selected = !self.rows[index].selected;
            self.selection_anchor = Some(record_id.to_string());
            return;
        }

        // 锚点缺失或行已变化时退化成单点取反，避免区间落空。
        let anchor_index = self
            .selection_anchor
            .as_deref()
            .and_then(|anchor| self.rows.iter().position(|row| row.record_id() == anchor));
        let Some(anchor_index) = anchor_index else {
            self.rows[index].selected = !self.rows[index].selected;
            self.selection_anchor = Some(record_id.to_string());
            return;
        };

        // 以目标行当前状态的反面作为整段的目标状态：勾选区间、或取消区间。
        let target = !self.rows[index].selected;
        let (start, end) = if anchor_index <= index {
            (anchor_index, index)
        } else {
            (index, anchor_index)
        };
        for row in &mut self.rows[start..=end] {
            // 已保存/正在保存的行即使落在区间里也保持原样，避免出现能再次保存的假象。
            if is_save_candidate(&row.save_status) {
                row.selected = target;
            }
        }
    }

    /// 当前可保存的预览行数量（已保存、正在保存、重复跳过的不算）。
    pub(crate) fn candidate_row_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| is_save_candidate(&row.save_status))
            .count()
    }

    pub(crate) fn selected_candidate_row_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| is_save_candidate(&row.save_status) && row.selected)
            .count()
    }

    /// 可保存的行是否已经全部勾选，供「全选」复选框显示三态。
    pub(crate) fn all_candidates_selected(&self) -> bool {
        let total = self.candidate_row_count();
        total > 0 && self.selected_candidate_row_count() == total
    }

    /// 全选 / 一键取消：全部可保存行统一设为 `selected`。
    pub(crate) fn set_all_candidates_selected(&mut self, selected: bool) {
        for row in &mut self.rows {
            if is_save_candidate(&row.save_status) {
                row.selected = selected;
            }
        }
        self.selection_anchor = None;
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
            scan_warnings: Vec::new(),
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
