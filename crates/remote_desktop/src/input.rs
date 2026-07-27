#[derive(Clone, PartialEq, Eq)]
pub enum RemoteDesktopInput {
    Resize {
        width: u16,
        height: u16,
        scale_factor: u32,
    },
    MouseMove {
        x: u16,
        y: u16,
    },
    MouseButton {
        button: RemoteMouseButton,
        pressed: bool,
    },
    Wheel {
        vertical: bool,
        units: i16,
    },
    Key {
        key: RemoteKey,
        pressed: bool,
    },
    Text {
        text: String,
    },
    ClipboardText {
        text: String,
    },
    ClipboardFiles {
        transfer_id: u64,
        paths: Vec<String>,
    },
    CancelClipboardTransfer {
        transfer_id: u64,
    },
    Reconnect,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteMouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteKey {
    Named(RemoteNamedKey),
    Character(char),
    Scancode(u16),
    KeySym(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteNamedKey {
    Escape,
    Backspace,
    Tab,
    Enter,
    Space,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Shift,
    Control,
    Alt,
    Meta,
    CapsLock,
    F(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_debug_reports_metadata_without_text_paths_or_character_keys() {
        let inputs = [
            RemoteDesktopInput::Text {
                text: "typed-secret".to_string(),
            },
            RemoteDesktopInput::ClipboardText {
                text: "clipboard-secret".to_string(),
            },
            RemoteDesktopInput::ClipboardFiles {
                transfer_id: 17,
                paths: vec!["/Users/rachel/private-file".to_string()],
            },
            RemoteDesktopInput::Key {
                key: RemoteKey::Character('q'),
                pressed: true,
            },
        ];

        for input in inputs {
            let debug = format!("{input:?}");

            assert!(!debug.contains("secret"));
            assert!(!debug.contains("private-file"));
            assert!(!debug.contains("Character('q')"));
        }
    }
}
