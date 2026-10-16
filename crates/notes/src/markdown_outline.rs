//! Collapsible document outline for the embedded Markdown editor.
//!
//! The outline lists the ATX headings of the canonical Markdown serialization,
//! which is the same text [`markdown_editor::Editor::reveal_outline_heading`]
//! navigates by, so a click jumps to the owning block.

use crate::NotesView;
use gpui::{
    AnyElement, App, Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    div, px,
};
use gpui::StatefulInteractiveElement;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{ActiveTheme, h_flex, v_flex};
use markdown_editor::OutlineEntry;
use rust_i18n::t;

/// Width of the outline column to the right of the editor.
const OUTLINE_PANEL_WIDTH: f32 = 220.0;
/// Left padding before a level-1 heading, growing by [`OUTLINE_INDENT_STEP`] per level.
const OUTLINE_INDENT: f32 = 8.0;
/// Extra left padding per heading level.
const OUTLINE_INDENT_STEP: f32 = 12.0;

/// Cached outline of the active document.
///
/// Entries are recomputed only when the active document or its editor revision
/// changes, so repeated renders of a large document stay free.
#[derive(Default)]
pub(crate) struct MarkdownOutlineState {
    visible: bool,
    entries: Vec<OutlineEntry>,
    document_id: Option<String>,
    revision: u64,
}

impl MarkdownOutlineState {
    pub(crate) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn entries(&self) -> &[OutlineEntry] {
        &self.entries
    }

    /// Flips the panel and reports whether the outline is now visible.
    pub(crate) fn toggle(&mut self) -> bool {
        self.visible = !self.visible;
        self.visible
    }

    /// Recomputes entries when the document or its revision changed, and
    /// reports whether the cached entries changed.
    pub(crate) fn refresh(&mut self, document_id: &str, revision: u64, markdown: &str) -> bool {
        if self.document_id.as_deref() == Some(document_id) && self.revision == revision {
            return false;
        }
        self.document_id = Some(document_id.to_owned());
        self.revision = revision;
        let entries = markdown_editor::outline_entries(markdown);
        let changed = entries != self.entries;
        self.entries = entries;
        changed
    }

    /// Drops the cached document so the next refresh always reparses.
    pub(crate) fn clear(&mut self) {
        self.document_id = None;
        self.revision = 0;
        self.entries.clear();
    }
}

impl NotesView {
    /// Reparses the outline of the active document, if any, and reports whether
    /// the cached entries changed.
    pub(crate) fn refresh_markdown_outline(&mut self, cx: &App) -> bool {
        let Some(document_id) = self.active_document_id.clone() else {
            self.outline.clear();
            return false;
        };
        let Some(session) = self.markdown_sessions.get(&document_id) else {
            self.outline.clear();
            return false;
        };
        let revision = session.state.editor_revision;
        let markdown = session.editor.read(cx).markdown(cx);
        self.outline.refresh(&document_id, revision, &markdown)
    }

    pub(crate) fn toggle_markdown_outline(&mut self, cx: &mut Context<Self>) {
        if self.outline.toggle() {
            self.refresh_markdown_outline(cx);
        }
        cx.notify();
    }

    /// Focuses the block that owns the clicked heading's source line.
    pub(crate) fn reveal_outline_entry(&mut self, line: usize, cx: &mut Context<Self>) {
        let Some(document_id) = self.active_document_id.clone() else {
            return;
        };
        let Some(session) = self.markdown_sessions.get(&document_id) else {
            return;
        };
        let editor = session.editor.clone();
        editor.update(cx, |editor, cx| editor.reveal_outline_heading(line, cx));
    }

    pub(crate) fn render_markdown_outline(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.resolved_editor_theme(cx);
        let mut rows = Vec::new();
        if self.outline.entries().is_empty() {
            rows.push(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("Notes.markdown_outline_empty").to_string())
                    .into_any_element(),
            );
        }
        for (index, entry) in self.outline.entries().iter().enumerate() {
            let line = entry.line;
            let indent =
                px(OUTLINE_INDENT + (entry.level.saturating_sub(1)) as f32 * OUTLINE_INDENT_STEP);
            let hover_background = cx.theme().secondary;
            rows.push(
                h_flex()
                    .id(SharedString::from(format!("markdown-outline-row-{index}")))
                    .debug_selector(move || format!("markdown-outline-row-{index}"))
                    .h(px(24.0))
                    .w_full()
                    .min_w_0()
                    .rounded_sm()
                    .pl(indent)
                    .pr_2()
                    .cursor_pointer()
                    .hover(move |this| this.bg(hover_background))
                    .on_click(cx.listener(move |view, _, _window, cx| {
                        view.reveal_outline_entry(line, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_xs()
                            .text_color(theme.foreground)
                            .child(entry.title.clone()),
                    )
                    .into_any_element(),
            );
        }

        v_flex()
            .id("markdown-outline")
            .debug_selector(|| "markdown-outline".to_owned())
            .w(px(OUTLINE_PANEL_WIDTH))
            .h_full()
            .flex_shrink_0()
            .min_h_0()
            .overflow_hidden()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                h_flex()
                    .w_full()
                    .h_9()
                    .flex_shrink_0()
                    .items_center()
                    .px_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("Notes.markdown_outline").to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(v_flex().size_full().p_1().gap_1().overflow_y_scrollbar().children(rows)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_caches_until_the_document_or_revision_changes() {
        let mut state = MarkdownOutlineState::default();
        assert!(state.refresh("doc", 1, "# Alpha\n"));
        assert_eq!(1, state.entries().len());
        assert!(!state.refresh("doc", 1, "# Alpha\n"));
        assert!(state.refresh("doc", 2, "# Alpha\n\n## Beta\n"));
        assert_eq!(2, state.entries().len());
        assert!(state.refresh("other", 2, "# Gamma\n"));
        assert_eq!(1, state.entries().len());
    }

    #[test]
    fn clear_forces_a_reparse_of_the_next_document() {
        let mut state = MarkdownOutlineState::default();
        state.refresh("doc", 1, "# Alpha\n");
        state.clear();
        assert!(state.refresh("doc", 1, "# Alpha\n"));
    }

    #[test]
    fn toggling_reports_the_new_visibility() {
        let mut state = MarkdownOutlineState::default();
        assert!(!state.is_visible());
        assert!(state.toggle());
        assert!(state.is_visible());
        assert!(!state.toggle());
    }
}
