//! Query 页面输入状态。
//!
//! 输入草稿由独立 Entity 持有:页面切换不丢稿,按键通知不重绘整个工作台
//! (遵循仓库“输入面板独立 Entity”的沉淀经验)。

use extension_runtime::extension::manifest::ResourceWorkbenchPage;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, Window, div, px,
};
use gpui_component::input::{Input, InputState};
use std::collections::BTreeMap;

pub struct QueryInputState {
    inputs: BTreeMap<String, Entity<InputState>>,
    focus_handle: FocusHandle,
}

impl QueryInputState {
    pub fn new(page: &ResourceWorkbenchPage, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut inputs = BTreeMap::new();
        for input in &page.inputs {
            let initial = input.default.clone().unwrap_or_default();
            let state = cx.new(|cx| {
                let mut input = InputState::new(window, cx);
                input.set_value(initial, window, cx);
                input
            });
            inputs.insert(input.id.clone(), state);
        }
        Self {
            inputs,
            focus_handle: cx.focus_handle(),
        }
    }

    /// 当前所有输入值(按字段 id)。
    pub fn values(&self, cx: &App) -> BTreeMap<String, String> {
        self.inputs
            .iter()
            .map(|(id, state)| (id.clone(), state.read(cx).text().to_string()))
            .collect()
    }
}

impl Focusable for QueryInputState {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for QueryInputState {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(self.inputs.iter().map(|(id, state)| {
                let state = state.clone();
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(div().w(px(120.)).child(id.clone()))
                    .child(div().flex_1().min_w_0().child(Input::new(&state)))
            }))
    }
}
