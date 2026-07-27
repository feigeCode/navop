use crate::markdown_file_store::MarkdownFileStore;
use crate::markdown_mode::{switch_to_source, switch_to_wysiwyg};
use crate::markdown_renderer::{block_render_provider, markdown_editor_theme};
use crate::markdown_session::{MarkdownSession, MarkdownSessionState, MarkdownSyncState};
use crate::markdown_source::{create_source_editor, subscribe_source_changes};
use crate::notes_notifications::notify_operation_error;
use crate::path_policy::remap_path;
use crate::{DocumentDescriptor, MarkdownViewMode, NotesView};
use gpui::{AppContext, AsyncApp, Context, Window};
use markdown_editor::{MarkdownEditor, MarkdownEditorEvent};
use markdown_source::SourceMarkdownDocument;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

const MARKDOWN_SAVE_DELAY: Duration = Duration::from_millis(700);

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
            let source_editor = create_source_editor(&snapshot.source, window, cx);
            let source_document = Arc::new(std::sync::Mutex::new(SourceMarkdownDocument::parse(
                snapshot.source.clone(),
            )?));
            let theme = markdown_editor_theme(self.resolved_editor_theme(cx));
            let preview = cx.new(|cx| {
                let mut editor = MarkdownEditor::new(snapshot.source.clone(), theme, window, cx)
                    .expect("prevalidated Markdown must initialize the editor");
                editor.set_block_render_provider(block_render_provider(cx), cx);
                editor
            });
            let preview_subscription = subscribe_markdown_changes(
                &preview,
                &source_editor,
                document_id.clone(),
                window,
                cx,
            );
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
                    source_document,
                    save_generation: Default::default(),
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
        source: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.markdown_sessions.get_mut(document_id) else {
            return;
        };
        let generation = session.state.source_changed();
        if let Err(error) = replace_session_source(session, &source) {
            session
                .state
                .source_save_failed(generation, error.to_string());
            notify_operation_error(window, cx, error);
            return;
        }
        session.save_generation.store(generation, Ordering::Release);
        let _ = session.state.begin_source_save(generation);
        let store = session.store.clone();
        let generation_token = session.save_generation.clone();
        let weak = cx.entity().downgrade();
        let window_handle = window.window_handle();
        let id = document_id.to_owned();
        let executor = cx.background_executor().clone();
        let task = cx.background_spawn(async move {
            executor.timer(MARKDOWN_SAVE_DELAY).await;
            if generation_token.load(Ordering::Acquire) != generation {
                return Ok(None);
            }
            store.save(&source).map(Some)
        });
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = weak.update(cx, |view, cx| {
                    view.finish_markdown_save(&id, generation, result, window, cx)
                });
            });
        })
        .detach();
        cx.notify();
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
        if let Some(source) = source_commit {
            self.markdown_source_changed(&document_id, source, window, cx);
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
            .any(|session| !matches!(session.state.sync_state, MarkdownSyncState::Clean))
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

fn replace_session_source(session: &MarkdownSession, source: &str) -> anyhow::Result<()> {
    let mut document = session
        .source_document
        .lock()
        .map_err(|_| anyhow::anyhow!("Markdown source document lock is poisoned"))?;
    *document = document.replace_source(source)?;
    Ok(())
}

fn subscribe_markdown_changes(
    preview: &gpui::Entity<MarkdownEditor>,
    source_editor: &gpui::Entity<gpui_component::input::InputState>,
    document_id: String,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> gpui::Subscription {
    let source_editor = source_editor.clone();
    cx.subscribe_in(
        preview,
        window,
        move |view, _, event: &MarkdownEditorEvent, window, cx| {
            let MarkdownEditorEvent::Changed { source, .. } = event;
            let source_mode = view
                .markdown_sessions
                .get(&document_id)
                .is_some_and(|session| session.state.mode == MarkdownViewMode::Source);
            if !source_mode {
                source_editor.update(cx, |input, cx| {
                    input.set_value(source.clone(), window, cx);
                });
            }
            view.markdown_source_changed(&document_id, source.clone(), window, cx);
        },
    )
}
