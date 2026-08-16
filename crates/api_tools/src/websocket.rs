//! WebSocket 请求、帧与会话模型。

use anyhow::{Context as _, Result};
use tokio_tungstenite::tungstenite::{
    Message,
    client::IntoClientRequest,
    http::{HeaderName, HeaderValue},
};

use crate::http::{PreparedRequest, RequestMethod, normalize_url_with_default};

#[path = "websocket_transport.rs"]
mod transport;

pub use transport::{ConnectionCommand, ConnectionEventEnvelope, ConnectionTask, start_connection};

const DEFAULT_WEBSOCKET_SCHEME: &str = "ws";
const BINARY_PREVIEW_BYTES: usize = 16;
const RESERVED_HANDSHAKE_HEADERS: [&str; 5] = [
    "connection",
    "host",
    "sec-websocket-key",
    "sec-websocket-version",
    "upgrade",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Closed { code: Option<u16>, reason: String },
    Error(String),
}

pub fn event_from_message(message: Message) -> ConnectionEvent {
    match message {
        Message::Text(text) => ConnectionEvent::Text(text.to_string()),
        Message::Binary(bytes) => ConnectionEvent::Binary(bytes.to_vec()),
        Message::Ping(bytes) => ConnectionEvent::Ping(bytes.to_vec()),
        Message::Pong(bytes) => ConnectionEvent::Pong(bytes.to_vec()),
        Message::Close(frame) => frame.map_or_else(
            || ConnectionEvent::Closed {
                code: None,
                reason: String::new(),
            },
            |frame| ConnectionEvent::Closed {
                code: Some(frame.code.into()),
                reason: frame.reason.to_string(),
            },
        ),
        Message::Frame(_) => ConnectionEvent::Error("unexpected raw WebSocket frame".into()),
    }
}

pub fn prepare_websocket_request(mut request: PreparedRequest) -> PreparedRequest {
    request.url = normalize_url_with_default(&request.url, DEFAULT_WEBSOCKET_SCHEME);
    request.method = RequestMethod::Get;
    request.body.clear();
    request
}

pub fn build_client_request(
    request: &PreparedRequest,
) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request> {
    let mut client_request = request
        .url
        .as_str()
        .into_client_request()
        .context("invalid WebSocket URL")?;
    append_custom_headers(&mut client_request, &request.headers)?;
    Ok(client_request)
}

fn append_custom_headers(
    request: &mut tokio_tungstenite::tungstenite::handshake::client::Request,
    headers: &[(String, String)],
) -> Result<()> {
    for (name, value) in headers {
        if is_reserved_handshake_header(name) {
            continue;
        }
        let name = HeaderName::from_bytes(name.as_bytes())
            .with_context(|| format!("invalid WebSocket header name: {name}"))?;
        let value = HeaderValue::try_from(value)
            .with_context(|| format!("invalid WebSocket header value for {name}"))?;
        request.headers_mut().append(name, value);
    }
    Ok(())
}

fn is_reserved_handshake_header(name: &str) -> bool {
    RESERVED_HANDSHAKE_HEADERS
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageDirection {
    Sent,
    Received,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TimelinePayload {
    Text(String),
    Binary(Vec<u8>),
    Status(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEntry {
    direction: MessageDirection,
    payload: TimelinePayload,
}

impl TimelineEntry {
    pub fn text(direction: MessageDirection, text: impl Into<String>) -> Self {
        Self {
            direction,
            payload: TimelinePayload::Text(text.into()),
        }
    }

    pub fn binary(direction: MessageDirection, bytes: Vec<u8>) -> Self {
        Self {
            direction,
            payload: TimelinePayload::Binary(bytes),
        }
    }

    pub fn system(status: impl Into<String>) -> Self {
        Self {
            direction: MessageDirection::System,
            payload: TimelinePayload::Status(status.into()),
        }
    }

    pub fn direction(&self) -> MessageDirection {
        self.direction
    }

    #[cfg(test)]
    pub fn binary_bytes(&self) -> Option<&[u8]> {
        match &self.payload {
            TimelinePayload::Binary(bytes) => Some(bytes),
            TimelinePayload::Text(_) | TimelinePayload::Status(_) => None,
        }
    }

    pub fn display_text(&self) -> String {
        match &self.payload {
            TimelinePayload::Text(text) | TimelinePayload::Status(text) => text.clone(),
            TimelinePayload::Binary(bytes) => binary_display(bytes),
        }
    }
}

pub struct Timeline {
    limit: usize,
    entries: Vec<TimelineEntry>,
}

impl Timeline {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            entries: Vec::new(),
        }
    }

    pub fn push(&mut self, entry: TimelineEntry) {
        if self.limit == 0 {
            return;
        }
        if self.entries.len() == self.limit {
            self.entries.remove(0);
        }
        self.entries.push(entry);
    }

    pub fn entries(&self) -> &[TimelineEntry] {
        &self.entries
    }
}

fn binary_display(bytes: &[u8]) -> String {
    let preview = bytes
        .iter()
        .take(BINARY_PREVIEW_BYTES)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    let suffix = if bytes.len() > BINARY_PREVIEW_BYTES {
        " …"
    } else {
        ""
    };
    format!("{} bytes · {preview}{suffix}", bytes.len())
}
