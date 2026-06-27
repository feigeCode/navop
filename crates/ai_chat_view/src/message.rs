use std::collections::hash_map::DefaultHasher;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};

use uuid::Uuid;

pub const MESSAGE_RENDER_LIMIT: usize = 60;
pub const MESSAGE_RENDER_STEP: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageVariant {
    Text,
    Card { kind: String },
    SqlResult,
    Status { title: String, is_done: bool },
}

pub trait MessageExtension: Clone + Debug + Send + Sync + 'static {
    fn on_finalize_streaming(&mut self) {}
    fn clear_cache(&mut self) {}
}

#[derive(Clone, Debug, Default)]
pub struct NoExtension;

impl MessageExtension for NoExtension {}

#[derive(Clone, Debug)]
pub struct ChatMessageUIGeneric<E: MessageExtension = NoExtension> {
    pub id: String,
    pub role: ChatRole,
    pub content: String,
    pub reasoning_content: String,
    pub variant: MessageVariant,
    pub is_streaming: bool,
    pub is_expanded: bool,
    pub is_reasoning_expanded: bool,
    cached_content_hash: Option<u64>,
    pub extension: E,
}

pub type ChatMessageUI = ChatMessageUIGeneric<NoExtension>;

impl<E: MessageExtension + Default> ChatMessageUIGeneric<E> {
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(ChatRole::User, content, MessageVariant::Text, false)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(ChatRole::Assistant, content, MessageVariant::Text, false)
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(ChatRole::System, content, MessageVariant::Text, false)
    }

    pub fn card(kind: impl Into<String>, content: impl Into<String>) -> Self {
        Self::new(
            ChatRole::Assistant,
            content,
            MessageVariant::Card { kind: kind.into() },
            false,
        )
    }

    pub fn status(title: impl Into<String>, is_done: bool) -> Self {
        Self::new(
            ChatRole::Assistant,
            String::new(),
            MessageVariant::Status {
                title: title.into(),
                is_done,
            },
            !is_done,
        )
        .with_expanded(!is_done)
    }

    pub fn streaming_assistant() -> Self {
        Self::new(
            ChatRole::Assistant,
            String::new(),
            MessageVariant::Text,
            true,
        )
        .with_reasoning_expanded(true)
    }

    pub fn from_history(id: impl Into<String>, role: ChatRole, content: impl Into<String>) -> Self {
        Self::new(role, content, MessageVariant::Text, false).with_id(id)
    }

    fn new(
        role: ChatRole,
        content: impl Into<String>,
        variant: MessageVariant,
        is_streaming: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role,
            content: content.into(),
            reasoning_content: String::new(),
            variant,
            is_streaming,
            is_expanded: true,
            is_reasoning_expanded: false,
            cached_content_hash: None,
            extension: E::default(),
        }
    }
}

impl<E: MessageExtension> ChatMessageUIGeneric<E> {
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_variant(mut self, variant: MessageVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn with_streaming(mut self, is_streaming: bool) -> Self {
        self.is_streaming = is_streaming;
        self
    }

    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = content.into();
        self.cached_content_hash = None;
        self
    }

    pub fn with_reasoning_content(mut self, content: impl Into<String>) -> Self {
        self.reasoning_content = content.into();
        self
    }

    fn with_expanded(mut self, is_expanded: bool) -> Self {
        self.is_expanded = is_expanded;
        self
    }

    fn with_reasoning_expanded(mut self, is_reasoning_expanded: bool) -> Self {
        self.is_reasoning_expanded = is_reasoning_expanded;
        self
    }

    pub fn content_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.content.hash(&mut hasher);
        hasher.finish()
    }

    pub fn is_cache_valid(&self) -> bool {
        self.cached_content_hash
            .map(|hash| hash == self.content_hash())
            .unwrap_or(false)
    }

    pub fn update_cache(&mut self) {
        self.cached_content_hash = Some(self.content_hash());
    }

    pub fn finalize_streaming(&mut self) {
        self.is_streaming = false;
        self.cached_content_hash = None;
        self.extension.on_finalize_streaming();
    }

    pub fn clear_cache(&mut self) {
        self.cached_content_hash = None;
        self.extension.clear_cache();
    }
}

impl MessageVariant {
    pub fn card_kind(&self) -> Option<&str> {
        match self {
            MessageVariant::Card { kind } => Some(kind.as_str()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_message_keeps_kind_and_content() {
        let msg = ChatMessageUI::card("chart", "{\"chart_type\":\"bar\"}");

        assert_eq!(msg.role, ChatRole::Assistant);
        assert_eq!(msg.content, "{\"chart_type\":\"bar\"}");
        assert_eq!(msg.variant.card_kind(), Some("chart"));
        assert!(!msg.is_streaming);
    }

    #[test]
    fn status_message_streams_until_done() {
        let running = ChatMessageUI::status("查询中", false);
        let done = ChatMessageUI::status("完成", true);

        assert!(running.is_streaming);
        assert!(!done.is_streaming);
    }

    #[test]
    fn content_hash_changes_after_content_update() {
        let mut msg = ChatMessageUI::assistant("old");
        let old_hash = msg.content_hash();

        msg = msg.with_content("new");

        assert_ne!(old_hash, msg.content_hash());
    }
}
