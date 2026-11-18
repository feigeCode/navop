use crate::markdown_session::MarkdownSession;
use crate::{MarkdownViewMode, NotesView};
use gpui::{Context, Window};
use markdown_editor::ViewMode;
use one_core::settings::MarkdownDefaultViewMode;

/// 把设置里的「笔记默认视图」映射成笔记页签的视图模式（#132）。
pub(crate) fn default_markdown_view_mode(mode: MarkdownDefaultViewMode) -> MarkdownViewMode {
    match mode {
        MarkdownDefaultViewMode::Wysiwyg => MarkdownViewMode::Wysiwyg,
        MarkdownDefaultViewMode::Source => MarkdownViewMode::Source,
    }
}

pub(crate) fn editor_view_mode(mode: MarkdownViewMode) -> ViewMode {
    match mode {
        MarkdownViewMode::Wysiwyg => ViewMode::Rendered,
        MarkdownViewMode::Source => ViewMode::Source,
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
    session.state.set_mode(mode);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_markdown_view_mode_follows_the_setting() {
        assert_eq!(
            default_markdown_view_mode(MarkdownDefaultViewMode::Wysiwyg),
            MarkdownViewMode::Wysiwyg
        );
        assert_eq!(
            default_markdown_view_mode(MarkdownDefaultViewMode::Source),
            MarkdownViewMode::Source
        );
    }
}
