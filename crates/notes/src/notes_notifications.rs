use crate::markdown_file_store::MarkdownSaveOutcome;
use crate::{NotesView, NotesViewEvent};
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
        result: anyhow::Result<Option<MarkdownSaveOutcome>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.markdown_sessions.get_mut(document_id) else {
            return;
        };
        let mut saved_path = None;
        let notification = match result {
            Ok(Some(MarkdownSaveOutcome::Saved(_))) => {
                let is_current = generation == session.state.generation;
                session.state.source_saved(generation);
                if is_current {
                    session.preview.update(cx, |editor, _| editor.mark_saved());
                }
                saved_path = session.store.path().ok();
                None
            }
            Ok(Some(MarkdownSaveOutcome::Conflict(_))) => {
                session.state.conflict();
                Some(Notification::warning(
                    t!("Notes.markdown_external_change").to_string(),
                ))
            }
            Ok(None) => None,
            Err(error) => {
                session
                    .state
                    .source_save_failed(generation, error.to_string());
                Some(Notification::error(
                    t!("Notes.markdown_save_failed", error = error.to_string()).to_string(),
                ))
            }
        };
        if let Some(notification) = notification {
            window.push_notification(notification.autohide(false), cx);
        }
        if let Some(path) = saved_path {
            cx.emit(NotesViewEvent::FileSaved(path));
        }
        cx.notify();
    }
}
