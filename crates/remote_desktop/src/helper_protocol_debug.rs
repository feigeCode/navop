use std::fmt;

use crate::helper_protocol::HelperRequest;

impl fmt::Debug for HelperRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect { .. } => debug_connect_request(self, formatter),
            Self::Resize { .. }
            | Self::MouseMove { .. }
            | Self::MouseButton { .. }
            | Self::Wheel { .. }
            | Self::Key { .. }
            | Self::KeySym { .. } => debug_input_request(self, formatter),
            Self::Text { .. }
            | Self::ClipboardText { .. }
            | Self::ClipboardFiles { .. }
            | Self::CancelClipboardTransfer { .. } => debug_clipboard_request(self, formatter),
            Self::Close => formatter.write_str("Close"),
        }
    }
}

fn debug_connect_request(
    request: &HelperRequest,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    let HelperRequest::Connect {
        destination,
        username,
        password,
        domain,
        width,
        height,
        scale_factor,
        audio_playback,
        audio_capture,
        shared_folders,
    } = request
    else {
        unreachable!("connect debug called for another request");
    };
    formatter
        .debug_struct("Connect")
        .field("destination", destination)
        .field("username_present", &username.is_some())
        .field("username_len", &option_len(username))
        .field("password_present", &password.is_some())
        .field("domain_present", &domain.is_some())
        .field("domain_len", &option_len(domain))
        .field("width", width)
        .field("height", height)
        .field("scale_factor", scale_factor)
        .field("audio_playback", audio_playback)
        .field("audio_capture", audio_capture)
        .field("shared_folder_count", &shared_folders.len())
        .finish()
}

fn debug_input_request(request: &HelperRequest, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match request {
        HelperRequest::Resize {
            width,
            height,
            scale_factor,
        } => formatter
            .debug_struct("Resize")
            .field("width", width)
            .field("height", height)
            .field("scale_factor", scale_factor)
            .finish(),
        HelperRequest::MouseMove { .. } => formatter.write_str("MouseMove"),
        HelperRequest::MouseButton { button, pressed } => formatter
            .debug_struct("MouseButton")
            .field("button", button)
            .field("pressed", pressed)
            .finish(),
        HelperRequest::Wheel { vertical, units } => formatter
            .debug_struct("Wheel")
            .field("vertical", vertical)
            .field("units", units)
            .finish(),
        HelperRequest::Key {
            extended, pressed, ..
        } => formatter
            .debug_struct("Key")
            .field("extended", extended)
            .field("pressed", pressed)
            .finish(),
        HelperRequest::KeySym { pressed, .. } => formatter
            .debug_struct("KeySym")
            .field("pressed", pressed)
            .finish(),
        _ => unreachable!("input debug called for another request"),
    }
}

fn debug_clipboard_request(
    request: &HelperRequest,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    match request {
        HelperRequest::Text { text } => formatter
            .debug_struct("Text")
            .field("text_len", &text.len())
            .finish(),
        HelperRequest::ClipboardText { text } => formatter
            .debug_struct("ClipboardText")
            .field("text_len", &text.len())
            .finish(),
        HelperRequest::ClipboardFiles { transfer_id, paths } => formatter
            .debug_struct("ClipboardFiles")
            .field("transfer_id", transfer_id)
            .field("path_count", &paths.len())
            .finish(),
        HelperRequest::CancelClipboardTransfer { transfer_id } => formatter
            .debug_struct("CancelClipboardTransfer")
            .field("transfer_id", transfer_id)
            .finish(),
        _ => unreachable!("clipboard debug called for another request"),
    }
}

fn option_len(value: &Option<String>) -> Option<usize> {
    value.as_ref().map(String::len)
}
