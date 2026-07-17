use crate::markdown_adapter::{
    MarkdownProjectionConfig, apply_markdown_source, build_markdown_projection,
    export_markdown_strict,
};
use crate::markdown_file_store::MarkdownFileStore;
use crate::markdown_session::{MarkdownSession, MarkdownSessionState, MarkdownSyncState};
use crate::markdown_source::{create_source_editor, subscribe_source_changes};
use crate::notes_notifications::notify_operation_error;
use crate::path_policy::remap_path;
use crate::{DocumentDescriptor, MarkdownViewMode, NotesView};
use gpui::{AppContext, AsyncApp, Context, Window};
use one_core::settings::AppSettings;
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
            let mode = MarkdownViewMode::Wysiwyg;
            self.tree
                .markdown_view_modes
                .insert(document_id.clone(), mode);
            let store = MarkdownFileStore::new(descriptor.absolute_path.clone());
            let snapshot = store.load()?;
            let source_editor = create_source_editor(&snapshot.source, window, cx);
            let ai_model_id = AppSettings::global(cx).ai_chat.notes_model_id.clone();
            let projection = build_markdown_projection(
                MarkdownProjectionConfig {
                    document_id: &document_id,
                    source: &snapshot.source,
                    store: store.clone(),
                    ai_provider: self.ai_provider.clone(),
                    ai_model_id: ai_model_id.as_deref(),
                    syntax_highlight_provider: self.syntax_highlight_provider.clone(),
                    document_renderer_provider: self
                        .document_renderer_provider
                        .clone()
                        .map(|provider| provider as Arc<dyn cditor_app::DocumentRendererProvider>),
                },
                cx,
            )?;
            self.observe_editor_events(projection.events, window, cx);
            let subscription =
                subscribe_source_changes(&source_editor, document_id.clone(), window, cx);
            self.markdown_sessions.insert(
                document_id.clone(),
                MarkdownSession {
                    relative_path: descriptor.relative_path,
                    store,
                    source_editor,
                    preview: projection.handle,
                    compatibility: projection.compatibility,
                    diagnostics: projection.diagnostics,
                    normalization_accepted: false,
                    save_generation: Default::default(),
                    state: MarkdownSessionState {
                        mode,
                        ..MarkdownSessionState::default()
                    },
                    _subscription: subscription,
                },
            );
            if let Some(storage) = self.storage.as_ref() {
                storage.save_state(&self.tree.to_ui_state())?;
            }
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.markdown_sessions.get_mut(document_id) else {
            return;
        };
        let generation = session.state.source_changed();
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
            MarkdownViewMode::Source => switch_to_wysiwyg(session, cx),
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

    pub(crate) fn accept_markdown_normalization(
        &mut self,
        document_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.markdown_sessions.get_mut(document_id) else {
            return;
        };
        if !matches!(
            session.compatibility,
            cditor_app::MarkdownCompatibility::EditableWithNormalization(_)
        ) {
            return;
        }
        session.normalization_accepted = true;
        if let Err(error) = session.preview.set_readonly(false, cx) {
            notify_operation_error(window, cx, error);
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

fn switch_to_wysiwyg(
    session: &mut MarkdownSession,
    cx: &mut Context<NotesView>,
) -> anyhow::Result<Option<String>> {
    let source = session.source_editor.read(cx).value().to_string();
    let imported = apply_markdown_source(
        &session.preview,
        &source,
        session.normalization_accepted,
        cx,
    )?;
    session.compatibility = imported.compatibility;
    session.diagnostics = imported.diagnostics;
    session.state.switch_to_wysiwyg();
    Ok(None)
}

fn switch_to_source(
    session: &mut MarkdownSession,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> anyhow::Result<Option<String>> {
    let markdown = export_markdown_strict(&session.preview, cx)?;
    session.source_editor.update(cx, |input, cx| {
        input.set_value(markdown.clone(), window, cx)
    });
    session.state.switch_to_source();
    let input = session.source_editor.clone();
    window.defer(cx, move |window, cx| {
        input.update(cx, |input, cx| input.focus(window, cx))
    });
    Ok(Some(markdown))
}
use std::sync::Arc;
