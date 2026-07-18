//! Ask AI 可复用按钮与全局事件机制。

use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, Global, IntoElement, SharedString,
};
use gpui_component::{
    IconName, Sizable, Size,
    button::{Button, ButtonVariants},
};
use rust_i18n::t;

#[derive(Clone, Debug)]
pub enum AskAiEvent {
    Request(String),
}

pub struct AskAiNotifier;

impl EventEmitter<AskAiEvent> for AskAiNotifier {}

#[derive(Clone)]
pub struct GlobalAskAiNotifier(pub Entity<AskAiNotifier>);

impl Global for GlobalAskAiNotifier {}

pub fn init_ask_ai_notifier(cx: &mut App) {
    let notifier = cx.new(|_| AskAiNotifier);
    cx.set_global(GlobalAskAiNotifier(notifier));
}

pub fn get_ask_ai_notifier(cx: &App) -> Option<Entity<AskAiNotifier>> {
    cx.try_global::<GlobalAskAiNotifier>().map(|g| g.0.clone())
}

pub fn emit_ask_ai_event<T>(message: String, cx: &mut Context<T>) {
    if let Some(notifier) = cx.try_global::<GlobalAskAiNotifier>().cloned() {
        notifier.0.update(cx, |_, cx| {
            cx.emit(AskAiEvent::Request(message));
        });
    }
}

pub fn emit_ask_ai_event_app(message: String, cx: &mut App) {
    if let Some(notifier) = cx.try_global::<GlobalAskAiNotifier>().cloned() {
        notifier.0.update(cx, |_, cx| {
            cx.emit(AskAiEvent::Request(message));
        });
    }
}

pub fn format_ask_ai_message(sql: &str, error_message: &str, context: Option<&str>) -> String {
    let mut message = format!(
        "I encountered an error while running the following SQL:\n\n```sql\n{}\n```\n\nError message:\n```\n{}\n```",
        sql.trim(),
        error_message.trim()
    );

    if let Some(context) = context {
        message.push_str(&format!("\n\nContext:\n{}", context));
    }

    message.push_str("\n\nPlease help me analyze this error and provide a solution.");
    message
}

pub struct AskAiButton {
    id: SharedString,
    sql: String,
    error_message: String,
    context: Option<String>,
    size: Size,
}

impl AskAiButton {
    pub fn new(
        id: impl Into<SharedString>,
        sql: impl Into<String>,
        error_message: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            sql: sql.into(),
            error_message: error_message.into(),
            context: None,
            size: Size::Small,
        }
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    pub fn with_size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
}

impl IntoElement for AskAiButton {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let message =
            format_ask_ai_message(&self.sql, &self.error_message, self.context.as_deref());

        Button::new(self.id)
            .icon(IconName::AI.color())
            .label(t!("AgentUi.ask_ai").to_string())
            .ghost()
            .with_size(self.size)
            .on_click(move |_event, _window, cx| {
                emit_ask_ai_event_app(message.clone(), cx);
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ask_ai_message_includes_sql_error_context_and_request() {
        let message = format_ask_ai_message(
            " select * from users ",
            " relation users does not exist ",
            Some("connection: local"),
        );

        assert!(message.contains("```sql\nselect * from users\n```"));
        assert!(message.contains("relation users does not exist"));
        assert!(message.contains("connection: local"));
        assert!(message.contains("Please help me analyze this error"));
    }

    #[test]
    fn format_ask_ai_message_omits_context_when_absent() {
        let message = format_ask_ai_message("select 1", "timeout", None);

        assert!(message.contains("select 1"));
        assert!(message.contains("timeout"));
        assert!(!message.contains("Context:"));
    }
}
