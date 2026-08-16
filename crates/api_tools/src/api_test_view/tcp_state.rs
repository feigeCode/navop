use rust_i18n::t;

use crate::tcp::ConnectionEvent;
use crate::websocket::{MessageDirection, Timeline, TimelineEntry};

pub(super) const TIMELINE_LIMIT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TcpState {
    Disconnected,
    Connecting,
    Connected,
    Closing,
    Failed(String),
}

impl TcpState {
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

pub(super) struct TcpSession {
    pub(super) state: TcpState,
    pub(super) timeline: Timeline,
    pub(super) peer: Option<String>,
    generation: u64,
}

impl TcpSession {
    pub(super) fn new() -> Self {
        Self {
            state: TcpState::Disconnected,
            timeline: Timeline::new(TIMELINE_LIMIT),
            peer: None,
            generation: 0,
        }
    }

    pub(super) fn begin(&mut self, generation: u64) {
        self.generation = generation;
        self.state = TcpState::Connecting;
        self.timeline = Timeline::new(TIMELINE_LIMIT);
        self.peer = None;
        self.timeline
            .push(TimelineEntry::system(t!("ApiTest.tcp_opening").to_string()));
    }

    pub(super) fn set_closing(&mut self) {
        if self.state.is_active() {
            self.state = TcpState::Closing;
            self.timeline
                .push(TimelineEntry::system(t!("ApiTest.tcp_closing").to_string()));
        }
    }

    pub(super) fn cancel(&mut self, generation: u64) {
        self.generation = generation;
        self.state = TcpState::Disconnected;
        self.timeline = Timeline::new(TIMELINE_LIMIT);
        self.peer = None;
    }

    pub(super) fn apply_event(
        &mut self,
        generation: u64,
        event: ConnectionEvent,
    ) -> EventDisposition {
        if generation != self.generation {
            return EventDisposition::Ignored;
        }
        match event {
            ConnectionEvent::Connecting => self.state = TcpState::Connecting,
            ConnectionEvent::Connected { peer } => {
                self.state = TcpState::Connected;
                self.peer = Some(peer.clone());
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.tcp_connected_peer", peer = peer).to_string(),
                ));
            }
            ConnectionEvent::Data(bytes) => {
                self.timeline
                    .push(payload_entry(MessageDirection::Received, bytes));
            }
            ConnectionEvent::Closed => {
                self.state = TcpState::Disconnected;
                self.timeline
                    .push(TimelineEntry::system(t!("ApiTest.tcp_closed").to_string()));
                return EventDisposition::Terminal;
            }
            ConnectionEvent::Error(error) => {
                self.state = TcpState::Failed(error.clone());
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.tcp_connection_error", error = error).to_string(),
                ));
                return EventDisposition::Terminal;
            }
        }
        EventDisposition::Continue
    }

    pub(super) fn push_sent(&mut self, bytes: Vec<u8>) {
        self.timeline
            .push(payload_entry(MessageDirection::Sent, bytes));
    }

    pub(super) fn finish(&mut self, generation: u64) -> bool {
        if generation != self.generation {
            return false;
        }
        if !matches!(self.state, TcpState::Disconnected | TcpState::Failed(_)) {
            self.state = TcpState::Disconnected;
            self.timeline.push(TimelineEntry::system(
                t!("ApiTest.tcp_transport_ended").to_string(),
            ));
        }
        true
    }
}

fn payload_entry(direction: MessageDirection, bytes: Vec<u8>) -> TimelineEntry {
    match readable_text(&bytes) {
        Some(text) => TimelineEntry::text(direction, text),
        None => TimelineEntry::binary(direction, bytes),
    }
}

fn readable_text(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    text.chars()
        .all(|character| !character.is_control() || matches!(character, '\r' | '\n' | '\t'))
        .then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::{EventDisposition, TcpSession, TcpState};
    use crate::tcp::ConnectionEvent;

    #[test]
    fn stale_events_cannot_mutate_a_new_connection() {
        let mut session = TcpSession::new();
        session.begin(2);

        assert_eq!(
            session.apply_event(1, ConnectionEvent::Data(b"stale".to_vec())),
            EventDisposition::Ignored
        );
        assert_eq!(session.timeline.entries().len(), 1);
        assert_eq!(session.state, TcpState::Connecting);
    }

    #[test]
    fn connected_data_and_close_are_reduced_into_the_timeline() {
        let mut session = TcpSession::new();
        session.begin(3);
        session.apply_event(
            3,
            ConnectionEvent::Connected {
                peer: "127.0.0.1:9000".into(),
            },
        );
        session.apply_event(3, ConnectionEvent::Data(vec![0, 255, 16]));
        let disposition = session.apply_event(3, ConnectionEvent::Closed);

        assert_eq!(disposition, EventDisposition::Terminal);
        assert_eq!(session.state, TcpState::Disconnected);
        assert_eq!(session.peer.as_deref(), Some("127.0.0.1:9000"));
        assert_eq!(session.timeline.entries().len(), 4);
        assert_eq!(
            session.timeline.entries()[2].binary_bytes(),
            Some([0, 255, 16].as_slice())
        );
    }

    #[test]
    fn failed_state_survives_receiver_completion() {
        let mut session = TcpSession::new();
        session.begin(4);
        session.apply_event(4, ConnectionEvent::Error("boom".into()));

        assert!(session.finish(4));
        assert_eq!(session.state, TcpState::Failed("boom".into()));
    }
}
