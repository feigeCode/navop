//! Socket.IO v5 over Engine.IO v4 WebSocket protocol support.

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::http::{PreparedRequest, RequestMethod, normalize_url_with_default};

const DEFAULT_SOCKET_IO_SCHEME: &str = "ws";
const DEFAULT_SOCKET_IO_PATH: &str = "/socket.io/";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct EngineOpenPacket {
    pub sid: String,
    #[serde(default)]
    pub upgrades: Vec<String>,
    #[serde(rename = "pingInterval")]
    pub ping_interval: u64,
    #[serde(rename = "pingTimeout")]
    pub ping_timeout: u64,
    #[serde(default, rename = "maxPayload")]
    pub max_payload: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EnginePacket {
    Open(EngineOpenPacket),
    Close,
    Ping(String),
    Pong(String),
    Message(SocketPacket),
    Upgrade,
    Noop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SocketPacket {
    Connect {
        namespace: String,
        data: Option<Value>,
    },
    Disconnect {
        namespace: String,
    },
    Event {
        namespace: String,
        id: Option<u64>,
        data: Value,
    },
    Ack {
        namespace: String,
        id: Option<u64>,
        data: Option<Value>,
    },
    ConnectError {
        namespace: String,
        data: Option<Value>,
    },
}

pub fn prepare_socket_io_request(mut request: PreparedRequest) -> Result<PreparedRequest> {
    request.url = socket_io_websocket_url(&request.url)?;
    request.method = RequestMethod::Get;
    request.body.clear();
    Ok(request)
}

pub fn socket_io_websocket_url(input: &str) -> Result<String> {
    let normalized = normalize_url_with_default(input, DEFAULT_SOCKET_IO_SCHEME);
    // `url::Url` treats ws/wss as special schemes and refuses to switch an
    // already-parsed non-special URL (such as socketio://) to them. Rewrite
    // Socket.IO's convenience schemes before parsing so authority/path parsing
    // follows the WebSocket URL rules from the start.
    let normalized = normalized
        .strip_prefix("socketio://")
        .map(|rest| format!("ws://{rest}"))
        .or_else(|| {
            normalized
                .strip_prefix("socketios://")
                .map(|rest| format!("wss://{rest}"))
        })
        .unwrap_or(normalized);
    let mut url = Url::parse(&normalized).context("invalid Socket.IO URL")?;
    let target_scheme = match url.scheme() {
        "http" | "ws" => "ws",
        "https" | "wss" => "wss",
        scheme => bail!("unsupported Socket.IO URL scheme: {scheme}"),
    };
    url.set_scheme(target_scheme)
        .map_err(|_| anyhow!("failed to set Socket.IO WebSocket scheme"))?;

    let current_path = url.path();
    let path = if current_path.is_empty() || current_path == "/" {
        DEFAULT_SOCKET_IO_PATH.to_string()
    } else if current_path.ends_with("/socket.io/") {
        current_path.to_string()
    } else if current_path.ends_with("/socket.io") {
        format!("{current_path}/")
    } else if current_path.ends_with('/') {
        format!("{current_path}socket.io/")
    } else {
        format!("{current_path}/socket.io/")
    };
    url.set_path(&path);
    url.set_fragment(None);

    let query = url
        .query_pairs()
        .filter(|(key, _)| {
            !key.eq_ignore_ascii_case("EIO") && !key.eq_ignore_ascii_case("transport")
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(&key, &value);
        }
        pairs
            .append_pair("EIO", "4")
            .append_pair("transport", "websocket");
    }
    Ok(url.into())
}

pub fn parse_engine_packet(input: &str) -> Result<EnginePacket> {
    let Some(packet_type) = input.as_bytes().first().copied() else {
        bail!("empty Engine.IO packet");
    };
    if !matches!(packet_type, b'0'..=b'6') {
        bail!("unknown Engine.IO packet type");
    }
    let payload = input
        .get(1..)
        .ok_or_else(|| anyhow!("unknown Engine.IO packet type"))?;
    match packet_type {
        b'0' => Ok(EnginePacket::Open(
            serde_json::from_str(payload).context("invalid Engine.IO open packet")?,
        )),
        b'1' if payload.is_empty() => Ok(EnginePacket::Close),
        b'2' => Ok(EnginePacket::Ping(payload.to_string())),
        b'3' => Ok(EnginePacket::Pong(payload.to_string())),
        b'4' => Ok(EnginePacket::Message(parse_socket_packet(payload)?)),
        b'5' if payload.is_empty() => Ok(EnginePacket::Upgrade),
        b'6' if payload.is_empty() => Ok(EnginePacket::Noop),
        b'0'..=b'6' => bail!("invalid Engine.IO packet payload"),
        _ => unreachable!("packet type was validated above"),
    }
}

pub fn parse_socket_packet(input: &str) -> Result<SocketPacket> {
    let bytes = input.as_bytes();
    let Some(packet_type) = bytes.first().copied() else {
        bail!("empty Socket.IO packet");
    };
    if matches!(packet_type, b'5' | b'6') {
        bail!("binary Socket.IO packets are not supported yet");
    }
    if !matches!(packet_type, b'0'..=b'4') {
        bail!("unknown Socket.IO packet type");
    }

    let mut cursor = 1;
    let namespace = if bytes.get(cursor) == Some(&b'/') {
        let comma = input[cursor..]
            .find(',')
            .map(|offset| cursor + offset)
            .unwrap_or(input.len());
        let namespace = input[cursor..comma].to_string();
        cursor = if comma < input.len() {
            comma + 1
        } else {
            comma
        };
        namespace
    } else {
        "/".to_string()
    };

    let id_start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    let id = if cursor > id_start {
        Some(
            input[id_start..cursor]
                .parse::<u64>()
                .context("invalid Socket.IO acknowledgment id")?,
        )
    } else {
        None
    };
    let data = if cursor < input.len() {
        Some(
            serde_json::from_str::<Value>(&input[cursor..])
                .context("invalid Socket.IO JSON payload")?,
        )
    } else {
        None
    };

    match packet_type {
        b'0' => Ok(SocketPacket::Connect { namespace, data }),
        b'1' => {
            if id.is_some() || data.is_some() {
                bail!("Socket.IO disconnect packet cannot contain a payload");
            }
            Ok(SocketPacket::Disconnect { namespace })
        }
        b'2' => {
            let data = data.ok_or_else(|| anyhow!("Socket.IO event payload is missing"))?;
            if !data.as_array().is_some_and(|items| !items.is_empty()) {
                bail!("Socket.IO event payload must be a non-empty JSON array");
            }
            Ok(SocketPacket::Event {
                namespace,
                id,
                data,
            })
        }
        b'3' => {
            if data.as_ref().is_some_and(|value| !value.is_array()) {
                bail!("Socket.IO acknowledgment payload must be a JSON array");
            }
            Ok(SocketPacket::Ack {
                namespace,
                id,
                data,
            })
        }
        b'4' => Ok(SocketPacket::ConnectError { namespace, data }),
        _ => unreachable!("packet type was validated above"),
    }
}

pub fn encode_event_input(input: &str) -> Result<String> {
    let input = input.trim_end_matches(['\r', '\n']);
    if input.trim().is_empty() {
        bail!("Socket.IO event is empty");
    }
    if parse_engine_packet(input).is_ok() {
        return Ok(input.to_string());
    }
    if let Ok(value) = serde_json::from_str::<Value>(input) {
        if value.as_array().is_some_and(|items| !items.is_empty()) {
            return Ok(format!("42{}", serde_json::to_string(&value)?));
        }
    }
    Ok(format!(
        "42{}",
        serde_json::to_string(&serde_json::json!(["message", input]))?
    ))
}

pub fn display_socket_packet(packet: &SocketPacket) -> String {
    match packet {
        SocketPacket::Connect { namespace, data } => {
            format_packet("CONNECT", namespace, None, data.as_ref())
        }
        SocketPacket::Disconnect { namespace } => {
            format_packet("DISCONNECT", namespace, None, None)
        }
        SocketPacket::Event {
            namespace,
            id,
            data,
        } => format_packet("EVENT", namespace, *id, Some(data)),
        SocketPacket::Ack {
            namespace,
            id,
            data,
        } => format_packet("ACK", namespace, *id, data.as_ref()),
        SocketPacket::ConnectError { namespace, data } => {
            format_packet("CONNECT_ERROR", namespace, None, data.as_ref())
        }
    }
}

fn format_packet(kind: &str, namespace: &str, id: Option<u64>, data: Option<&Value>) -> String {
    let namespace = if namespace == "/" {
        String::new()
    } else {
        format!(" · {namespace}")
    };
    let id = id.map_or_else(String::new, |id| format!(" · #{id}"));
    let payload = data.map_or_else(String::new, |value| {
        let rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
        format!("\n{rendered}")
    });
    format!("{kind}{namespace}{id}{payload}")
}
