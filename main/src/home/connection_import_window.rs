use connection_import_protocol::ImporterDescriptor;
use gpui::{
    AnyWindowHandle, App, AppContext, AsyncApp, Context, Entity, FocusHandle, Focusable,
    WeakEntity, Window,
};
use one_core::gpui_tokio::Tokio;
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use rust_i18n::t;

use super::connection_import_actions::{
    ImportSaveResult, preview_import_records, save_import_draft, scan_import_sources,
};
use super::connection_import_model::{ImportRowSaveStatus, previewable_source_ids_after_scan};
use crate::home_tab::HomePage;

mod editor;
mod model;
mod render;

pub(crate) use model::ConnectionImportWindowModel;

pub(crate) struct ConnectionImportWindow {
    parent: Entity<HomePage>,
    focus_handle: FocusHandle,
    model: ConnectionImportWindowModel,
    loading_sources: bool,
    scanning: bool,
    status_message: Option<String>,
}

pub(crate) fn show_connection_import_window(
    parent: Entity<HomePage>,
    parent_window: AnyWindowHandle,
    cx: &mut App,
) {
    open_popup_window(
        PopupWindowOptions::new(t!("Home.import").to_string()).size(1040.0, 720.0),
        move |window, cx| {
            cx.new(|cx| ConnectionImportWindow::new(parent, parent_window, window, cx))
        },
        cx,
    );
}

impl ConnectionImportWindow {
    pub(crate) fn new(
        parent: Entity<HomePage>,
        _parent_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        let (descriptors, status_message) = load_importer_descriptors();
        Self {
            parent,
            focus_handle,
            model: ConnectionImportWindowModel::new(descriptors),
            loading_sources: false,
            scanning: false,
            status_message,
        }
    }

    fn refresh_sources(&mut self, cx: &mut Context<Self>) {
        let (descriptors, status_message) = load_importer_descriptors();
        self.model = ConnectionImportWindowModel::new(descriptors);
        self.status_message = status_message;
        self.loading_sources = false;
        cx.notify();
    }

    fn scan_selected(&mut self, cx: &mut Context<Self>) {
        let importer_ids = self.model.selected_source_ids();
        if importer_ids.is_empty() || self.scanning {
            return;
        }
        self.scanning = true;
        self.status_message = None;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = {
                let ids = importer_ids.clone();
                let task = Tokio::spawn(cx, async move {
                    let reports = scan_import_sources(ids.clone()).await?;
                    let preview_ids = previewable_source_ids_after_scan(&ids, &reports);
                    let records = preview_import_records(preview_ids, true).await?;
                    Ok::<_, String>((reports, records))
                });
                match task.await {
                    Ok(result) => result,
                    Err(error) => Err(format!("导入扫描任务失败: {error}")),
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.scanning = false;
                match result {
                    Ok((reports, records)) => {
                        this.model.apply_scan_reports(reports);
                        this.model.apply_preview_records(records);
                        this.status_message = None;
                    }
                    Err(error) => this.status_message = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_row(&mut self, record_id: String, cx: &mut Context<Self>) {
        let Some(draft) = self.model.draft(&record_id) else {
            return;
        };
        self.model.mark_saving(&record_id);
        match save_import_draft(&draft, cx) {
            Ok(ImportSaveResult::Saved { connection_id }) => {
                self.model.mark_saved(&record_id, connection_id);
            }
            Ok(ImportSaveResult::SkippedDuplicate { existing_name }) => {
                self.model.mark_duplicate(&record_id, existing_name);
            }
            Err(error) => self.model.mark_failed(&record_id, error),
        }
        cx.notify();
    }

    fn save_selected(&mut self, cx: &mut Context<Self>) {
        let row_ids = self.model.batch_save_row_ids();
        for row_id in row_ids {
            self.save_row(row_id, cx);
        }
    }
}

impl Focusable for ConnectionImportWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn load_importer_descriptors() -> (Vec<ImporterDescriptor>, Option<String>) {
    let Some(root) = extension_runtime::extension::extensions_root() else {
        return (Vec::new(), Some("扩展目录不可用".to_string()));
    };
    let composite_root =
        root.join(extension_runtime::extension::ExtensionKind::Composite.dir_name());
    match extension_runtime::connection_import_provider::list_manifest_connection_importers(
        &composite_root,
    ) {
        Ok(importers) if importers.is_empty() => {
            (Vec::new(), Some("未安装连接导入扩展".to_string()))
        }
        Ok(importers) => (
            importers
                .into_iter()
                .map(|importer| importer.descriptor)
                .collect(),
            None,
        ),
        Err(error) => (Vec::new(), Some(format!("加载连接导入扩展失败: {error}"))),
    }
}

fn is_save_candidate(status: &ImportRowSaveStatus) -> bool {
    matches!(
        status,
        ImportRowSaveStatus::Pending | ImportRowSaveStatus::Failed { .. }
    )
}
