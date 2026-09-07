//! 将 JSON 格式化器接入主应用 Tab 容器。

use gpui::{App, EventEmitter, FocusHandle, Focusable, SharedString};
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;

use crate::JsonFormatterView;

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
        SharedString::from(t!("JsonView.json_formatter"))
    }
}
