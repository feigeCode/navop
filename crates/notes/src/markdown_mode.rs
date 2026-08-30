use crate::markdown_session::MarkdownSession;
use crate::{MarkdownViewMode, NotesView};
use gpui::{Context, Window};
use markdown_editor::ViewMode;

pub(crate) fn editor_view_mode(mode: MarkdownViewMode) -> ViewMode {
    match mode {
        MarkdownViewMode::Wysiwyg => ViewMode::Rendered,
        MarkdownViewMode::Source | MarkdownViewMode::Split => ViewMode::Source,
    }
}

pub(crate) fn markdown_view_mode(mode: ViewMode) -> MarkdownViewMode {
    match mode {
        ViewMode::Rendered => MarkdownViewMode::Wysiwyg,
        ViewMode::Source => MarkdownViewMode::Source,
    }
}

pub(crate) fn switch_markdown_mode(
    session: &mut MarkdownSession,
    mode: MarkdownViewMode,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) {
    // 先更新 state 再切换编辑器：编辑器回发的 ViewModeChanged 事件以
    // session.state.mode 为准判断是否需要退出 Split，避免被中间态覆盖。
    session.state.set_mode(mode);
    if mode != MarkdownViewMode::Split {
        session.preview = None;
    }
    session.editor.update(cx, |editor, cx| {
        editor.set_view_mode(editor_view_mode(mode), cx);
    });
    focus_markdown_editor(session, window, cx);
}

pub(crate) fn focus_markdown_editor(
    session: &MarkdownSession,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) {
    let editor = session.editor.clone();
    window.defer(cx, move |window, cx| {
        editor.update(cx, |editor, cx| {
            editor.focus(window, cx);
        });
    });
}
