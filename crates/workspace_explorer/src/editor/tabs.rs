use super::WorkspaceEditor;
use crate::model::active_index_after_close;
use gpui::{AppContext as _, Context, PromptLevel, Window};
use gpui_component::input::Search;
use rust_i18n::t;

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

    pub(super) fn trigger_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_tab().and_then(|tab| tab.editor.as_ref()) {
            editor.update(cx, |state, cx| state.focus(window, cx));
            window.dispatch_action(Box::new(Search), cx);
        }
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
