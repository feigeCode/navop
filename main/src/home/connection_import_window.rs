use std::path::PathBuf;

use connection_import_protocol::ImporterDescriptor;
use gpui::{
    AnyWindowHandle, App, AppContext, AsyncApp, Context, Entity, FocusHandle, Focusable,
    PathPromptOptions, WeakEntity, Window,
};
use one_core::gpui_tokio::Tokio;
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use rust_i18n::t;

use super::connection_import_actions::{
    ImportSaveResult, preview_import_records, preview_import_records_from_files, save_import_draft,
    scan_import_sources,
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
                    Err(error) => {
                        Err(t!("Home.ConnectionImport.scan_task_failed", error = error).to_string())
                    }
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

    fn import_source_file(
        &mut self,
        importer_id: String,
        prompt: String,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.scanning {
            return;
        }
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(prompt.into()),
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(paths))) = future.await else {
                return;
            };
            if paths.is_empty() {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.scanning = true;
                this.status_message = None;
                cx.notify();
            });
            let result = {
                let id = importer_id.clone();
                let selected_paths: Vec<PathBuf> = paths.into_iter().collect();
                let task = Tokio::spawn(cx, async move {
                    preview_import_records_from_files(id, selected_paths, true).await
                });
                match task.await {
                    Ok(result) => result,
                    Err(error) => Err(t!(
                        "Home.ConnectionImport.file_parse_task_failed",
                        error = error
                    )
                    .to_string()),
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.scanning = false;
                match result {
                    Ok(records) => {
                        let is_empty = records.is_empty();
                        this.model.apply_preview_records(records);
                        this.status_message = is_empty.then(|| {
                            t!("Home.ConnectionImport.no_importable_connections").to_string()
                        });
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
        return (
            Vec::new(),
            Some(t!("Home.ConnectionImport.extension_directory_unavailable").to_string()),
        );
    };
    let composite_root =
        root.join(extension_runtime::extension::ExtensionKind::Composite.dir_name());
    match extension_runtime::connection_import_provider::list_manifest_connection_importers(
        &composite_root,
    ) {
        Ok(importers) if importers.is_empty() => (
            Vec::new(),
            Some(t!("Home.ConnectionImport.no_import_extensions").to_string()),
        ),
        Ok(importers) => (
            importers
                .into_iter()
                .map(|importer| importer.descriptor)
                .collect(),
            None,
        ),
        Err(error) => (
            Vec::new(),
            Some(
                t!(
                    "Home.ConnectionImport.load_extensions_failed",
                    error = error
                )
                .to_string(),
            ),
        ),
    }
}

fn is_save_candidate(status: &ImportRowSaveStatus) -> bool {
    matches!(
        status,
        ImportRowSaveStatus::Pending | ImportRowSaveStatus::Failed { .. }
    )
}
