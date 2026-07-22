use crate::NotesView;
use crate::markdown_session::MarkdownSession;
use gpui::{Context, Window};

pub(crate) fn switch_to_wysiwyg(
    session: &mut MarkdownSession,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> anyhow::Result<Option<String>> {
    session.state.switch_to_wysiwyg();
    focus_preview(session, window, cx);
    Ok(None)
}

pub(crate) fn switch_to_source(
    session: &mut MarkdownSession,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) -> anyhow::Result<Option<String>> {
    let source = session.preview.read(cx).source().to_owned();
    session
        .source_editor
        .update(cx, |input, cx| input.set_value(source, window, cx));
    session.state.switch_to_source();
    focus_source_editor(session, window, cx);
    Ok(None)
}

fn focus_source_editor(
    session: &MarkdownSession,
    window: &mut Window,
    cx: &mut Context<NotesView>,
) {
    let input = session.source_editor.clone();
    window.defer(cx, move |window, cx| {
        input.update(cx, |input, cx| input.focus(window, cx))
    });
}

fn focus_preview(session: &MarkdownSession, window: &mut Window, cx: &mut Context<NotesView>) {
    let preview = session.preview.clone();
    window.defer(cx, move |window, cx| {
        preview.update(cx, |editor, cx| editor.focus(window, cx))
    });
}
