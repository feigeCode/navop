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
    // 测试构建里没有监视桥,重挂入口整块被 cfg 掉,窗口参数用不上。
    #[cfg_attr(test, allow(unused_variables))]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // provider 换代后要重新挂载 gpui 视图,而挂载必须有 `Window`;
        // 监视事件入口没有窗口,所以重挂在这里落地(下一帧执行,避免在
        // render 中改变挂载状态)。
        #[cfg(not(test))]
        if self.take_pending_remount() {
            cx.defer_in(window, |this, window, cx| {
                this.remount_after_runtime_change(window, cx);
            });
        }
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
