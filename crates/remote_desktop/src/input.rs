#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteDesktopInput {
    Resize {
        width: u16,
        height: u16,
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
