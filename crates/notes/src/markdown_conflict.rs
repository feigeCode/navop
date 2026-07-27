use crate::notes_notifications::notify_operation_error;
use crate::{MarkdownViewMode, NotesView, markdown_session::MarkdownSyncState};
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
        if snapshot.source == session.preview.read(cx).source() {
            return;
        }
        if session.preview.read(cx).is_dirty()
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
        if session.preview.read(cx).is_dirty()
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
        let result = match session.state.mode {
            MarkdownViewMode::Source => {
                let source = session.source_editor.read(cx).value().to_string();
                session.store.force_save(&source).map(|_| ())
            }
            MarkdownViewMode::Wysiwyg => session
                .store
                .force_save(session.preview.read(cx).source())
                .map(|_| ()),
        };
        match result {
            Ok(()) => {
                session.state.conflict_resolved();
                session.preview.update(cx, |editor, _| editor.mark_saved());
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
            let source = snapshot.source;
            session.preview.update(cx, |editor, cx| {
                editor
                    .replace_source(source.clone(), window, cx)
                    .map_err(anyhow::Error::from)
            })?;
            session.source_editor.update(cx, |input, cx| {
                input.set_value(source, window, cx);
            });
            Ok(())
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
