use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use gpui::{Element as _, ParentElement as _, Styled as _, div, px, rgb};

use crate::plugin::{FlowEvent, Plugin, PluginContext, RenderContext};

const DEFAULT_TOAST_DURATION: Duration = Duration::from_millis(3000);
const MAX_TOASTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct ToastMessage {
    pub text: String,
    pub level: ToastLevel,
    pub duration: Duration,
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
    level: ToastLevel,
    expires_at: Instant,
}

pub struct ToastPlugin {
    queue: VecDeque<ToastItem>,
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
        }
    }

    fn gc_expired(&mut self) {
        let now = Instant::now();
        self.queue.retain(|item| item.expires_at > now);
    }

    fn push(&mut self, msg: ToastMessage) {
        self.gc_expired();
        self.queue.push_back(ToastItem {
            text: msg.text,
            level: msg.level,
            expires_at: Instant::now() + msg.duration,
        });
        while self.queue.len() > MAX_TOASTS {
            let _ = self.queue.pop_front();
        }
    }

    fn bg_color(level: ToastLevel) -> u32 {
        match level {
            ToastLevel::Info => 0x001F2937,
            ToastLevel::Success => 0x001E8E3E,
            ToastLevel::Warning => 0x00B35A00,
            ToastLevel::Error => 0x00D32F2F,
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
        if let Some(msg) = event.as_custom::<ToastMessage>() {
            let duration = msg.duration;
            self.push(msg.clone());
            ctx.schedule_after(duration);
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

    fn render(&mut self, _ctx: &mut RenderContext) -> Option<gpui::AnyElement> {
        self.gc_expired();
        if self.queue.is_empty() {
            return None;
        }

        let items = self.queue.iter().rev().map(|item| {
            div()
                .mb_2()
                .max_w(px(360.0))
                .rounded(px(8.0))
                .bg(rgb(Self::bg_color(item.level)))
                .px_3()
                .py_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x00FFFFFF))
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
