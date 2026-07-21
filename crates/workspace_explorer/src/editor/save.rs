use super::{DocumentKey, WorkspaceEditor, WorkspaceEditorEvent};
use crate::file_system::save_file;
use gpui::{AppContext as _, AsyncApp, Context, WeakEntity, Window};
use gpui_component::{WindowExt as _, notification::Notification};
use rust_i18n::t;
use std::path::PathBuf;

struct SaveOutcome {
    path: PathBuf,
    text: String,
    close_after_save: bool,
}

impl WorkspaceEditor {
    pub(super) fn save(
        &mut self,
        close_after_save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(attempt) = self.prepare_save(close_after_save, cx) else {
            return;
        };
        let task_path = attempt.outcome.path.clone();
        let task_text = attempt.outcome.text.clone();
        let task = cx.background_spawn(async move { save_file(&task_path, &task_text) });
        let entity = cx.entity().downgrade();
        let window_handle = window.window_handle();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let Some(entity) = entity.upgrade() else {
                    return;
                };
                entity.update(cx, |this, cx| match result {
                    Ok(()) => this.apply_saved(attempt, window, cx),
                    Err(error) => {
                        this.apply_save_error(attempt.tab_id, &attempt.key, cx);
                        window.push_notification(Notification::error(error.to_string()), cx);
                    }
                });
            });
        })
        .detach();
    }

    fn prepare_save(
        &mut self,
        close_after_save: bool,
        cx: &mut Context<Self>,
    ) -> Option<SaveAttempt> {
        let index = self.active_tab;
        let tab = self.tabs.get_mut(index)?;
        if tab.read_only || tab.saving {
            return None;
        }
        let DocumentKey::File(path) = &tab.key else {
            return None;
        };
        let editor = tab.editor.clone()?;
        let outcome = SaveOutcome {
            path: path.clone(),
            text: editor.read(cx).text().to_string(),
            close_after_save,
        };
        tab.saving = true;
        tab.status_message = t!("WorkspaceExplorer.status.saving").to_string();
        cx.notify();
        Some(SaveAttempt {
            tab_id: tab.id,
            key: tab.key.clone(),
            outcome,
        })
    }

    fn apply_saved(
        &mut self,
        completion: SaveAttempt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(completion.tab_id, &completion.key) else {
            return;
        };
        let outcome = completion.outcome;
        let tab = &mut self.tabs[index];
        tab.saved_text = outcome.text;
        tab.file_size = tab.saved_text.len();
        tab.saving = false;
        tab.status_message = t!("WorkspaceExplorer.status.saved").to_string();
        cx.emit(WorkspaceEditorEvent::FileSaved(outcome.path));
        if outcome.close_after_save {
            self.close_clean_tab(index, window, cx);
        } else {
            window.push_notification(
                Notification::success(t!("WorkspaceExplorer.notification.saved").to_string()),
                cx,
            );
            cx.notify();
        }
    }

    fn apply_save_error(&mut self, tab_id: u64, key: &DocumentKey, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id, key) else {
            return;
        };
        let tab = &mut self.tabs[index];
        tab.saving = false;
        tab.status_message = t!("WorkspaceExplorer.status.save_failed").to_string();
        cx.notify();
    }
}

struct SaveAttempt {
    tab_id: u64,
    key: DocumentKey,
    outcome: SaveOutcome,
}
