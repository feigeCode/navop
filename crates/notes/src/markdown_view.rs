use crate::markdown_file_store::MarkdownFileStore;
use crate::markdown_mode::{switch_to_source, switch_to_wysiwyg};
use crate::markdown_renderer::{block_render_provider, markdown_editor_theme};
use crate::markdown_session::{MarkdownSession, MarkdownSessionState, MarkdownSyncState};
use crate::markdown_source::{create_source_editor, subscribe_source_changes};
use crate::notes_notifications::notify_operation_error;
use crate::path_policy::remap_path;
use crate::{DocumentDescriptor, MarkdownSaveMode, MarkdownViewMode, NotesView};
use gpui::{AppContext, AsyncApp, Context, Window};
use markdown_editor::{MarkdownEditor, MarkdownEditorEvent};
use markdown_source::SourceMarkdownDocument;
use std::time::Duration;

const MARKDOWN_SAVE_INTERVAL: Duration = Duration::from_secs(2);

impl NotesView {
    pub(crate) fn apply_source_mode_history(
        &mut self,
        undo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(document_id) = self.active_document_id.as_ref() else {
            return;
        };
        let Some(session) = self.markdown_sessions.get(document_id) else {
            return;
        };
        let selection = session.preview.update(cx, |editor, cx| {
            if undo {
                editor.undo_source_mode(window, cx)
            } else {
                editor.redo_source_mode(window, cx)
            }
        });
        let Ok(Some(selection)) = selection else {
            return;
        };
        let source = session.preview.read(cx).source().to_owned();
        session.source_editor.update(cx, |input, cx| {
            input.set_value(source, window, cx);
            input.set_selected_range(
                selection.anchor.min(selection.head)..selection.anchor.max(selection.head),
                selection.anchor > selection.head,
                window,
                cx,
            );
        });
    }

    pub(crate) fn open_markdown_document(
        &mut self,
        descriptor: DocumentDescriptor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let document_id = descriptor.document_id.clone();
        if !self.markdown_sessions.contains_key(&document_id) {
            let mode = MarkdownViewMode::Wysiwyg;
            self.tree
                .markdown_view_modes
                .insert(document_id.clone(), mode);
            let store = MarkdownFileStore::new(descriptor.absolute_path.clone());
            let snapshot = store.load()?;
            let document = SourceMarkdownDocument::parse(snapshot.source.clone())?;
            let source_editor = create_source_editor(&snapshot.source, window, cx);
            let theme = markdown_editor_theme(self.resolved_editor_theme(cx));
            let preview = cx.new({
                let window = &mut *window;
                move |cx| {
                    let mut editor = MarkdownEditor::from_document(document, theme, window, cx);
                    editor.set_block_render_provider(block_render_provider(cx), cx);
                    editor
                }
            });
            let preview_subscription =
                subscribe_markdown_changes(&preview, document_id.clone(), window, cx);
            let source_subscription =
                subscribe_source_changes(&source_editor, &preview, window, cx);
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
                    source_editor,
                    preview,
                    state: MarkdownSessionState::with_mode(mode),
                    _subscriptions: vec![preview_subscription, source_subscription],
                    _file_watcher: Some(file_watcher),
                },
            );
            if let Some(storage) = self.storage.as_ref() {
                storage.save_state(&self.tree.to_ui_state())?;
            }
        }
        self.active_document_id = Some(document_id.clone());
        if let Some(session) = self.markdown_sessions.get(&document_id)
            && session.state.mode == MarkdownViewMode::Source
        {
            let input = session.source_editor.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |input, cx| input.focus(window, cx))
            });
        }
        Ok(())
    }

    pub(crate) fn markdown_source_changed(
        &mut self,
        document_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let save_mode = self.tree.markdown_save_mode;
        let (schedule_epoch, sync_state_changed) = {
            let Some(session) = self.markdown_sessions.get_mut(document_id) else {
                return;
            };
            let sync_state_changed =
                !matches!(session.state.sync_state, MarkdownSyncState::SourceDirty);
            session.state.source_changed();
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
                let generation = session.state.begin_scheduled_source_save(epoch)?;
                Some((
                    generation,
                    session.preview.read(cx).source().to_owned(),
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
                let generation = session.state.begin_manual_source_save()?;
                Some((
                    generation,
                    session.preview.read(cx).source().to_owned(),
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
        let Some((mode, source_commit)) = self.switch_markdown_session(&document_id, window, cx)
        else {
            return;
        };
        if source_commit.is_some() {
            self.markdown_source_changed(&document_id, window, cx);
        }
        self.tree.markdown_view_modes.insert(document_id, mode);
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
    ) -> Option<(MarkdownViewMode, Option<String>)> {
        let session = self.markdown_sessions.get_mut(document_id)?;
        if !session.state.begin_switch() {
            return None;
        }
        let result = match session.state.mode {
            MarkdownViewMode::Source => switch_to_wysiwyg(session, window, cx),
            MarkdownViewMode::Wysiwyg => switch_to_source(session, window, cx),
        };
        match result {
            Ok(source) => Some((session.state.mode, source)),
            Err(error) => {
                session
                    .state
                    .source_save_failed(session.state.generation, error.to_string());
                notify_operation_error(window, cx, error);
                None
            }
        }
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
    preview: &gpui::Entity<MarkdownEditor>,
    document_id: String,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> gpui::Subscription {
    cx.subscribe_in(
        preview,
        window,
        move |view, _, event: &MarkdownEditorEvent, window, cx| {
            let MarkdownEditorEvent::Changed { .. } = event;
            view.markdown_source_changed(&document_id, window, cx);
        },
    )
}
