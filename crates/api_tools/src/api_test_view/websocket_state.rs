use crate::http::KeyValue;
use crate::websocket::{
    ConnectionEvent, ConnectionEventEnvelope, MessageDirection, Timeline, TimelineEntry,
};
use rust_i18n::t;

pub(super) const TIMELINE_LIMIT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WebSocketState {
    Disconnected,
    Connecting,
    Connected,
    Closing,
    Failed(String),
}

impl WebSocketState {
    pub(super) fn is_active(&self) -> bool {
        matches!(self, Self::Connecting | Self::Connected | Self::Closing)
    }

    pub(super) fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EventDisposition {
    Ignored,
    Continue,
    Terminal,
}

pub(super) struct WebSocketSession {
    pub(super) state: WebSocketState,
    pub(super) timeline: Timeline,
    pub(super) handshake_status: Option<u16>,
    pub(super) handshake_headers: Vec<KeyValue>,
    generation: u64,
}

impl WebSocketSession {
    pub(super) fn new() -> Self {
        Self {
            state: WebSocketState::Disconnected,
            timeline: Timeline::new(TIMELINE_LIMIT),
            handshake_status: None,
            handshake_headers: Vec::new(),
            generation: 0,
        }
    }

    pub(super) fn begin(&mut self, generation: u64) {
        self.generation = generation;
        self.state = WebSocketState::Connecting;
        self.timeline = Timeline::new(TIMELINE_LIMIT);
        self.handshake_status = None;
        self.handshake_headers.clear();
        self.timeline.push(TimelineEntry::system(
            t!("ApiTest.websocket_opening").to_string(),
        ));
    }

    pub(super) fn set_closing(&mut self) {
        if self.state.is_active() {
            self.state = WebSocketState::Closing;
            self.timeline.push(TimelineEntry::system(
                t!("ApiTest.websocket_closing").to_string(),
            ));
        }
    }

    pub(super) fn cancel(&mut self, generation: u64) {
        self.generation = generation;
        self.state = WebSocketState::Disconnected;
        self.timeline = Timeline::new(TIMELINE_LIMIT);
        self.handshake_status = None;
        self.handshake_headers.clear();
    }

    pub(super) fn apply_event(
        &mut self,
        generation: u64,
        event: ConnectionEventEnvelope,
    ) -> EventDisposition {
        if generation != self.generation {
            return EventDisposition::Ignored;
        }
        match event {
            ConnectionEventEnvelope::Connecting => self.state = WebSocketState::Connecting,
            ConnectionEventEnvelope::Connected { status, headers } => {
                self.state = WebSocketState::Connected;
                self.handshake_status = Some(status);
                self.handshake_headers = headers
                    .into_iter()
                    .map(|(key, value)| KeyValue::new(key, value))
                    .collect();
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.websocket_connected_status", status = status).to_string(),
                ));
            }
            ConnectionEventEnvelope::Message(event) => {
                return self.apply_message(event);
            }
            ConnectionEventEnvelope::Error(error) => {
                self.state = WebSocketState::Failed(error.clone());
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.websocket_connection_error", error = error).to_string(),
                ));
                return EventDisposition::Terminal;
            }
        }
        EventDisposition::Continue
    }

    fn apply_message(&mut self, event: ConnectionEvent) -> EventDisposition {
        match event {
            ConnectionEvent::Text(text) => self
                .timeline
                .push(TimelineEntry::text(MessageDirection::Received, text)),
            ConnectionEvent::Binary(bytes) => self
                .timeline
                .push(TimelineEntry::binary(MessageDirection::Received, bytes)),
            ConnectionEvent::Ping(bytes) => self.timeline.push(TimelineEntry::system(
                t!("ApiTest.websocket_ping_bytes", count = bytes.len()).to_string(),
            )),
            ConnectionEvent::Pong(bytes) => self.timeline.push(TimelineEntry::system(
                t!("ApiTest.websocket_pong_bytes", count = bytes.len()).to_string(),
            )),
            ConnectionEvent::Closed { code, reason } => {
                self.state = WebSocketState::Disconnected;
                let code = code.map_or_else(
                    || t!("ApiTest.websocket_no_close_code").to_string(),
                    |code| code.to_string(),
                );
                self.timeline.push(TimelineEntry::system(format!(
                    "{} · {code}{}",
                    t!("ApiTest.websocket_closed"),
                    if reason.is_empty() {
                        String::new()
                    } else {
                        format!(" · {reason}")
                    }
                )));
                return EventDisposition::Terminal;
            }
            ConnectionEvent::Error(error) => {
                self.state = WebSocketState::Failed(error.clone());
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.websocket_protocol_error", error = error).to_string(),
                ));
                return EventDisposition::Terminal;
            }
        }
        EventDisposition::Continue
    }

    pub(super) fn push_sent_text(&mut self, text: String) {
        self.timeline
            .push(TimelineEntry::text(MessageDirection::Sent, text));
    }

    pub(super) fn finish(&mut self, generation: u64) -> bool {
        if generation != self.generation {
            return false;
        }
        if !matches!(
            self.state,
            WebSocketState::Disconnected | WebSocketState::Failed(_)
        ) {
            self.state = WebSocketState::Disconnected;
            self.timeline.push(TimelineEntry::system(
                t!("ApiTest.websocket_transport_ended").to_string(),
            ));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_events_cannot_mutate_a_new_connection() {
        let mut session = WebSocketSession::new();
        session.begin(2);

        assert_eq!(
            session.apply_event(
                1,
                ConnectionEventEnvelope::Message(ConnectionEvent::Text("stale".into()))
            ),
            EventDisposition::Ignored
        );
        assert_eq!(session.timeline.entries().len(), 1);
        assert_eq!(session.state, WebSocketState::Connecting);
    }

    #[test]
    fn connected_messages_and_close_are_reduced_into_the_timeline() {
        let mut session = WebSocketSession::new();
        session.begin(3);
        session.apply_event(
            3,
            ConnectionEventEnvelope::Connected {
                status: 101,
                headers: vec![("upgrade".into(), "websocket".into())],
            },
        );
        session.apply_event(
            3,
            ConnectionEventEnvelope::Message(ConnectionEvent::Text("hello".into())),
        );
        let disposition = session.apply_event(
            3,
            ConnectionEventEnvelope::Message(ConnectionEvent::Closed {
                code: Some(1000),
                reason: "done".into(),
            }),
        );

        assert_eq!(disposition, EventDisposition::Terminal);
        assert_eq!(session.state, WebSocketState::Disconnected);
        assert_eq!(session.handshake_status, Some(101));
        assert_eq!(session.handshake_headers.len(), 1);
        assert_eq!(session.timeline.entries().len(), 4);
    }

    #[test]
    fn failed_state_survives_receiver_completion() {
        let mut session = WebSocketSession::new();
        session.begin(4);
        session.apply_event(4, ConnectionEventEnvelope::Error("boom".into()));

        assert!(session.finish(4));
        assert_eq!(session.state, WebSocketState::Failed("boom".into()));
    }
}
