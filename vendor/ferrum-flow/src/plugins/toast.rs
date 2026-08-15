use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use gpui::{Element as _, ParentElement as _, Styled as _, div, px, rgb};

use crate::{
    FlowTheme,
    plugin::{CanvasMessage, FlowEvent, MessageLevel, Plugin, PluginContext, RenderContext},
};

const DEFAULT_TOAST_DURATION: Duration = Duration::from_millis(3000);
const MAX_TOASTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl From<MessageLevel> for ToastLevel {
    fn from(level: MessageLevel) -> Self {
        match level {
            MessageLevel::Error => ToastLevel::Error,
            MessageLevel::Warning => ToastLevel::Warning,
            MessageLevel::Info => ToastLevel::Info,
            MessageLevel::Success => ToastLevel::Success,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToastMessage {
    #[allow(unused)]
    text: String,
    #[allow(unused)]
    level: ToastLevel,
    duration: Duration,
}

impl From<CanvasMessage> for ToastMessage {
    fn from(message: CanvasMessage) -> Self {
        Self {
            text: message.message().into(),
            level: message.level().into(),
            duration: DEFAULT_TOAST_DURATION,
        }
    }
}

impl ToastMessage {
    pub fn new(text: impl Into<String>, level: ToastLevel) -> Self {
        Self {
            text: text.into(),
            level,
            duration: DEFAULT_TOAST_DURATION,
        }
    }

    pub fn info(text: impl Into<String>) -> Self {
        Self::new(text, ToastLevel::Info)
    }

    pub fn success(text: impl Into<String>) -> Self {
        Self::new(text, ToastLevel::Success)
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self::new(text, ToastLevel::Warning)
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::new(text, ToastLevel::Error)
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }
}

#[derive(Debug, Clone)]
struct ToastItem {
    text: String,
    level: MessageLevel,
    expires_at: Instant,
}

pub struct ToastPlugin {
    queue: VecDeque<ToastItem>,
    duration: Duration,
    max_toasts: usize,
}

impl Default for ToastPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ToastPlugin {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            duration: DEFAULT_TOAST_DURATION,
            max_toasts: MAX_TOASTS,
        }
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    pub fn with_max(mut self, max_toasts: usize) -> Self {
        self.max_toasts = max_toasts;
        self
    }

    fn gc_expired(&mut self) {
        let now = Instant::now();
        self.queue.retain(|item| item.expires_at > now);
    }

    fn push(&mut self, msg: &CanvasMessage) {
        self.gc_expired();
        self.queue.push_back(ToastItem {
            text: msg.message().into(),
            level: msg.level(),
            expires_at: Instant::now() + self.duration,
        });
        while self.queue.len() > self.max_toasts {
            let _ = self.queue.pop_front();
        }
    }

    fn bg_color(level: MessageLevel, theme: &FlowTheme) -> u32 {
        match level {
            MessageLevel::Info => theme.info,
            MessageLevel::Success => theme.success,
            MessageLevel::Warning => theme.warning,
            MessageLevel::Error => theme.error,
        }
    }
}

impl Plugin for ToastPlugin {
    fn name(&self) -> &'static str {
        "toast"
    }

    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut PluginContext,
    ) -> crate::plugin::EventResult {
        self.gc_expired();
        if let FlowEvent::Message(msg) = event {
            ctx.schedule_after(self.duration);
            self.push(msg);
            ctx.notify();
        }
        crate::plugin::EventResult::Continue
    }

    fn priority(&self) -> i32 {
        10
    }

    fn render_layer(&self) -> crate::plugin::RenderLayer {
        crate::plugin::RenderLayer::Overlay
    }

    fn render(&mut self, ctx: &mut RenderContext) -> Option<gpui::AnyElement> {
        self.gc_expired();
        if self.queue.is_empty() {
            return None;
        }

        let items = self.queue.iter().rev().map(|item| {
            div()
                .mb_2()
                .max_w(px(360.0))
                .rounded(px(8.0))
                .bg(rgb(Self::bg_color(item.level, ctx.theme)))
                .px_3()
                .py_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(ctx.theme.toast_text))
                        .child(item.text.clone()),
                )
        });

        Some(
            div()
                .absolute()
                .right(px(12.0))
                .bottom(px(12.0))
                .children(items)
                .into_any(),
        )
    }
}
