use crate::{ChatMessageUI, SessionSummary};

/// 通用 ChatView 的可测试状态层。
#[derive(Clone, Debug)]
pub struct ChatViewState {
    messages: Vec<ChatMessageUI>,
    sessions: Vec<SessionSummary>,
    current_session: Option<String>,
}

impl Default for ChatViewState {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatViewState {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            sessions: Vec::new(),
            current_session: None,
        }
    }

    pub fn with_messages(messages: Vec<ChatMessageUI>) -> Self {
        Self {
            messages,
            sessions: Vec::new(),
            current_session: None,
        }
    }

    pub fn with_sessions(mut self, sessions: Vec<SessionSummary>) -> Self {
        self.replace_sessions(sessions);
        self
    }

    pub fn messages(&self) -> &[ChatMessageUI] {
        &self.messages
    }

    pub fn sessions(&self) -> &[SessionSummary] {
        &self.sessions
    }

    pub fn current_session_id(&self) -> Option<&str> {
        self.current_session.as_deref()
    }

    pub fn replace_messages(&mut self, messages: Vec<ChatMessageUI>) {
        self.messages = messages;
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }

    pub fn replace_sessions(&mut self, sessions: Vec<SessionSummary>) {
        self.current_session = sessions.first().map(|session| session.id.clone());
        self.sessions = sessions;
    }

    pub fn select_session(&mut self, id: &str) -> bool {
        if !self.sessions.iter().any(|session| session.id == id) {
            return false;
        }
        self.current_session = Some(id.to_string());
        true
    }

    pub fn push_user(&mut self, content: impl Into<String>) -> String {
        self.push(ChatMessageUI::user(content))
    }

    pub fn push_assistant(&mut self, content: impl Into<String>) -> String {
        self.push(ChatMessageUI::assistant(content))
    }

    pub fn push_assistant_with_reasoning(
        &mut self,
        content: impl Into<String>,
        reasoning: impl Into<String>,
    ) -> String {
        self.push(ChatMessageUI::assistant(content).with_reasoning_content(reasoning))
    }

    pub fn push_system(&mut self, content: impl Into<String>) -> String {
        self.push(ChatMessageUI::system(content))
    }

    pub fn push_card(&mut self, kind: impl Into<String>, content: impl Into<String>) -> String {
        self.push(ChatMessageUI::card(kind, content))
    }

    pub fn push_status(&mut self, title: impl Into<String>, is_done: bool) -> String {
        self.push(ChatMessageUI::status(title, is_done))
    }

    pub fn push_streaming_assistant(&mut self) -> String {
        self.push(ChatMessageUI::streaming_assistant())
    }

    pub fn append_to_message(&mut self, id: &str, delta: &str) -> bool {
        let Some(message) = self.messages.iter_mut().find(|message| message.id == id) else {
            return false;
        };
        message.content.push_str(delta);
        true
    }

    pub fn append_reasoning_to_message(&mut self, id: &str, delta: &str) -> bool {
        let Some(message) = self.messages.iter_mut().find(|message| message.id == id) else {
            return false;
        };
        message.reasoning_content.push_str(delta);
        true
    }

    pub fn finalize_message(&mut self, id: &str) -> bool {
        let Some(message) = self.messages.iter_mut().find(|message| message.id == id) else {
            return false;
        };
        message.finalize_streaming();
        true
    }

    fn push(&mut self, message: ChatMessageUI) -> String {
        let id = message.id.clone();
        self.messages.push(message);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatRole, MessageVariant};

    #[test]
    fn state_accepts_external_initial_messages() {
        let state = ChatViewState::with_messages(vec![
            ChatMessageUI::user("show tables"),
            ChatMessageUI::assistant("ok"),
        ]);

        assert_eq!(2, state.messages().len());
        assert_eq!(ChatRole::User, state.messages()[0].role);
    }

    #[test]
    fn state_appends_text_card_and_streaming_messages() {
        let mut state = ChatViewState::new();

        let stream_id = state.push_streaming_assistant();
        state.append_to_message(&stream_id, "hello");
        state.push_card("json", r#"{"ok":true}"#);
        state.finalize_message(&stream_id);

        assert_eq!("hello", state.messages()[0].content);
        assert!(!state.messages()[0].is_streaming);
        assert_eq!(Some("json"), state.messages()[1].variant.card_kind());
    }

    #[test]
    fn state_switches_and_clears_sessions() {
        let mut state = ChatViewState::new();

        state.replace_sessions(vec![
            SessionSummary::new("s1", "one", 1),
            SessionSummary::new("s2", "two", 2),
        ]);
        state.select_session("s2");
        state.push_user("hello");
        state.clear_messages();

        assert_eq!(Some("s2"), state.current_session_id());
        assert!(state.messages().is_empty());
        assert_eq!(2, state.sessions().len());
    }

    #[test]
    fn state_status_message_uses_status_variant() {
        let mut state = ChatViewState::new();

        state.push_status("running", false);

        assert!(matches!(
            state.messages()[0].variant,
            MessageVariant::Status { ref title, is_done: false } if title == "running"
        ));
    }

    #[test]
    fn state_can_store_assistant_reasoning_separately_from_content() {
        let mut state = ChatViewState::new();

        state.push_assistant_with_reasoning("最终回答", "内部推理");

        assert_eq!("最终回答", state.messages()[0].content);
        assert_eq!("内部推理", state.messages()[0].reasoning_content);
    }

    #[test]
    fn state_can_append_reasoning_to_streaming_message() {
        let mut state = ChatViewState::new();

        let id = state.push_streaming_assistant();
        assert!(state.append_reasoning_to_message(&id, "先分析"));
        assert!(state.append_to_message(&id, "再回答"));

        assert_eq!("再回答", state.messages()[0].content);
        assert_eq!("先分析", state.messages()[0].reasoning_content);
        assert!(state.messages()[0].is_reasoning_expanded);
    }
}
