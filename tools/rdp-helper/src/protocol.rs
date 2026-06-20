use base64::Engine as _;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HelperRequest {
    Connect {
        destination: String,
        username: Option<String>,
        password: Option<String>,
        domain: Option<String>,
        width: u16,
        height: u16,
    },
    Resize {
        width: u16,
        height: u16,
    },
    MouseMove {
        x: u16,
        y: u16,
    },
    MouseButton {
        button: HelperMouseButton,
        pressed: bool,
    },
    Wheel {
        vertical: bool,
        units: i16,
    },
    Key {
        code: u16,
        extended: bool,
        pressed: bool,
    },
    Text {
        text: String,
    },
    ClipboardText {
        text: String,
    },
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HelperMouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectRequest {
    pub destination: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub domain: Option<String>,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HelperEvent {
    Status {
        message: String,
    },
    Connected {
        width: u16,
        height: u16,
    },
    Frame {
        width: u16,
        height: u16,
        rgba_base64: String,
    },
    CursorDefault,
    CursorHidden,
    CursorPosition {
        x: u16,
        y: u16,
    },
    ClipboardText {
        text: String,
    },
    ConnectionFailure {
        message: String,
    },
    Terminated {
        message: String,
    },
}

impl HelperEvent {
    pub fn frame(width: u16, height: u16, rgba: Vec<u8>) -> Self {
        Self::Frame {
            width,
            height,
            rgba_base64: base64::engine::general_purpose::STANDARD.encode(rgba),
        }
    }
}

pub fn connect_request(request: HelperRequest) -> anyhow::Result<ConnectRequest> {
    match request {
        HelperRequest::Connect {
            destination,
            username,
            password,
            domain,
            width,
            height,
        } => Ok(ConnectRequest {
            destination,
            username,
            password,
            domain,
            width,
            height,
        }),
        _ => anyhow::bail!("first helper request must be Connect"),
    }
}

pub fn decode_request_line(line: &str) -> anyhow::Result<HelperRequest> {
    Ok(serde_json::from_str(line.trim_end())?)
}

pub fn encode_event_line(event: &HelperEvent) -> anyhow::Result<String> {
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_connect_request_shape_from_main_process() {
        let line = r#"{"type":"Connect","destination":"10.2.178.12:3389","username":"administrator","password":"secret","domain":null,"width":1280,"height":720}"#;

        let request = connect_request(decode_request_line(line).expect("request decodes"))
            .expect("connect request");

        assert_eq!(request.destination, "10.2.178.12:3389");
        assert_eq!(request.username.as_deref(), Some("administrator"));
        assert_eq!(request.password.as_deref(), Some("secret"));
        assert_eq!(request.width, 1280);
        assert_eq!(request.height, 720);
    }

    #[test]
    fn encodes_frame_event_shape_for_main_process() {
        let event = HelperEvent::frame(1, 1, vec![0x11, 0x22, 0x33, 0xff]);

        let line = encode_event_line(&event).expect("event encodes");

        assert_eq!(
            line,
            "{\"type\":\"Frame\",\"width\":1,\"height\":1,\"rgba_base64\":\"ESIz/w==\"}\n"
        );
    }

    #[test]
    fn decodes_clipboard_text_request_shape_from_main_process() {
        let line = r#"{"type":"ClipboardText","text":"local 中文"}"#;

        let request = decode_request_line(line).expect("request decodes");

        assert_eq!(
            request,
            HelperRequest::ClipboardText {
                text: "local 中文".to_string()
            }
        );
    }

    #[test]
    fn encodes_clipboard_text_event_shape_for_main_process() {
        let event = HelperEvent::ClipboardText {
            text: "remote 中文".to_string(),
        };

        let line = encode_event_line(&event).expect("event encodes");

        assert_eq!(
            line,
            "{\"type\":\"ClipboardText\",\"text\":\"remote 中文\"}\n"
        );
    }
}
