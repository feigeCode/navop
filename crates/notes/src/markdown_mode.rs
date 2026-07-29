use crate::markdown_session::MarkdownSession;
use crate::{MarkdownViewMode, NotesView};
use gpui::{Context, Window};
use markdown_editor::ViewMode;

pub(crate) fn editor_view_mode(mode: MarkdownViewMode) -> ViewMode {
    match mode {
        MarkdownViewMode::Wysiwyg => ViewMode::Rendered,
        MarkdownViewMode::Source => ViewMode::Source,
    }
}

pub(crate) fn switch_markdown_mode(
    session: &mut MarkdownSession,
    mode: MarkdownViewMode,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) {
    session.editor.update(cx, |editor, cx| {
        editor.set_view_mode(editor_view_mode(mode), cx);
    });
    session.state.set_mode(mode);
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
