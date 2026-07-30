use crate::notes_notifications::notify_operation_error;
use crate::{NotesView, markdown_session::MarkdownSyncState};
use gpui::{Context, Window};

impl NotesView {
    pub(crate) fn markdown_file_changed_on_disk(
        &mut self,
        document_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.markdown_sessions.get_mut(document_id) else {
            return;
        };
        let snapshot = match session.store.load_external_change() {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return,
            Err(_) => {
                session.state.conflict();
                cx.notify();
                return;
            }
        };
        if snapshot.source == session.editor.read(cx).markdown(cx) {
            return;
        }
        if session.editor.read(cx).is_dirty()
            || !matches!(session.state.sync_state, MarkdownSyncState::Clean)
        {
            session.state.conflict();
            cx.notify();
            return;
        }
        self.resolve_markdown_conflict_use_external(document_id, window, cx);
    }

    pub fn reload_active_markdown_from_disk(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(document_id) = self.active_document_id.clone() else {
            return;
        };
        let Some(session) = self.markdown_sessions.get_mut(&document_id) else {
            return;
        };
        if session.editor.read(cx).is_dirty()
            || !matches!(session.state.sync_state, MarkdownSyncState::Clean)
        {
            session.state.conflict();
            cx.notify();
            return;
        }
        self.resolve_markdown_conflict_use_external(&document_id, window, cx);
    }

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
        let source = session.editor.read(cx).markdown(cx);
        let result = session.store.force_save(&source).map(|_| ());
        match result {
            Ok(()) => {
                session.state.conflict_resolved();
                session
                    .editor
                    .update(cx, |editor, cx| editor.mark_saved(cx));
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
            let revision = session.editor.update(cx, |editor, cx| {
                editor.reload_markdown(snapshot.source, cx);
                let revision = editor.revision();
                revision
            });
            session.state.external_reloaded(revision);
            Ok(())
        });
        match result {
            Ok(()) => {
                cx.notify();
            }
            Err(error) => notify_operation_error(window, cx, error),
        }
    }
}
