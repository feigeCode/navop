use super::{WorkspaceEditor, WorkspaceEditorEvent};
use crate::WorkspaceTheme;
use gpui::{AppContext as _, AsyncApp, Context, Entity, Subscription, WeakEntity, Window};
use notes::{MarkdownEditorTheme, NotesView, NotesViewEvent};
use std::path::PathBuf;

pub(super) fn create_markdown_editor(
    path: PathBuf,
    theme: WorkspaceTheme,
    window: &mut Window,
    cx: &mut Context<WorkspaceEditor>,
) -> (Entity<NotesView>, Vec<Subscription>) {
    let theme = markdown_editor_theme(theme);
    let markdown =
        cx.new(|cx| NotesView::new_for_markdown_file_with_theme(path, theme, window, cx));
    let observe = cx.observe(&markdown, |_, _, cx| cx.notify());
    let saved = cx.subscribe(&markdown, |_, _, event: &NotesViewEvent, cx| match event {
        NotesViewEvent::FileSaved(path) => {
            cx.emit(WorkspaceEditorEvent::FileSaved(path.clone()));
        }
    });
    (markdown, vec![observe, saved])
}

pub(super) fn markdown_editor_theme(theme: WorkspaceTheme) -> MarkdownEditorTheme {
    MarkdownEditorTheme {
        background: theme.background,
        foreground: theme.foreground,
        muted: theme.muted,
        muted_foreground: theme.muted_foreground,
        border: theme.border,
        primary: theme.accent,
        primary_foreground: theme.accent_foreground,
        danger: theme.danger,
        warning: theme.warning,
        highlight_theme: theme.highlight_theme(),
    }
}

impl WorkspaceEditor {
    pub(super) fn request_close_markdown_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let Some(markdown) = tab.markdown.clone() else {
            return;
        };
        self.close_prompt_open = true;
        let tab_id = tab.id;
        let key = tab.key.clone();
        let close = markdown.update(cx, |view, cx| view.prepare_close(window, cx));
        let editor = cx.entity().downgrade();
        let window_handle = window.window_handle();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let should_close = close.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = editor.update(cx, |editor, cx| {
                    editor.close_prompt_open = false;
                    if should_close && let Some(index) = editor.tab_index(tab_id, &key) {
                        editor.close_clean_tab(index, window, cx);
                    } else {
                        cx.notify();
                    }
                });
            });
        })
        .detach();
    }
}
