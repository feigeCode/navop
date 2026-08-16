use crate::http::KeyValue;
use crate::socket_io::{EnginePacket, SocketPacket, display_socket_packet, parse_engine_packet};
use crate::websocket::{
    ConnectionEvent, ConnectionEventEnvelope, MessageDirection, Timeline, TimelineEntry,
};
use rust_i18n::t;

pub(super) const TIMELINE_LIMIT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SocketIoState {
    Disconnected,
    Connecting,
    EngineOpen,
    NamespaceConnecting,
    Connected,
    Closing,
    Failed(String),
}

impl SocketIoState {
    pub(super) fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Connecting
                | Self::EngineOpen
                | Self::NamespaceConnecting
                | Self::Connected
                | Self::Closing
        )
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

pub(super) struct SocketIoTransition {
    pub(super) disposition: EventDisposition,
    pub(super) outbound: Option<String>,
}

impl SocketIoTransition {
    fn continue_with(outbound: Option<String>) -> Self {
        Self {
            disposition: EventDisposition::Continue,
            outbound,
        }
    }

    fn terminal() -> Self {
        Self {
            disposition: EventDisposition::Terminal,
            outbound: None,
        }
    }

    fn ignored() -> Self {
        Self {
            disposition: EventDisposition::Ignored,
            outbound: None,
        }
    }
}

pub(super) struct SocketIoSession {
    pub(super) state: SocketIoState,
    pub(super) timeline: Timeline,
    pub(super) handshake_status: Option<u16>,
    pub(super) handshake_headers: Vec<KeyValue>,
    pub(super) engine_sid: Option<String>,
    pub(super) namespace_sid: Option<String>,
    pub(super) ping_interval: Option<u64>,
    pub(super) ping_timeout: Option<u64>,
    generation: u64,
}

impl SocketIoSession {
    pub(super) fn new() -> Self {
        Self {
            state: SocketIoState::Disconnected,
            timeline: Timeline::new(TIMELINE_LIMIT),
            handshake_status: None,
            handshake_headers: Vec::new(),
            engine_sid: None,
            namespace_sid: None,
            ping_interval: None,
            ping_timeout: None,
            generation: 0,
        }
    }

    pub(super) fn begin(&mut self, generation: u64) {
        self.generation = generation;
        self.state = SocketIoState::Connecting;
        self.timeline = Timeline::new(TIMELINE_LIMIT);
        self.handshake_status = None;
        self.handshake_headers.clear();
        self.engine_sid = None;
        self.namespace_sid = None;
        self.ping_interval = None;
        self.ping_timeout = None;
        self.timeline.push(TimelineEntry::system(
            t!("ApiTest.socketio_opening").to_string(),
        ));
    }

    pub(super) fn set_closing(&mut self) {
        if self.state.is_active() {
            self.state = SocketIoState::Closing;
            self.timeline.push(TimelineEntry::system(
                t!("ApiTest.socketio_closing").to_string(),
            ));
        }
    }

    pub(super) fn cancel(&mut self, generation: u64) {
        self.generation = generation;
        self.state = SocketIoState::Disconnected;
        self.timeline = Timeline::new(TIMELINE_LIMIT);
        self.handshake_status = None;
        self.handshake_headers.clear();
        self.engine_sid = None;
        self.namespace_sid = None;
        self.ping_interval = None;
        self.ping_timeout = None;
    }

    pub(super) fn apply_event(
        &mut self,
        generation: u64,
        event: ConnectionEventEnvelope,
    ) -> SocketIoTransition {
        if generation != self.generation {
            return SocketIoTransition::ignored();
        }
        match event {
            ConnectionEventEnvelope::Connecting => {
                if !matches!(self.state, SocketIoState::Closing) {
                    self.state = SocketIoState::Connecting;
                }
                SocketIoTransition::continue_with(None)
            }
            ConnectionEventEnvelope::Connected { status, headers } => {
                self.handshake_status = Some(status);
                self.handshake_headers = headers
                    .into_iter()
                    .map(|(key, value)| KeyValue::new(key, value))
                    .collect();
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.socketio_ws_connected", status = status).to_string(),
                ));
                SocketIoTransition::continue_with(None)
            }
            ConnectionEventEnvelope::Message(event) => self.apply_message(event),
            ConnectionEventEnvelope::Error(error) => {
                self.fail(error);
                SocketIoTransition::terminal()
            }
        }
    }

    fn apply_message(&mut self, event: ConnectionEvent) -> SocketIoTransition {
        match event {
            ConnectionEvent::Text(text) => self.apply_text(&text),
            ConnectionEvent::Binary(bytes) => {
                self.timeline
                    .push(TimelineEntry::binary(MessageDirection::Received, bytes));
                SocketIoTransition::continue_with(None)
            }
            ConnectionEvent::Ping(bytes) => {
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.websocket_ping_bytes", count = bytes.len()).to_string(),
                ));
                SocketIoTransition::continue_with(None)
            }
            ConnectionEvent::Pong(bytes) => {
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.websocket_pong_bytes", count = bytes.len()).to_string(),
                ));
                SocketIoTransition::continue_with(None)
            }
            ConnectionEvent::Closed { code, reason } => {
                self.state = SocketIoState::Disconnected;
                let code = code.map_or_else(
                    || t!("ApiTest.websocket_no_close_code").to_string(),
                    |code| code.to_string(),
                );
                self.timeline.push(TimelineEntry::system(format!(
                    "{} · {code}{}",
                    t!("ApiTest.socketio_disconnected"),
                    if reason.is_empty() {
                        String::new()
                    } else {
                        format!(" · {reason}")
                    }
                )));
                SocketIoTransition::terminal()
            }
            ConnectionEvent::Error(error) => {
                self.fail(error);
                SocketIoTransition::terminal()
            }
        }
    }

    fn apply_text(&mut self, text: &str) -> SocketIoTransition {
        let packet = match parse_engine_packet(text) {
            Ok(packet) => packet,
            Err(error) => {
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.socketio_protocol_error", error = error.to_string()).to_string(),
                ));
                return SocketIoTransition::continue_with(None);
            }
        };
        match packet {
            EnginePacket::Open(open) => {
                self.state = SocketIoState::EngineOpen;
                self.engine_sid = Some(open.sid.clone());
                self.ping_interval = Some(open.ping_interval);
                self.ping_timeout = Some(open.ping_timeout);
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.socketio_engine_open", sid = open.sid).to_string(),
                ));
                self.state = SocketIoState::NamespaceConnecting;
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.socketio_namespace_connecting").to_string(),
                ));
                SocketIoTransition::continue_with(Some("40".to_string()))
            }
            EnginePacket::Close => {
                self.state = SocketIoState::Disconnected;
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.socketio_disconnected").to_string(),
                ));
                SocketIoTransition::terminal()
            }
            EnginePacket::Ping(data) => {
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.socketio_ping").to_string(),
                ));
                SocketIoTransition::continue_with(Some(format!("3{data}")))
            }
            EnginePacket::Pong(_) => {
                self.timeline.push(TimelineEntry::system(
                    t!("ApiTest.socketio_pong").to_string(),
                ));
                SocketIoTransition::continue_with(None)
            }
            EnginePacket::Message(packet) => self.apply_socket_packet(packet),
            EnginePacket::Upgrade | EnginePacket::Noop => SocketIoTransition::continue_with(None),
        }
    }

    fn apply_socket_packet(&mut self, packet: SocketPacket) -> SocketIoTransition {
        match &packet {
            SocketPacket::Connect {
                namespace, data, ..
            } => {
                self.state = SocketIoState::Connected;
                self.namespace_sid = data
                    .as_ref()
                    .and_then(|value| value.get("sid"))
                    .and_then(ValueExt::as_string);
                self.timeline.push(TimelineEntry::system(
                    t!(
                        "ApiTest.socketio_connected_namespace",
                        namespace = namespace.clone()
                    )
                    .to_string(),
                ));
                SocketIoTransition::continue_with(None)
            }
            SocketPacket::Disconnect { namespace } => {
                self.state = SocketIoState::Disconnected;
                self.timeline.push(TimelineEntry::system(
                    t!(
                        "ApiTest.socketio_namespace_disconnected",
                        namespace = namespace.clone()
                    )
                    .to_string(),
                ));
                SocketIoTransition::terminal()
            }
            SocketPacket::Event { .. } | SocketPacket::Ack { .. } => {
                self.timeline.push(TimelineEntry::text(
                    MessageDirection::Received,
                    display_socket_packet(&packet),
                ));
                SocketIoTransition::continue_with(None)
            }
            SocketPacket::ConnectError { data, .. } => {
                let error = data
                    .as_ref()
                    .map(|value| {
                        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
                    })
                    .unwrap_or_else(|| t!("ApiTest.connection_failed").to_string());
                self.fail(error);
                SocketIoTransition::terminal()
            }
        }
    }

    pub(super) fn push_sent_text(&mut self, text: String) {
        self.timeline
            .push(TimelineEntry::text(MessageDirection::Sent, text));
    }

    pub(super) fn fail(&mut self, error: String) {
        self.state = SocketIoState::Failed(error.clone());
        self.timeline.push(TimelineEntry::system(
            t!("ApiTest.socketio_connection_error", error = error).to_string(),
        ));
    }

    pub(super) fn finish(&mut self, generation: u64) -> bool {
        if generation != self.generation {
            return false;
        }
        if !matches!(
            self.state,
            SocketIoState::Disconnected | SocketIoState::Failed(_)
        ) {
            self.state = SocketIoState::Disconnected;
            self.timeline.push(TimelineEntry::system(
                t!("ApiTest.socketio_transport_ended").to_string(),
            ));
        }
        true
    }
}

trait ValueExt {
    fn as_string(&self) -> Option<String>;
}

impl ValueExt for serde_json::Value {
    fn as_string(&self) -> Option<String> {
        self.as_str().map(ToOwned::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_packet() -> ConnectionEventEnvelope {
        ConnectionEventEnvelope::Message(ConnectionEvent::Text(
            r#"0{"sid":"engine-1","upgrades":[],"pingInterval":25000,"pingTimeout":20000}"#.into(),
        ))
    }

    #[test]
    fn websocket_upgrade_does_not_mark_socket_io_connected() {
        let mut session = SocketIoSession::new();
        session.begin(3);
        let transition = session.apply_event(
            3,
            ConnectionEventEnvelope::Connected {
                status: 101,
                headers: Vec::new(),
            },
        );

        assert_eq!(transition.disposition, EventDisposition::Continue);
        assert_eq!(session.state, SocketIoState::Connecting);
        assert!(!session.state.is_connected());
    }

    #[test]
    fn open_connect_and_ping_drive_the_eio4_lifecycle() {
        let mut session = SocketIoSession::new();
        session.begin(4);

        let transition = session.apply_event(4, open_packet());
        assert_eq!(transition.outbound.as_deref(), Some("40"));
        assert_eq!(session.state, SocketIoState::NamespaceConnecting);
        assert_eq!(session.engine_sid.as_deref(), Some("engine-1"));

        session.apply_event(
            4,
            ConnectionEventEnvelope::Message(ConnectionEvent::Text(
                r#"40{"sid":"namespace-1"}"#.into(),
            )),
        );
        assert_eq!(session.state, SocketIoState::Connected);
        assert_eq!(session.namespace_sid.as_deref(), Some("namespace-1"));

        let transition = session.apply_event(
            4,
            ConnectionEventEnvelope::Message(ConnectionEvent::Text("2heartbeat".into())),
        );
        assert_eq!(transition.outbound.as_deref(), Some("3heartbeat"));
    }

    #[test]
    fn stale_events_cannot_mutate_a_new_session() {
        let mut session = SocketIoSession::new();
        session.begin(8);

        let transition = session.apply_event(7, open_packet());

        assert_eq!(transition.disposition, EventDisposition::Ignored);
        assert_eq!(session.state, SocketIoState::Connecting);
        assert!(session.engine_sid.is_none());
    }

    #[test]
    fn late_connecting_event_cannot_reopen_a_closing_session() {
        let mut session = SocketIoSession::new();
        session.begin(9);
        session.set_closing();

        let transition = session.apply_event(9, ConnectionEventEnvelope::Connecting);

        assert_eq!(transition.disposition, EventDisposition::Continue);
        assert_eq!(session.state, SocketIoState::Closing);
    }
}
