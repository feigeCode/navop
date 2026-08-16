//! TabContent 集成：让两个视图可以作为主应用 Tab 打开（参考 notes 的集成方式）。

use gpui::{App, EventEmitter, FocusHandle, Focusable, SharedString};
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;

use crate::{ApiTestView, JsonFormatterView};

impl EventEmitter<TabContentEvent> for ApiTestView {}

impl Focusable for ApiTestView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TabContent for ApiTestView {
    fn content_key(&self) -> &'static str {
        "ApiTest"
    }

    fn title(&self, _cx: &App) -> SharedString {
        SharedString::from(t!("ApiTools.api_testing"))
    }
}

impl EventEmitter<TabContentEvent> for JsonFormatterView {}

impl Focusable for JsonFormatterView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TabContent for JsonFormatterView {
    fn content_key(&self) -> &'static str {
        "JsonFormatter"
    }

    fn title(&self, _cx: &App) -> SharedString {
        SharedString::from(t!("ApiTools.json_formatter"))
    }
}
