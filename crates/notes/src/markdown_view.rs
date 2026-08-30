use crate::markdown_file_store::MarkdownFileStore;
use crate::markdown_mode::{
    editor_view_mode, focus_markdown_editor, markdown_view_mode, switch_markdown_mode,
};
use crate::markdown_session::{MarkdownPreview, MarkdownSession, MarkdownSessionState, MarkdownSyncState};
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
                    preview: None,
                    state: MarkdownSessionState::with_mode_and_revision(mode, editor_revision),
                    _subscriptions: vec![editor_subscription],
                    _file_watcher: Some(file_watcher),
                },
            );
            if let Some(storage) = self.storage.as_ref() {
                storage.save_state(&self.tree.to_ui_state())?;
            }
            if mode == MarkdownViewMode::Split {
                self.ensure_markdown_preview(&document_id, window, cx);
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
        self.sync_markdown_preview(document_id, cx);
    }

    /// Split 模式下把主编辑器内容镜像到只读预览。
    fn sync_markdown_preview(&mut self, document_id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.markdown_sessions.get(document_id) else {
            return;
        };
        if session.state.mode != MarkdownViewMode::Split {
            return;
        }
        let Some(preview) = &session.preview else {
            return;
        };
        let source = session.editor.read(cx).markdown(cx);
        preview.editor.update(cx, |preview_editor, cx| {
            preview_editor.replace_markdown(source, cx);
        });
    }

    /// 创建 Split 模式的预览编辑器（Rendered 模式）。
    ///
    /// 预览内容在主编辑器每次变更时被整体替换；用户在预览侧的瞬时编辑会由
    /// 订阅回调立即回滚为镜像内容，因此预览永远不参与持久化。
    fn ensure_markdown_preview(
        &mut self,
        document_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme = crate::markdown_renderer::markdown_editor_theme(self.resolved_editor_theme(cx));
        let Some(session) = self.markdown_sessions.get_mut(document_id) else {
            return;
        };
        if session.preview.is_some() {
            return;
        }
        let host_services = markdown_editor::markdown_editor_host_services(
            theme,
            crate::markdown_renderer::block_render_provider(cx),
        );
        let source = session.editor.read(cx).markdown(cx);
        let preview = cx.new(|cx| {
            MarkdownEditor::from_markdown_embedded_with_host(cx, source, host_services)
        });
        let subscription =
            subscribe_preview_changes(&preview, document_id.to_owned(), window, cx);
        session.preview = Some(MarkdownPreview {
            editor: preview,
            _subscription: subscription,
        });
    }

    /// 预览侧发生编辑：立即用主编辑器内容覆盖，保证预览与文档一致。
    ///
    /// 内容一致时 `replace_markdown` 返回 false 且不再发事件，循环自然终止。
    pub(crate) fn markdown_preview_changed(
        &mut self,
        document_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_markdown_preview(document_id, cx);
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

    /// 直接切换到指定视图模式（工具栏按钮入口），重复选择同一模式为 no-op。
    pub(crate) fn set_markdown_mode(
        &mut self,
        document_id: &str,
        mode: MarkdownViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .markdown_sessions
            .get(document_id)
            .map(|session| session.state.mode);
        if current == Some(mode) || current.is_none() {
            return;
        }
        let session = self.markdown_sessions.get_mut(document_id).expect("checked above");
        switch_markdown_mode(session, mode, window, cx);
        if mode == MarkdownViewMode::Split {
            self.ensure_markdown_preview(document_id, window, cx);
        }
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
        let mapped = markdown_view_mode(mode);
        if session.state.mode == MarkdownViewMode::Split {
            // 分栏模式下编辑器被切回所见即所得（编辑器内部快捷键），退出分栏。
            if mapped == MarkdownViewMode::Wysiwyg {
                session.preview = None;
                session.state.set_mode(MarkdownViewMode::Wysiwyg);
                self.tree
                    .markdown_view_modes
                    .insert(document_id.to_owned(), MarkdownViewMode::Wysiwyg);
                if let Some(storage) = self.storage.as_ref()
                    && let Err(error) = storage.save_state(&self.tree.to_ui_state())
                {
                    notify_operation_error(window, cx, error);
                }
                cx.notify();
            }
            return;
        }
        if session.state.mode == mapped {
            return;
        }
        session.state.set_mode(mapped);
        self.tree
            .markdown_view_modes
            .insert(document_id.to_owned(), mapped);
        if let Some(storage) = self.storage.as_ref()
            && let Err(error) = storage.save_state(&self.tree.to_ui_state())
        {
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

fn subscribe_preview_changes(
    editor: &gpui::Entity<MarkdownEditor>,
    document_id: String,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> gpui::Subscription {
    cx.subscribe_in(
        editor,
        window,
        move |view, _, event: &MarkdownEditorEvent, window, cx| match event {
            MarkdownEditorEvent::Changed { .. } => {
                view.markdown_preview_changed(&document_id, window, cx);
            }
            MarkdownEditorEvent::ViewModeChanged { .. } => {}
        },
    )
}
