use std::fmt;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::backends::rdp_keyboard::remote_key_to_scancode;
use crate::{
    RemoteDesktopConnectionOptions, RemoteDesktopInput, RemoteDesktopProtocol, RemoteDesktopSize,
    RemoteKey, RemoteMouseButton, RemoteNamedKey,
};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    KeySym {
        keysym: u32,
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

impl HelperRequest {
    pub fn connect_from_options(
        options: &RemoteDesktopConnectionOptions,
        size: RemoteDesktopSize,
    ) -> Self {
        Self::Connect {
            destination: options.destination.clone(),
            username: options.username.clone(),
            password: options.password.clone(),
            domain: options.domain.clone(),
            width: size.width,
            height: size.height,
        }
    }

    pub fn from_remote_input(input: &RemoteDesktopInput) -> Option<Self> {
        Self::from_remote_input_for_protocol(input, RemoteDesktopProtocol::Rdp)
    }

    pub fn from_remote_input_for_protocol(
        input: &RemoteDesktopInput,
        protocol: RemoteDesktopProtocol,
    ) -> Option<Self> {
        Some(match input {
            RemoteDesktopInput::Resize { width, height } => Self::Resize {
                width: *width,
                height: *height,
            },
            RemoteDesktopInput::MouseMove { x, y } => Self::MouseMove { x: *x, y: *y },
            RemoteDesktopInput::MouseButton { button, pressed } => Self::MouseButton {
                button: HelperMouseButton::from_remote(*button),
                pressed: *pressed,
            },
            RemoteDesktopInput::Wheel { vertical, units } => Self::Wheel {
                vertical: *vertical,
                units: *units,
            },
            RemoteDesktopInput::Key { key, pressed } => key_request(key, *pressed, protocol)?,
            RemoteDesktopInput::Text { text } => Self::Text { text: text.clone() },
            RemoteDesktopInput::ClipboardText { text } => {
                Self::ClipboardText { text: text.clone() }
            }
            RemoteDesktopInput::Reconnect => return None,
            RemoteDesktopInput::Close => Self::Close,
        })
    }
}

impl fmt::Debug for HelperRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect {
                destination,
                username,
                password,
                domain,
                width,
                height,
            } => f
                .debug_struct("Connect")
                .field("destination", destination)
                .field("username", username)
                .field("password", &password.as_ref().map(|_| "<redacted>"))
                .field("domain", domain)
                .field("width", width)
                .field("height", height)
                .finish(),
            Self::Resize { width, height } => f
                .debug_struct("Resize")
                .field("width", width)
                .field("height", height)
                .finish(),
            Self::MouseMove { x, y } => f
                .debug_struct("MouseMove")
                .field("x", x)
                .field("y", y)
                .finish(),
            Self::MouseButton { button, pressed } => f
                .debug_struct("MouseButton")
                .field("button", button)
                .field("pressed", pressed)
                .finish(),
            Self::Wheel { vertical, units } => f
                .debug_struct("Wheel")
                .field("vertical", vertical)
                .field("units", units)
                .finish(),
            Self::Key {
                code,
                extended,
                pressed,
            } => f
                .debug_struct("Key")
                .field("code", code)
                .field("extended", extended)
                .field("pressed", pressed)
                .finish(),
            Self::KeySym { keysym, pressed } => f
                .debug_struct("KeySym")
                .field("keysym", keysym)
                .field("pressed", pressed)
                .finish(),
            Self::Text { text } => f.debug_struct("Text").field("text", text).finish(),
            Self::ClipboardText { text } => {
                f.debug_struct("ClipboardText").field("text", text).finish()
            }
            Self::Close => f.write_str("Close"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HelperMouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

impl HelperMouseButton {
    fn from_remote(button: RemoteMouseButton) -> Self {
        match button {
            RemoteMouseButton::Left => Self::Left,
            RemoteMouseButton::Middle => Self::Middle,
            RemoteMouseButton::Right => Self::Right,
            RemoteMouseButton::X1 => Self::X1,
            RemoteMouseButton::X2 => Self::X2,
        }
    }
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

    pub fn into_rgba(self) -> anyhow::Result<Vec<u8>> {
        match self {
            Self::Frame { rgba_base64, .. } => {
                Ok(base64::engine::general_purpose::STANDARD.decode(rgba_base64.as_bytes())?)
            }
            _ => anyhow::bail!("helper event is not a frame"),
        }
    }
}

pub fn encode_request_line(request: &HelperRequest) -> anyhow::Result<String> {
    encode_line(request)
}

pub fn decode_request_line(line: &str) -> anyhow::Result<HelperRequest> {
    Ok(serde_json::from_str(line.trim_end())?)
}

pub fn encode_event_line(event: &HelperEvent) -> anyhow::Result<String> {
    encode_line(event)
}

pub fn decode_event_line(line: &str) -> anyhow::Result<HelperEvent> {
    Ok(serde_json::from_str(line.trim_end())?)
}

fn encode_line<T>(value: &T) -> anyhow::Result<String>
where
    T: Serialize,
{
    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    Ok(line)
}

fn key_request(
    key: &RemoteKey,
    pressed: bool,
    protocol: RemoteDesktopProtocol,
) -> Option<HelperRequest> {
    match protocol {
        RemoteDesktopProtocol::Rdp => rdp_key_request(key, pressed),
        RemoteDesktopProtocol::Vnc => vnc_key_request(key, pressed),
    }
}

fn rdp_key_request(key: &RemoteKey, pressed: bool) -> Option<HelperRequest> {
    let scancode = match key {
        RemoteKey::Character(character) => character_to_scancode(*character)?,
        _ => remote_key_to_scancode(key)?,
    };

    Some(HelperRequest::Key {
        code: scancode.code,
        extended: scancode.extended,
        pressed,
    })
}

fn vnc_key_request(key: &RemoteKey, pressed: bool) -> Option<HelperRequest> {
    match key {
        RemoteKey::Character(character) => Some(HelperRequest::KeySym {
            keysym: character_to_keysym(*character),
            pressed,
        }),
        RemoteKey::KeySym(keysym) => Some(HelperRequest::KeySym {
            keysym: *keysym,
            pressed,
        }),
        RemoteKey::Named(named) => Some(HelperRequest::KeySym {
            keysym: named_key_to_keysym(*named)?,
            pressed,
        }),
        RemoteKey::Scancode(_) => rdp_key_request(key, pressed),
    }
}

fn character_to_keysym(character: char) -> u32 {
    let codepoint = character as u32;
    if codepoint <= 0xff {
        codepoint
    } else {
        0x0100_0000 | codepoint
    }
}

fn named_key_to_keysym(key: RemoteNamedKey) -> Option<u32> {
    Some(match key {
        RemoteNamedKey::Escape => 0xff1b,
        RemoteNamedKey::Backspace => 0xff08,
        RemoteNamedKey::Tab => 0xff09,
        RemoteNamedKey::Enter => 0xff0d,
        RemoteNamedKey::Space => 0x20,
        RemoteNamedKey::Insert => 0xff63,
        RemoteNamedKey::Delete => 0xffff,
        RemoteNamedKey::Home => 0xff50,
        RemoteNamedKey::End => 0xff57,
        RemoteNamedKey::PageUp => 0xff55,
        RemoteNamedKey::PageDown => 0xff56,
        RemoteNamedKey::ArrowUp => 0xff52,
        RemoteNamedKey::ArrowDown => 0xff54,
        RemoteNamedKey::ArrowLeft => 0xff51,
        RemoteNamedKey::ArrowRight => 0xff53,
        RemoteNamedKey::Shift => 0xffe1,
        RemoteNamedKey::Control => 0xffe3,
        RemoteNamedKey::Alt => 0xffe9,
        RemoteNamedKey::Meta => 0xffeb,
        RemoteNamedKey::CapsLock => 0xffe5,
        RemoteNamedKey::F(index @ 1..=35) => 0xffbe + u32::from(index - 1),
        RemoteNamedKey::F(_) => return None,
    })
}

fn character_to_scancode(character: char) -> Option<crate::backends::rdp_keyboard::RdpScanCode> {
    let code = match character.to_ascii_lowercase() {
        'a' => 0x1e,
        'b' => 0x30,
        'c' => 0x2e,
        'd' => 0x20,
        'e' => 0x12,
        'f' => 0x21,
        'g' => 0x22,
        'h' => 0x23,
        'i' => 0x17,
        'j' => 0x24,
        'k' => 0x25,
        'l' => 0x26,
        'm' => 0x32,
        'n' => 0x31,
        'o' => 0x18,
        'p' => 0x19,
        'q' => 0x10,
        'r' => 0x13,
        's' => 0x1f,
        't' => 0x14,
        'u' => 0x16,
        'v' => 0x2f,
        'w' => 0x11,
        'x' => 0x2d,
        'y' => 0x15,
        'z' => 0x2c,
        '0' => 0x0b,
        '1' => 0x02,
        '2' => 0x03,
        '3' => 0x04,
        '4' => 0x05,
        '5' => 0x06,
        '6' => 0x07,
        '7' => 0x08,
        '8' => 0x09,
        '9' => 0x0a,
        ' ' => return remote_key_to_scancode(&RemoteKey::Named(RemoteNamedKey::Space)),
        _ => return None,
    };

    Some(crate::backends::rdp_keyboard::RdpScanCode {
        code,
        extended: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RemoteDesktopConnectionOptions, RemoteDesktopProtocol, RemoteDesktopSize};

    fn rdp_options() -> RemoteDesktopConnectionOptions {
        RemoteDesktopConnectionOptions {
            protocol: RemoteDesktopProtocol::Rdp,
            destination: "10.2.178.12:3389".to_string(),
            username: Some("administrator".to_string()),
            password: Some("Seeyon123@cd".to_string()),
            domain: None,
            read_only: false,
        }
    }

    #[test]
    fn connect_request_from_options_redacts_debug_password() {
        let request = HelperRequest::connect_from_options(
            &rdp_options(),
            RemoteDesktopSize {
                width: 1280,
                height: 720,
            },
        );
        let debug = format!("{request:?}");

        assert!(debug.contains("10.2.178.12:3389"));
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("Seeyon123@cd"));
    }

    #[test]
    fn helper_frame_roundtrips_rgba_payload() {
        let event = HelperEvent::frame(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]);

        let line = encode_event_line(&event).expect("event encodes");
        let decoded = decode_event_line(&line).expect("event decodes");

        assert_eq!(
            decoded,
            HelperEvent::Frame {
                width: 2,
                height: 1,
                rgba_base64: "AQID/wQFBv8=".to_string()
            }
        );
        assert_eq!(
            decoded.into_rgba().expect("rgba decodes"),
            vec![1, 2, 3, 255, 4, 5, 6, 255]
        );
    }

    #[test]
    fn input_key_converts_to_helper_scancode() {
        let request = HelperRequest::from_remote_input(&crate::RemoteDesktopInput::Key {
            key: crate::RemoteKey::Named(crate::RemoteNamedKey::Enter),
            pressed: true,
        });

        assert_eq!(
            request,
            Some(HelperRequest::Key {
                code: 0x1c,
                extended: false,
                pressed: true,
            })
        );
    }

    #[test]
    fn input_prefixed_scancode_converts_to_extended_helper_scancode() {
        let request = HelperRequest::from_remote_input(&crate::RemoteDesktopInput::Key {
            key: crate::RemoteKey::Scancode(0xe048),
            pressed: true,
        });

        assert_eq!(
            request,
            Some(HelperRequest::Key {
                code: 0x48,
                extended: true,
                pressed: true,
            })
        );
    }

    #[test]
    fn vnc_character_key_converts_to_helper_keysym() {
        let request = HelperRequest::from_remote_input_for_protocol(
            &crate::RemoteDesktopInput::Key {
                key: crate::RemoteKey::Character(':'),
                pressed: true,
            },
            crate::RemoteDesktopProtocol::Vnc,
        );

        assert_eq!(
            request,
            Some(HelperRequest::KeySym {
                keysym: b':' as u32,
                pressed: true,
            })
        );
    }

    #[test]
    fn vnc_named_key_converts_to_helper_keysym() {
        let request = HelperRequest::from_remote_input_for_protocol(
            &crate::RemoteDesktopInput::Key {
                key: crate::RemoteKey::Named(crate::RemoteNamedKey::Tab),
                pressed: true,
            },
            crate::RemoteDesktopProtocol::Vnc,
        );

        assert_eq!(
            request,
            Some(HelperRequest::KeySym {
                keysym: 0xff09,
                pressed: true,
            })
        );
    }

    #[test]
    fn input_clipboard_text_converts_to_helper_request() {
        let request = HelperRequest::from_remote_input(&crate::RemoteDesktopInput::ClipboardText {
            text: "hello 中文".to_string(),
        });

        assert_eq!(
            request,
            Some(HelperRequest::ClipboardText {
                text: "hello 中文".to_string()
            })
        );
    }

    #[test]
    fn input_reconnect_is_backend_control_not_helper_request() {
        let request = HelperRequest::from_remote_input(&crate::RemoteDesktopInput::Reconnect);

        assert_eq!(None, request);
    }

    #[test]
    fn helper_clipboard_text_event_decodes() {
        let line = r#"{"type":"ClipboardText","text":"remote 中文"}"#;

        let event = decode_event_line(line).expect("event decodes");

        assert_eq!(
            event,
            HelperEvent::ClipboardText {
                text: "remote 中文".to_string()
            }
        );
    }
}
