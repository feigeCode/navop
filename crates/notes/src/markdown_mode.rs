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

/// 设置里的默认视图与当前页签不一致时，返回点击提示后应切换到的模式（#132）。
///
/// 已经打开的笔记页签不会跟随设置变化，因此在页签上给出「跟随默认视图」的提示，
/// 由用户决定是否切换；一致时返回 `None`，不打扰用户。
pub(crate) fn markdown_default_mode_target(
    current: MarkdownViewMode,
    default: MarkdownDefaultViewMode,
) -> Option<MarkdownViewMode> {
    let target = default_markdown_view_mode(default);
    (target != current).then_some(target)
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

    #[test]
    fn default_mode_target_only_reports_a_difference() {
        assert_eq!(
            markdown_default_mode_target(
                MarkdownViewMode::Wysiwyg,
                MarkdownDefaultViewMode::Source
            ),
            Some(MarkdownViewMode::Source)
        );
        assert_eq!(
            markdown_default_mode_target(
                MarkdownViewMode::Wysiwyg,
                MarkdownDefaultViewMode::Wysiwyg
            ),
            None
        );
    }
}
