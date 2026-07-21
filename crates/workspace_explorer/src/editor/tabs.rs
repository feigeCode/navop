use super::WorkspaceEditor;
use crate::diff::change_starts;
use crate::model::active_index_after_close;
use gpui::{AppContext as _, Context, PromptLevel, Window};
use gpui_component::input::Search;
use rust_i18n::t;

#[derive(Clone, Copy)]
enum DiffNavigationDirection {
    Previous,
    Next,
}

impl WorkspaceEditor {
    pub(super) fn request_close_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.tabs.len() || self.close_prompt_open {
            return;
        }
        if self.tabs[index].is_dirty(cx) {
            self.show_unsaved_prompt(index, window, cx);
        } else {
            self.close_clean_tab(index, window, cx);
        }
    }

    pub(super) fn close_clean_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.tabs.len() {
            return;
        }
        let next_active = active_index_after_close(self.tabs.len(), self.active_tab, index);
        self.tabs.remove(index);
        if let Some(next_active) = next_active {
            self.active_tab = next_active;
            self.focus_editor(window, cx);
        } else {
            self.active_tab = 0;
            cx.emit(super::WorkspaceEditorEvent::VisibilityChanged(false));
        }
        cx.notify();
    }

    fn show_unsaved_prompt(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.close_prompt_open = true;
        self.pending_close_tab = Some(index);
        let labels = [
            t!("WorkspaceExplorer.action.save").to_string(),
            t!("WorkspaceExplorer.action.discard").to_string(),
            t!("WorkspaceExplorer.action.cancel").to_string(),
        ];
        let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
        let answer = window.prompt(
            PromptLevel::Warning,
            &t!("WorkspaceExplorer.prompt.unsaved_title"),
            Some(&t!("WorkspaceExplorer.prompt.unsaved_message")),
            &label_refs,
            cx,
        );
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let selection = answer.await.ok();
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    let index = this.pending_close_tab.take();
                    this.close_prompt_open = false;
                    match (selection, index) {
                        (Some(0), Some(index)) => {
                            this.active_tab = index;
                            this.save(true, window, cx);
                        }
                        (Some(1), Some(index)) => this.close_clean_tab(index, window, cx),
                        _ => {}
                    }
                });
            });
        })
        .detach();
    }

    pub(super) fn switch_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() || index == self.active_tab {
            return;
        }
        self.active_tab = index;
        self.focus_editor(window, cx);
        cx.notify();
    }

    pub(super) fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab().filter(|tab| !tab.read_only) else {
            return;
        };
        if let Some(editor) = tab.editor.as_ref() {
            editor.update(cx, |state, cx| state.focus(window, cx));
        }
    }

    pub(super) fn toggle_soft_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        tab.soft_wrap = !tab.soft_wrap;
        if let Some(editor) = tab.editor.as_ref() {
            editor.update(cx, |state, cx| {
                state.set_soft_wrap(tab.soft_wrap, window, cx);
            });
        }
        cx.notify();
    }

    pub(super) fn toggle_diff_view(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab_mut().filter(|tab| tab.diff.is_some()) else {
            return;
        };
        tab.diff_side_by_side = !tab.diff_side_by_side;
        cx.notify();
    }

    pub(super) fn previous_diff_change(&mut self, cx: &mut Context<Self>) {
        self.navigate_diff_change(DiffNavigationDirection::Previous, cx);
    }

    pub(super) fn next_diff_change(&mut self, cx: &mut Context<Self>) {
        self.navigate_diff_change(DiffNavigationDirection::Next, cx);
    }

    fn navigate_diff_change(&mut self, direction: DiffNavigationDirection, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let (Some(diff), Some(editors)) = (&tab.diff, &tab.diff_editors) else {
            return;
        };
        let changes = change_starts(diff);
        if changes.is_empty() {
            return;
        }

        let visible_range = editors
            .left
            .read(cx)
            .visible_row_range()
            .or_else(|| editors.right.read(cx).visible_row_range());
        let next_index = diff_navigation_index(
            tab.diff_change_cursor,
            &changes,
            visible_range.as_ref(),
            direction,
        );
        let row = changes[next_index];
        let left = editors.left.clone();
        let right = editors.right.clone();

        if let Some(tab) = self.active_tab_mut() {
            tab.diff_change_cursor = Some(next_index);
        }
        left.update(cx, |state, cx| state.scroll_to_line(row, cx));
        right.update(cx, |state, cx| state.scroll_to_line(row, cx));
        cx.notify();
    }

    pub(super) fn trigger_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let editor = if tab.diff_side_by_side {
            tab.diff_editors.as_ref().map(|editors| {
                if editors.right.read(cx).is_focused(window) {
                    editors.right.clone()
                } else {
                    editors.left.clone()
                }
            })
        } else {
            tab.editor.clone()
        };
        let Some(editor) = editor else {
            return;
        };
        editor.update(cx, |state, cx| state.focus(window, cx));
        window.dispatch_action(Box::new(Search), cx);
    }

    pub(super) fn trigger_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab().filter(|tab| !tab.read_only) else {
            return;
        };
        if let Some(editor) = tab.editor.as_ref() {
            editor.update(cx, |state, cx| state.open_search_and_replace(window, cx));
        }
    }
}

fn diff_navigation_index(
    current: Option<usize>,
    changes: &[usize],
    visible_range: Option<&std::ops::Range<usize>>,
    direction: DiffNavigationDirection,
) -> usize {
    if let Some(current) = current.filter(|&index| {
        changes
            .get(index)
            .is_some_and(|row| visible_range.is_none_or(|range| range.contains(row)))
    }) {
        return match direction {
            DiffNavigationDirection::Previous => {
                current.checked_sub(1).unwrap_or(changes.len() - 1)
            }
            DiffNavigationDirection::Next => (current + 1) % changes.len(),
        };
    }

    match (direction, visible_range) {
        (DiffNavigationDirection::Previous, Some(range)) => changes
            .iter()
            .rposition(|row| *row < range.end)
            .unwrap_or(changes.len() - 1),
        (DiffNavigationDirection::Next, Some(range)) => changes
            .iter()
            .position(|row| *row >= range.start)
            .unwrap_or(0),
        (DiffNavigationDirection::Previous, None) => changes.len() - 1,
        (DiffNavigationDirection::Next, None) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_navigation_wraps_at_both_ends() {
        let changes = [2, 8, 15];
        let visible = 0..20;

        assert_eq!(
            0,
            diff_navigation_index(
                Some(2),
                &changes,
                Some(&visible),
                DiffNavigationDirection::Next
            )
        );
        assert_eq!(
            2,
            diff_navigation_index(
                Some(0),
                &changes,
                Some(&visible),
                DiffNavigationDirection::Previous
            )
        );
    }

    #[test]
    fn diff_navigation_resumes_from_the_visible_rows_after_manual_scroll() {
        let changes = [2, 8, 15, 24];
        let visible = 12..20;

        assert_eq!(
            2,
            diff_navigation_index(
                Some(0),
                &changes,
                Some(&visible),
                DiffNavigationDirection::Next
            )
        );
        assert_eq!(
            2,
            diff_navigation_index(
                Some(3),
                &changes,
                Some(&visible),
                DiffNavigationDirection::Previous
            )
        );
    }
}
