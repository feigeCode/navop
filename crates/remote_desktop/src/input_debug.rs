use std::fmt;

use crate::{RemoteDesktopInput, RemoteKey};

impl fmt::Debug for RemoteDesktopInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text { text } | Self::ClipboardText { text } => formatter
                .debug_struct(input_name(self))
                .field("text_len", &text.len())
                .finish(),
            Self::ClipboardFiles { transfer_id, paths } => formatter
                .debug_struct("ClipboardFiles")
                .field("transfer_id", transfer_id)
                .field("path_count", &paths.len())
                .finish(),
            Self::Resize { .. }
            | Self::MouseMove { .. }
            | Self::MouseButton { .. }
            | Self::Wheel { .. } => debug_pointer_input(self, formatter),
            Self::Key { key, pressed } => formatter
                .debug_struct("Key")
                .field("key_kind", &key_kind(key))
                .field("pressed", pressed)
                .finish(),
            Self::CancelClipboardTransfer { transfer_id } => formatter
                .debug_struct("CancelClipboardTransfer")
                .field("transfer_id", transfer_id)
                .finish(),
            Self::Reconnect => formatter.write_str("Reconnect"),
            Self::Close => formatter.write_str("Close"),
        }
    }
}

fn debug_pointer_input(
    input: &RemoteDesktopInput,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    match input {
        RemoteDesktopInput::Resize {
            width,
            height,
            scale_factor,
        } => formatter
            .debug_struct("Resize")
            .field("width", width)
            .field("height", height)
            .field("scale_factor", scale_factor)
            .finish(),
        RemoteDesktopInput::MouseMove { .. } => formatter.write_str("MouseMove"),
        RemoteDesktopInput::MouseButton { button, pressed } => formatter
            .debug_struct("MouseButton")
            .field("button", button)
            .field("pressed", pressed)
            .finish(),
        RemoteDesktopInput::Wheel { vertical, units } => formatter
            .debug_struct("Wheel")
            .field("vertical", vertical)
            .field("units", units)
            .finish(),
        _ => unreachable!("pointer debug called for another input"),
    }
}

fn input_name(input: &RemoteDesktopInput) -> &'static str {
    match input {
        RemoteDesktopInput::Text { .. } => "Text",
        RemoteDesktopInput::ClipboardText { .. } => "ClipboardText",
        _ => unreachable!("text name called for another input"),
    }
}

fn key_kind(key: &RemoteKey) -> &'static str {
    match key {
        RemoteKey::Named(_) => "named",
        RemoteKey::Character(_) => "character",
        RemoteKey::Scancode(_) => "scancode",
        RemoteKey::KeySym(_) => "keysym",
    }
}
