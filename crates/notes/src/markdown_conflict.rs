use crate::notes_notifications::notify_operation_error;
use crate::{MarkdownViewMode, NotesView, markdown_adapter, markdown_session::MarkdownSyncState};
use gpui::{Context, Window};

impl NotesView {
    /// Keep the local changes and overwrite the externally modified file.
    pub(crate) fn resolve_markdown_conflict_keep_local(
        &mut self,
        document_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.markdown_sessions.get_mut(document_id) else {
            return;
        };
        let result = match session.state.mode {
            MarkdownViewMode::Source => {
                let source = session.source_editor.read(cx).value().to_string();
                session.store.force_save(&source).map(|_| ())
            }
            MarkdownViewMode::Wysiwyg => {
                markdown_adapter::export_markdown_bundle(&session.preview, &session.store, cx)
                    .and_then(|markdown| session.store.force_save(&markdown).map(|_| ()))
            }
        };
        match result {
            Ok(()) => {
                session.state.conflict_resolved();
                if session.state.mode == MarkdownViewMode::Wysiwyg {
                    // Clear the editor's failed save state; the fingerprint now
                    // matches the file we just wrote, so this save succeeds.
                    let _ = session.preview.save(cx);
                }
                cx.notify();
            }
            Err(error) => notify_operation_error(window, cx, error),
        }
    }

    /// Discard the local changes and reload the externally modified file.
    pub(crate) fn resolve_markdown_conflict_use_external(
        &mut self,
        document_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.markdown_sessions.get_mut(document_id) else {
            return;
        };
        let result = session.store.load().and_then(|snapshot| {
            session.source_editor.update(cx, |input, cx| {
                input.set_value(snapshot.source.clone(), window, cx);
            });
            session
                .preview
                .reload(cx)
                .map_err(|error| anyhow::anyhow!(error))
        });
        match result {
            Ok(()) => {
                session.state.external_reloaded();
                cx.notify();
            }
            Err(error) => notify_operation_error(window, cx, error),
        }
    }

    /// Ids of documents whose local changes conflict with external
    /// modifications or failed to save.
    pub(crate) fn blocked_markdown_document_ids(&self) -> Vec<String> {
        self.markdown_sessions
            .iter()
            .filter(|(_, session)| {
                matches!(
                    session.state.sync_state,
                    MarkdownSyncState::Conflict | MarkdownSyncState::Failed(_)
                )
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Discard local changes of every blocked session so the view can close.
    pub(crate) fn discard_blocked_markdown_sessions(&mut self, cx: &mut Context<Self>) {
        for id in self.blocked_markdown_document_ids() {
            if let Some(session) = self.markdown_sessions.get_mut(&id) {
                session.state.external_reloaded();
            }
        }
        cx.notify();
    }
}
