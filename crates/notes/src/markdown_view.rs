use crate::markdown_adapter::{build_markdown_preview, refresh_markdown_preview};
use crate::markdown_file_store::{MarkdownFileStore, MarkdownSaveOutcome};
use crate::markdown_session::{MarkdownSession, MarkdownSessionState, MarkdownSyncState};
use crate::{DocumentDescriptor, MarkdownViewMode, NotesView};
use gpui::{AppContext, Context, Entity, Subscription, WeakEntity, Window};
use gpui_component::input::{InputEvent, InputState};
use std::sync::atomic::Ordering;
use std::time::Duration;

const MARKDOWN_SAVE_DELAY: Duration = Duration::from_millis(700);

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
            let store = MarkdownFileStore::new(descriptor.absolute_path.clone());
            let snapshot = store.load()?;
            let source_editor = create_source_editor(&snapshot.source, window, cx);
            let preview = build_markdown_preview(&document_id, &snapshot.source, cx)?;
            let subscription = subscribe_source_changes(
                &source_editor,
                document_id.clone(),
                cx.entity().downgrade(),
                window,
                cx,
            );
            self.markdown_sessions.insert(
                document_id.clone(),
                MarkdownSession {
                    relative_path: descriptor.relative_path,
                    store,
                    source_editor,
                    preview,
                    fingerprint: Some(snapshot.fingerprint),
                    save_generation: Default::default(),
                    state: MarkdownSessionState {
                        mode,
                        ..MarkdownSessionState::default()
                    },
                    _subscription: subscription,
                },
            );
        }
        self.active_document_id = Some(document_id.clone());
        self.active_editor = None;
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
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.markdown_sessions.get_mut(document_id) else {
            return;
        };
        let generation = session.state.source_changed();
        session.save_generation.store(generation, Ordering::Release);
        let _ = session.state.begin_source_save(generation);
        let store = session.store.clone();
        let expected = session.fingerprint.clone();
        let generation_token = session.save_generation.clone();
        let weak = cx.entity().downgrade();
        let id = document_id.to_owned();
        let executor = cx.background_executor().clone();
        let task = cx.background_spawn(async move {
            executor.timer(MARKDOWN_SAVE_DELAY).await;
            if generation_token.load(Ordering::Acquire) != generation {
                return Ok(None);
            }
            store.save(&source, expected.as_ref()).map(Some)
        });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            let _ = weak.update(cx, |view, cx| {
                view.finish_markdown_save(&id, generation, result, cx)
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_markdown_save(
        &mut self,
        document_id: &str,
        generation: u64,
        result: anyhow::Result<Option<MarkdownSaveOutcome>>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.markdown_sessions.get_mut(document_id) else {
            return;
        };
        match result {
            Ok(Some(MarkdownSaveOutcome::Saved(fingerprint))) => {
                session.fingerprint = Some(fingerprint);
                session.state.source_saved(generation);
            }
            Ok(Some(MarkdownSaveOutcome::Conflict(_))) => {
                session.state.conflict();
                self.error = Some("Markdown 文件已被外部修改，请重新加载后再保存".into());
            }
            Ok(None) => {}
            Err(error) => {
                session
                    .state
                    .source_save_failed(generation, error.to_string());
                self.error = Some(format!("保存 Markdown 失败：{error}").into());
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_markdown_mode(
        &mut self,
        document_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.markdown_sessions.get_mut(&document_id) else {
            return;
        };
        if !session.state.begin_switch() {
            return;
        }
        match session.state.mode {
            MarkdownViewMode::Source => {
                let source = session.source_editor.read(cx).value().to_string();
                if let Err(error) = refresh_markdown_preview(&session.preview, &source, cx) {
                    session
                        .state
                        .source_save_failed(session.state.generation, error.to_string());
                    self.set_error(error);
                    return;
                }
                session.state.switch_to_wysiwyg();
            }
            MarkdownViewMode::Wysiwyg => {
                session.state.switch_to_source();
                let input = session.source_editor.clone();
                window.defer(cx, move |window, cx| {
                    input.update(cx, |input, cx| input.focus(window, cx))
                });
            }
        }
        let mode = session.state.mode;
        self.tree.markdown_view_modes.insert(document_id, mode);
        if let Err(error) = self
            .storage()
            .and_then(|storage| storage.save_state(&self.tree.to_ui_state()))
        {
            self.set_error(error);
        }
        cx.notify();
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

fn create_source_editor(
    source: &str,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .code_editor("markdown")
            .line_number(true)
            .multi_line(true)
            .soft_wrap(true)
            .default_value(source)
    })
}

fn subscribe_source_changes(
    input: &Entity<InputState>,
    document_id: String,
    view: WeakEntity<NotesView>,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> Subscription {
    cx.subscribe_in(input, window, move |_view, input, event, _window, cx| {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let source = input.read(cx).value().to_string();
        let _ = view.update(cx, |view, cx| {
            view.markdown_source_changed(&document_id, source, cx);
        });
    })
}

fn remap_path(
    path: &std::path::Path,
    old: &std::path::Path,
    new: &std::path::Path,
) -> std::path::PathBuf {
    path.strip_prefix(old)
        .map(|suffix| new.join(suffix))
        .unwrap_or_else(|_| path.to_path_buf())
}
