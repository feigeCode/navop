use gpui::{App, AppContext, Context, PathPromptOptions, Window};
use gpui_component::{
    WindowExt, button::ButtonVariant, dialog::DialogButtonProps, notification::Notification,
};
use one_core::settings::AppSettings;
use rust_i18n::t;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use terminal::recording::{
    RecordingFileLimits, SessionLogFavorites, export_recording_text, load_session_log_favorites,
    save_session_log_favorites,
};

use super::{SessionLogsPage, model::exported_text_base_name};

pub(super) struct FavoriteChange {
    pub(super) recording_id: String,
    pub(super) favorite: bool,
}

struct FavoriteSaveResult {
    change: FavoriteChange,
    favorites: SessionLogFavorites,
    result: Result<(), String>,
}

#[derive(Clone)]
struct DeleteRequest {
    items: Vec<DeleteItem>,
}

#[derive(Clone)]
struct DeleteItem {
    recording_id: String,
    path: PathBuf,
}

impl SessionLogsPage {
    pub(super) fn request_delete(
        &mut self,
        recording_id: String,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.deleting {
            return;
        }
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let request = DeleteRequest {
            items: vec![DeleteItem { recording_id, path }],
        };
        let view_entity = cx.entity().clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let entity = view_entity.clone();
            let request = request.clone();
            alert
                .title(t!("SessionLogs.delete_title").to_string())
                .description(
                    t!("SessionLogs.delete_description", name = file_name.clone()).to_string(),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.delete").to_string())
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text(t!("Common.cancel").to_string())
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let delete_request = request.clone();
                    // The on_ok callback runs inside this window's update;
                    // re-entering `update_window` for the same window would
                    // fail silently, so update the view entity directly. The
                    // actual file removal stays on a background task so it
                    // does not block the UI.
                    entity.update(cx, |this, cx| {
                        this.delete_log(delete_request, window, cx);
                    });
                    true
                })
        });
    }

    pub(super) fn request_delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.deleting {
            return;
        }
        let items = self
            .catalog
            .entries
            .iter()
            .filter(|entry| self.selected_ids.contains(&entry.header.navop.recording_id))
            .map(|entry| DeleteItem {
                recording_id: entry.header.navop.recording_id.clone(),
                path: entry.path.clone(),
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            return;
        }
        let count = items.len();
        let request = DeleteRequest { items };
        let entity = cx.entity().clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let entity = entity.clone();
            let request = request.clone();
            alert
                .title(t!("SessionLogs.delete_title").to_string())
                .description(
                    t!("SessionLogs.delete_selected_description", count = count).to_string(),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.delete").to_string())
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text(t!("Common.cancel").to_string())
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    entity.update(cx, |this, cx| this.delete_logs(request.clone(), window, cx));
                    true
                })
        });
    }

    fn delete_log(&mut self, request: DeleteRequest, window: &mut Window, cx: &mut Context<Self>) {
        if self.deleting {
            return;
        }
        if self.directory.is_none() {
            show_error(
                t!("SessionLogs.data_directory_unavailable").to_string(),
                window,
                cx,
            );
            return;
        };
        self.delete_logs(request, window, cx);
    }

    fn delete_logs(&mut self, request: DeleteRequest, window: &mut Window, cx: &mut Context<Self>) {
        if self.deleting {
            return;
        }
        let Some(directory) = self.directory.clone() else {
            return;
        };
        self.begin_delete();
        cx.notify();
        let delete_task = cx.background_spawn(async move {
            let results = delete_session_log_files(request.items);
            if let Ok(mut favorites) = load_session_log_favorites(&directory) {
                for (item, result) in &results {
                    if result.is_ok() {
                        favorites.set(&item.recording_id, false);
                    }
                }
                _ = save_session_log_favorites(&directory, &favorites);
            }
            results
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = delete_task.await;
            _ = cx.update_window(window_handle, |_, window, cx| {
                _ = this.update(cx, |this, cx| {
                    this.finish_delete(result, window, cx);
                });
            });
        })
        .detach();
    }

    fn finish_delete(
        &mut self,
        result: Vec<(DeleteItem, Result<(), String>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_delete_state();
        let successful = result
            .iter()
            .filter_map(|(item, result)| result.is_ok().then_some(item))
            .collect::<Vec<_>>();
        for item in &successful {
            self.catalog.entries.retain(|entry| {
                entry.path != item.path && entry.header.navop.recording_id != item.recording_id
            });
            self.favorites.set(&item.recording_id, false);
            self.selected_ids.remove(&item.recording_id);
        }
        if result.iter().all(|(_, result)| result.is_ok()) {
            window.push_notification(
                Notification::success(
                    t!("SessionLogs.delete_success_count", count = successful.len()).to_string(),
                )
                .autohide(true),
                cx,
            );
        } else {
            let failures = result
                .iter()
                .filter_map(|(item, result)| {
                    result
                        .as_ref()
                        .err()
                        .map(|error| format!("{}: {error}", item.path.display()))
                })
                .collect::<Vec<_>>()
                .join(", ");
            show_error(
                t!("SessionLogs.delete_failed", error = failures).to_string(),
                window,
                cx,
            );
        }
        cx.notify();
        self.refresh(cx);
    }
    pub(super) fn toggle_favorite(
        &mut self,
        change: FavoriteChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.favorite_saving {
            return;
        }
        let Some(directory) = self.directory.clone() else {
            show_error(
                t!("SessionLogs.data_directory_unavailable").to_string(),
                window,
                cx,
            );
            return;
        };
        let mut favorites = self.favorites.clone();
        favorites.set(change.recording_id.clone(), change.favorite);
        self.begin_favorite_save();
        cx.notify();
        let save_task = cx.background_spawn({
            let favorites = favorites.clone();
            async move {
                save_session_log_favorites(&directory, &favorites)
                    .map_err(|error| error.to_string())
            }
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = save_task.await;
            let outcome = FavoriteSaveResult {
                change,
                favorites,
                result,
            };
            _ = cx.update_window(window_handle, |_, window, cx| {
                _ = this.update(cx, |this, cx| {
                    this.finish_favorite_save(outcome, window, cx);
                });
            });
        })
        .detach();
    }

    fn finish_favorite_save(
        &mut self,
        outcome: FavoriteSaveResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.favorite_saving = false;
        match outcome.result {
            Ok(()) => {
                self.favorites = outcome.favorites;
                for entry in &mut self.catalog.entries {
                    if entry.header.navop.recording_id == outcome.change.recording_id {
                        entry.favorite = outcome.change.favorite;
                    }
                }
            }
            Err(error) => show_error(
                t!("SessionLogs.favorite_failed", error = error).to_string(),
                window,
                cx,
            ),
        }
        cx.notify();
    }

    pub(super) fn view_log(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        crate::file_open::open_session_log_file(path, window, cx);
    }

    pub(super) fn request_text_export(
        &mut self,
        source_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scrollback_lines = AppSettings::current(cx).terminal_scrollback_lines;
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(t!("SessionLogs.export_select_directory").to_string().into()),
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |_this, cx| {
            let directory = match future.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    show_async_export_error(error.to_string(), window_handle, cx);
                    return;
                }
                Err(error) => {
                    show_async_export_error(error.to_string(), window_handle, cx);
                    return;
                }
            };
            let Some(directory) = directory else { return };
            let export_task = cx.background_spawn(async move {
                export_to_directory(&source_path, &directory, scrollback_lines)
            });
            let result = export_task.await;
            _ = cx.update_window(window_handle, |_, window, cx| {
                show_export_result(result, window, cx);
            });
        })
        .detach();
    }
}

fn show_async_export_error(
    error: String,
    window_handle: gpui::AnyWindowHandle,
    cx: &mut gpui::AsyncApp,
) {
    _ = cx.update_window(window_handle, |_, window, cx| {
        show_error(
            t!("SessionLogs.export_failed", error = error).to_string(),
            window,
            cx,
        );
    });
}

fn export_to_directory(
    source_path: &Path,
    directory: &Path,
    scrollback_lines: usize,
) -> Result<PathBuf, String> {
    let export = export_recording_text(
        source_path,
        RecordingFileLimits::default(),
        scrollback_lines,
    )
    .map_err(|error| error.to_string())?;
    write_exported_text(directory, source_path, &export.text).map_err(|error| error.to_string())
}

fn write_exported_text(directory: &Path, source_path: &Path, text: &str) -> io::Result<PathBuf> {
    let base_name = exported_text_base_name(source_path);
    for suffix in 1_u32..=u32::MAX {
        let candidate = export_candidate(directory, &base_name, suffix);
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                let result = file
                    .write_all(text.as_bytes())
                    .and_then(|()| file.sync_all());
                if let Err(error) = result {
                    drop(file);
                    _ = std::fs::remove_file(&candidate);
                    return Err(error);
                }
                return Ok(candidate);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no available TXT export file name",
    ))
}

fn export_candidate(directory: &Path, base_name: &str, suffix: u32) -> PathBuf {
    if suffix == 1 {
        return directory.join(base_name);
    }
    let stem = base_name.strip_suffix(".txt").unwrap_or(base_name);
    directory.join(format!("{stem}-{suffix}.txt"))
}

fn show_export_result(result: Result<PathBuf, String>, window: &mut Window, cx: &mut gpui::App) {
    match result {
        Ok(path) => window.push_notification(
            Notification::success(
                t!(
                    "SessionLogs.export_success",
                    path = path.to_string_lossy().to_string()
                )
                .to_string(),
            )
            .autohide(true),
            cx,
        ),
        Err(error) => show_error(
            t!("SessionLogs.export_failed", error = error).to_string(),
            window,
            cx,
        ),
    }
}

fn show_error(message: String, window: &mut Window, cx: &mut gpui::App) {
    window.push_notification(Notification::error(message).autohide(true), cx);
}

fn delete_session_log_file(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

fn delete_session_log_files(items: Vec<DeleteItem>) -> Vec<(DeleteItem, Result<(), String>)> {
    items
        .into_iter()
        .map(|item| {
            let result = delete_session_log_file(&item.path).map_err(|error| error.to_string());
            (item, result)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn text_export_never_overwrites_existing_files() {
        let directory = tempdir().unwrap();
        let source = Path::new("session.cast.partial");
        std::fs::write(directory.path().join("session.txt"), "existing").unwrap();

        let output = write_exported_text(directory.path(), source, "new").unwrap();

        assert_eq!(directory.path().join("session-2.txt"), output);
        assert_eq!(
            "existing",
            std::fs::read_to_string(directory.path().join("session.txt")).unwrap()
        );
        assert_eq!("new", std::fs::read_to_string(output).unwrap());
    }

    #[test]
    fn batch_delete_reports_each_file_independently() {
        let directory = tempdir().unwrap();
        let existing = directory.path().join("existing.cast");
        let missing = directory.path().join("missing.cast");
        std::fs::write(&existing, "log").unwrap();

        let results = delete_session_log_files(vec![
            DeleteItem {
                recording_id: "existing".into(),
                path: existing.clone(),
            },
            DeleteItem {
                recording_id: "missing".into(),
                path: missing,
            },
        ]);

        assert!(results[0].1.is_ok());
        assert!(results[1].1.is_err());
        assert!(!existing.exists());
    }
}
