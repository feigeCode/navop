use crate::markdown_file_store::MarkdownFileStore;
use crate::markdown_mode::{
    editor_view_mode, focus_markdown_editor, markdown_view_mode, switch_markdown_mode,
};
use crate::markdown_session::{MarkdownSession, MarkdownSessionState, MarkdownSyncState};
use crate::notes_notifications::notify_operation_error;
use crate::path_policy::remap_path;
use crate::{DocumentDescriptor, MarkdownSaveMode, MarkdownViewMode, NotesView};
use gpui::{AppContext, AsyncApp, Context, Window};
use markdown_editor::{MarkdownEditor, MarkdownEditorEvent, ViewMode};
use std::time::Duration;

const MARKDOWN_SAVE_INTERVAL: Duration = Duration::from_secs(2);

impl NotesView {
    pub(crate) fn open_markdown_document(
        &mut self,
        descriptor: DocumentDescriptor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let document_id = descriptor.document_id.clone();
        if !self.markdown_sessions.contains_key(&document_id) {
            let mode = self
                .tree
                .markdown_view_modes
                .get(&document_id)
                .copied()
                .unwrap_or_default();
            self.tree
                .markdown_view_modes
                .entry(document_id.clone())
                .or_insert(mode);

            let store = MarkdownFileStore::new(descriptor.absolute_path.clone());
            let snapshot = store.load()?;
            let host_services = markdown_editor::markdown_editor_host_services(
                crate::markdown_renderer::markdown_editor_theme(self.resolved_editor_theme(cx)),
                crate::markdown_renderer::block_render_provider(cx),
            );
            let editor = cx.new(move |cx| {
                let mut editor = MarkdownEditor::from_markdown_embedded_with_host(
                    cx,
                    snapshot.source,
                    host_services,
                );
                editor.set_view_mode(editor_view_mode(mode), cx);
                editor
            });
            let editor_revision = editor.read(cx).revision();
            let editor_subscription =
                subscribe_markdown_changes(&editor, document_id.clone(), window, cx);
            let file_watcher = crate::markdown_watcher::watch_markdown_file(
                descriptor.absolute_path.clone(),
                document_id.clone(),
                window,
                cx,
            )?;

            self.markdown_sessions.insert(
                document_id.clone(),
                MarkdownSession {
                    relative_path: descriptor.relative_path,
                    store,
                    editor,
                    state: MarkdownSessionState::with_mode_and_revision(mode, editor_revision),
                    _subscriptions: vec![editor_subscription],
                    _file_watcher: Some(file_watcher),
                },
            );
            if let Some(storage) = self.storage.as_ref() {
                storage.save_state(&self.tree.to_ui_state())?;
            }
        }

        self.active_document_id = Some(document_id.clone());
        if let Some(session) = self.markdown_sessions.get(&document_id) {
            focus_markdown_editor(session, window, cx);
        }
        Ok(())
    }

    pub(crate) fn markdown_document_changed(
        &mut self,
        document_id: &str,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let save_mode = self.tree.markdown_save_mode;
        let (schedule_epoch, sync_state_changed) = {
            let Some(session) = self.markdown_sessions.get_mut(document_id) else {
                return;
            };
            let sync_state_changed = !matches!(session.state.sync_state, MarkdownSyncState::Dirty);
            if !session.state.document_changed(revision) {
                return;
            }
            (
                session.state.save_mode_changed(save_mode),
                sync_state_changed,
            )
        };
        if let Some(epoch) = schedule_epoch {
            self.schedule_markdown_auto_save(document_id.to_owned(), epoch, window, cx);
        }
        if sync_state_changed {
            cx.notify();
        }
    }

    pub(crate) fn schedule_markdown_auto_save(
        &self,
        document_id: String,
        epoch: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let window_handle = window.window_handle();
        let executor = cx.background_executor().clone();
        let task = cx.background_spawn(async move {
            executor.timer(MARKDOWN_SAVE_INTERVAL).await;
        });
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            task.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = weak.update(cx, |view, cx| {
                    view.fire_markdown_auto_save(&document_id, epoch, window, cx)
                });
            });
        })
        .detach();
    }

    fn fire_markdown_auto_save(
        &mut self,
        document_id: &str,
        epoch: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((generation, source, store)) = self
            .markdown_sessions
            .get_mut(document_id)
            .and_then(|session| {
                let generation = session.state.begin_scheduled_save(epoch)?;
                Some((
                    generation,
                    session.editor.read(cx).markdown(cx),
                    session.store.clone(),
                ))
            })
        else {
            return;
        };
        self.start_markdown_save(
            document_id.to_owned(),
            generation,
            source,
            store,
            window,
            cx,
        );
        cx.notify();
    }

    pub(crate) fn save_markdown_document(
        &mut self,
        document_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((generation, source, store)) = self
            .markdown_sessions
            .get_mut(document_id)
            .and_then(|session| {
                let generation = session.state.begin_manual_save()?;
                Some((
                    generation,
                    session.editor.read(cx).markdown(cx),
                    session.store.clone(),
                ))
            })
        else {
            return;
        };
        self.start_markdown_save(
            document_id.to_owned(),
            generation,
            source,
            store,
            window,
            cx,
        );
        cx.notify();
    }

    pub(crate) fn save_active_markdown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(document_id) = self.active_document_id.clone() else {
            return;
        };
        self.save_markdown_document(&document_id, window, cx);
    }

    pub(crate) fn set_markdown_save_mode(
        &mut self,
        mode: MarkdownSaveMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tree.markdown_save_mode == mode {
            return;
        }
        self.tree.markdown_save_mode = mode;
        let scheduled = self
            .markdown_sessions
            .iter_mut()
            .filter_map(|(document_id, session)| {
                session
                    .state
                    .save_mode_changed(mode)
                    .map(|epoch| (document_id.clone(), epoch))
            })
            .collect::<Vec<_>>();
        if let Some(storage) = self.storage.as_ref()
            && let Err(error) = storage.save_state(&self.tree.to_ui_state())
        {
            notify_operation_error(window, cx, error);
        }
        for (document_id, epoch) in scheduled {
            self.schedule_markdown_auto_save(document_id, epoch, window, cx);
        }
        cx.notify();
    }

    fn start_markdown_save(
        &mut self,
        document_id: String,
        generation: u64,
        source: String,
        store: crate::markdown_file_store::MarkdownFileStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let window_handle = window.window_handle();
        let task = cx.background_spawn(async move { store.save(&source) });
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = weak.update(cx, |view, cx| {
                    view.finish_markdown_save(&document_id, generation, result, window, cx)
                });
            });
        })
        .detach();
    }

    pub(crate) fn toggle_markdown_mode(
        &mut self,
        document_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mode) = self.switch_markdown_session(&document_id, window, cx) else {
            return;
        };
        self.tree.markdown_view_modes.insert(document_id, mode);
        if let Some(storage) = self.storage.as_ref()
            && let Err(error) = storage.save_state(&self.tree.to_ui_state())
        {
            notify_operation_error(window, cx, error);
        }
        cx.notify();
    }

    pub(crate) fn markdown_editor_view_mode_changed(
        &mut self,
        document_id: &str,
        mode: ViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.markdown_sessions.get_mut(document_id) else {
            return;
        };
        let mode = markdown_view_mode(mode);
        if session.state.mode == mode {
            return;
        }
        session.state.set_mode(mode);
        self.tree
            .markdown_view_modes
            .insert(document_id.to_owned(), mode);
        if let Some(storage) = self.storage.as_ref()
            && let Err(error) = storage.save_state(&self.tree.to_ui_state())
        {
            notify_operation_error(window, cx, error);
        }
        cx.notify();
    }

    fn switch_markdown_session(
        &mut self,
        document_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<MarkdownViewMode> {
        let session = self.markdown_sessions.get_mut(document_id)?;
        let mode = match session.state.mode {
            MarkdownViewMode::Source => MarkdownViewMode::Wysiwyg,
            MarkdownViewMode::Wysiwyg => MarkdownViewMode::Source,
        };
        switch_markdown_mode(session, mode, window, cx);
        Some(mode)
    }

    pub(crate) fn remap_markdown_sessions(
        &mut self,
        old: &std::path::Path,
        new: &std::path::Path,
    ) -> anyhow::Result<()> {
        let updates = self
            .markdown_sessions
            .iter()
            .filter(|(_, session)| session.relative_path.starts_with(old))
            .map(|(id, session)| (id.clone(), remap_path(&session.relative_path, old, new)))
            .collect::<Vec<_>>();
        for (id, relative_path) in updates {
            let absolute_path = self.storage()?.descriptor(&relative_path)?.absolute_path;
            if let Some(session) = self.markdown_sessions.get_mut(&id) {
                session.store.set_path(absolute_path)?;
                session.relative_path = relative_path;
            }
        }
        Ok(())
    }

    pub(crate) fn markdown_has_blocking_state(&self) -> bool {
        self.markdown_sessions
            .values()
            .any(|session| session.state.has_unpersisted_changes())
    }

    pub(crate) fn remove_markdown_sessions_under(&mut self, path: &std::path::Path) {
        let removed = self
            .markdown_sessions
            .iter()
            .filter(|(_, session)| session.relative_path.starts_with(path))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        self.markdown_sessions
            .retain(|id, _| !removed.iter().any(|removed| removed == id));
        self.tree
            .markdown_view_modes
            .retain(|id, _| !removed.iter().any(|removed| removed == id));
    }
}

fn subscribe_markdown_changes(
    editor: &gpui::Entity<MarkdownEditor>,
    document_id: String,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> gpui::Subscription {
    cx.subscribe_in(
        editor,
        window,
        move |view, _, event: &MarkdownEditorEvent, window, cx| match event {
            MarkdownEditorEvent::Changed { revision } => {
                view.markdown_document_changed(&document_id, *revision, window, cx);
            }
            MarkdownEditorEvent::ViewModeChanged { mode } => {
                view.markdown_editor_view_mode_changed(&document_id, *mode, window, cx);
            }
        },
    )
}
