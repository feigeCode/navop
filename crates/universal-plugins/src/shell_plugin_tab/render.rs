use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    SharedString, Styled, Task, Window, div,
};
use one_core::tab_container::{TabContent, TabContentEvent};

use super::{ShellPluginTab, ShellPluginTabState};

impl Drop for ShellPluginTab {
    fn drop(&mut self) {
        self.preparation.cancel.cancel();
        let mut activations = std::mem::take(&mut self.activations);
        activations.extend(self.preparation.take_late());
        let state = std::mem::replace(
            &mut self.state,
            ShellPluginTabState::Failed("Extension dropped".into()),
        );
        let session = match state {
            ShellPluginTabState::Ready(loaded) => Some(loaded.session()),
            _ => None,
        };
        self.host.release_after_session(session, activations);
    }
}

impl EventEmitter<TabContentEvent> for ShellPluginTab {}

impl Focusable for ShellPluginTab {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ShellPluginTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(match &self.state {
                ShellPluginTabState::Loading => div().p_4().child("Loading extension..."),
                ShellPluginTabState::Failed(error) => {
                    div().p_4().child(format!("Extension failed: {error}"))
                }
                ShellPluginTabState::Ready(loaded) => {
                    div().size_full().child(loaded.view().clone())
                }
            })
    }
}

impl TabContent for ShellPluginTab {
    fn content_key(&self) -> &'static str {
        "ShellPlugin"
    }

    fn title(&self, _cx: &App) -> SharedString {
        self.title.clone()
    }

    fn can_rename(&self, _cx: &App) -> bool {
        false
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if self.closing {
            return Task::ready(true);
        }
        self.close_task(false, cx)
    }
}
