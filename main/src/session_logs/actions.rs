use gpui::{AppContext, Context, PathPromptOptions, Window};
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
        let request = DeleteRequest { recording_id, path };
        let view_entity = cx.entity().clone();
        let delete_window_handle = window.window_handle();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let entity = view_entity.clone();
            let window_handle = delete_window_handle;
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
                .on_ok(move |_, _window, cx| {
                    let delete_request = request.clone();
                    // Dialog closes on `true`; dispatch deletion through the view
                    // so a slow filesystem removal does not block the UI.
                    _ = cx.update_window(window_handle, |_, window, cx| {
                        _ = entity.update(cx, |this, cx| {
                            this.delete_log(delete_request, window, cx);
                        });
                    });
                    true
                })
        });
    }

    fn delete_log(&mut self, request: DeleteRequest, window: &mut Window, cx: &mut Context<Self>) {
        if self.deleting {
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
        self.begin_delete();
        cx.notify();
        let delete_request = request.clone();
        let delete_task = cx.background_spawn(async move {
            delete_session_log_file(&delete_request.path).map_err(|error| error.to_string())?;
            if let Ok(mut favorites) = load_session_log_favorites(&directory) {
                favorites.set(&delete_request.recording_id, false);
                // Favorite cleanup is best-effort: the log file itself is the
                // primary artifact, and a stale favorite ID is harmless.
                _ = save_session_log_favorites(&directory, &favorites);
            }
            Ok(())
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = delete_task.await;
            _ = cx.update_window(window_handle, |_, window, cx| {
                _ = this.update(cx, |this, cx| {
                    this.finish_delete(request, result, window, cx);
                });
            });
        })
        .detach();
    }

    fn finish_delete(
        &mut self,
        request: DeleteRequest,
        result: Result<(), String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_delete_state();
        match result {
            Ok(()) => {
                self.catalog.entries.retain(|entry| {
                    entry.path != request.path
                        && entry.header.navop.recording_id != request.recording_id
                });
                self.favorites.set(&request.recording_id, false);
                window.push_notification(
                    Notification::success(t!("SessionLogs.delete_success").to_string())
                        .autohide(true),
                    cx,
                );
            }
            Err(error) => show_error(
                t!("SessionLogs.delete_failed", error = error).to_string(),
                window,
                cx,
            ),
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
}
