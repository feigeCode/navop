use agent_runtime::RuntimeEvent;

use crate::{AcpError, ChatMessageUI, MessageVariant};

use super::AgentTranscript;

impl AgentTranscript {
    pub(crate) fn set_acp_status(&mut self, title: impl Into<String>) {
        let title = title.into();
        if let Some(message) = self.acp_status_message_mut() {
            message.variant = MessageVariant::Status {
                title,
                is_done: false,
            };
            message.is_streaming = true;
            return;
        }
        let message = ChatMessageUI::status(title, false);
        self.acp_status_id = Some(message.id.clone());
        self.messages.push(message);
    }

    pub(crate) fn set_acp_error(&mut self, error: &AcpError) {
        self.finish_active_status();
        let content = format!("⚠️ {error}");
        if let Some(id) = self.acp_status_id.take()
            && let Some(index) = self.messages.iter().position(|message| message.id == id)
        {
            self.messages[index] = ChatMessageUI::system(content).with_id(id);
            return;
        }
        self.messages.push(ChatMessageUI::system(content));
    }

    pub(crate) fn apply_acp_failure(&mut self, event: &RuntimeEvent, error: &AcpError) -> bool {
        if let Some(key) = super::terminal_event_key(event)
            && !self.terminal_events.insert(key)
        {
            return false;
        }
        self.streaming_id = None;
        self.set_acp_error(error);
        true
    }

    pub(crate) fn clear_acp_status(&mut self) {
        if let Some(id) = self.acp_status_id.take() {
            self.messages.retain(|message| message.id != id);
        }
    }

    #[cfg(test)]
    pub(crate) fn acp_status_count(&self) -> usize {
        self.acp_status_id
            .iter()
            .filter(|id| self.messages.iter().any(|message| &message.id == *id))
            .count()
    }

    #[cfg(test)]
    pub(crate) fn acp_status_text(&self) -> Option<&str> {
        let id = self.acp_status_id.as_ref()?;
        let message = self.messages.iter().find(|message| &message.id == id)?;
        match &message.variant {
            MessageVariant::Status { title, .. } => Some(title),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn pending_status_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|message| {
                matches!(
                    message.variant,
                    MessageVariant::Status { is_done: false, .. }
                )
            })
            .count()
    }

    #[cfg(test)]
    pub(crate) fn last_message_content(&self) -> Option<&str> {
        self.messages.last().map(|message| message.content.as_str())
    }

    fn acp_status_message_mut(&mut self) -> Option<&mut ChatMessageUI> {
        let id = self.acp_status_id.as_ref()?;
        self.messages.iter_mut().find(|message| &message.id == id)
    }
}
