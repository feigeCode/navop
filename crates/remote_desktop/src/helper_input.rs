use crate::backends::rdp_keyboard::{RdpScanCode, remote_key_to_scancode};
use crate::helper_protocol::{HelperMouseButton, HelperRequest};
use crate::{
    RemoteDesktopInput, RemoteDesktopProtocol, RemoteKey, RemoteMouseButton, RemoteNamedKey,
};

impl HelperRequest {
    pub fn from_remote_input(input: &RemoteDesktopInput) -> Option<Self> {
        Self::from_remote_input_for_protocol(input, RemoteDesktopProtocol::Rdp)
    }

    pub fn from_remote_input_for_protocol(
        input: &RemoteDesktopInput,
        protocol: RemoteDesktopProtocol,
    ) -> Option<Self> {
        Some(match input {
            RemoteDesktopInput::Resize {
                width,
                height,
                scale_factor,
            } => Self::Resize {
                width: *width,
                height: *height,
                scale_factor: *scale_factor,
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
            RemoteDesktopInput::ClipboardFiles { transfer_id, paths } => {
                file_clipboard_request(protocol, *transfer_id, paths)?
            }
            RemoteDesktopInput::CancelClipboardTransfer { transfer_id } => {
                cancel_clipboard_request(protocol, *transfer_id)?
            }
            RemoteDesktopInput::Reconnect => return None,
            RemoteDesktopInput::Close => Self::Close,
        })
    }
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

fn file_clipboard_request(
    protocol: RemoteDesktopProtocol,
    transfer_id: u64,
    paths: &[String],
) -> Option<HelperRequest> {
    (protocol == RemoteDesktopProtocol::Rdp).then(|| HelperRequest::ClipboardFiles {
        transfer_id,
        paths: paths.to_vec(),
    })
}

fn cancel_clipboard_request(
    protocol: RemoteDesktopProtocol,
    transfer_id: u64,
) -> Option<HelperRequest> {
    (protocol == RemoteDesktopProtocol::Rdp)
        .then_some(HelperRequest::CancelClipboardTransfer { transfer_id })
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

fn character_to_scancode(character: char) -> Option<RdpScanCode> {
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

    Some(RdpScanCode {
        code,
        extended: false,
    })
}

#[cfg(test)]
#[path = "helper_input_tests.rs"]
mod tests;
