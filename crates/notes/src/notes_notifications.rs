use crate::markdown_file_store::MarkdownSaveOutcome;
use crate::{MarkdownSaveMode, NotesView, NotesViewEvent};
use gpui::{Context, Window};
use gpui_component::{WindowExt, notification::Notification};
use rust_i18n::t;

pub(crate) fn notify_operation_error<T>(
    window: &mut Window,
    cx: &mut Context<T>,
    error: impl std::fmt::Display,
) {
    notify_error_message(
        window,
        cx,
        t!("Notes.operation_failed", error = format!("{error:#}")),
    );
}

pub(crate) fn notify_error_message<T>(
    window: &mut Window,
    cx: &mut Context<T>,
    message: impl Into<String>,
) {
    window.push_notification(Notification::error(message.into()).autohide(false), cx);
}

impl NotesView {
    pub(crate) fn finish_markdown_save(
        &mut self,
        document_id: &str,
        generation: u64,
        result: anyhow::Result<MarkdownSaveOutcome>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let save_mode = self.tree.markdown_save_mode;
        let mut saved_path = None;
        let mut schedule_epoch = None;
        let notification = {
            let Some(session) = self.markdown_sessions.get_mut(document_id) else {
                return;
            };
            match result {
                Ok(MarkdownSaveOutcome::Saved(_)) => {
                    session.state.source_saved(generation);
                    if generation == session.state.generation
                        && matches!(
                            session.state.sync_state,
                            crate::markdown_session::MarkdownSyncState::Clean
                        )
                    {
                        session.preview.update(cx, |editor, _| editor.mark_saved());
                    }
                    saved_path = session.store.path().ok();
                    if save_mode == MarkdownSaveMode::Automatic {
                        schedule_epoch = session.state.save_mode_changed(save_mode);
                    }
                    None
                }
                Ok(MarkdownSaveOutcome::Conflict(_)) => {
                    session.state.conflict();
                    Some(Notification::warning(
                        t!("Notes.markdown_external_change").to_string(),
                    ))
                }
                Err(error) => {
                    session
                        .state
                        .source_save_failed(generation, error.to_string());
                    Some(Notification::error(
                        t!("Notes.markdown_save_failed", error = error.to_string()).to_string(),
                    ))
                }
            }
        };
        if let Some(epoch) = schedule_epoch {
            self.schedule_markdown_auto_save(document_id.to_owned(), epoch, window, cx);
        }
        if let Some(notification) = notification {
            window.push_notification(notification.autohide(false), cx);
        }
        if let Some(path) = saved_path {
            cx.emit(NotesViewEvent::FileSaved(path));
        }
        cx.notify();
    }
}
