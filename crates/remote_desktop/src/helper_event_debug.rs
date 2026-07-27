use std::fmt;

use crate::helper_protocol::HelperEvent;

struct FrameDebugFields {
    width: u16,
    height: u16,
    rect_count: Option<usize>,
    byte_len: usize,
}

impl fmt::Debug for HelperEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame { .. }
            | Self::FrameBytes { .. }
            | Self::FrameBgraBytes { .. }
            | Self::FrameBgraRects { .. } => debug_frame_event(self, formatter),
            Self::Status { .. }
            | Self::ClipboardTransferFailed { .. }
            | Self::ConnectionFailure { .. }
            | Self::Terminated { .. } => debug_message_event(self, formatter),
            _ => debug_data_event(self, formatter),
        }
    }
}

fn debug_frame_event(event: &HelperEvent, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match event {
        HelperEvent::Frame {
            width,
            height,
            rgba_base64,
        } => formatter
            .debug_struct("Frame")
            .field("width", width)
            .field("height", height)
            .field("base64_len", &rgba_base64.len())
            .finish(),
        HelperEvent::FrameBytes {
            width,
            height,
            rgba_len,
        } => debug_frame_lengths(
            "FrameBytes",
            FrameDebugFields {
                width: *width,
                height: *height,
                rect_count: None,
                byte_len: *rgba_len,
            },
            formatter,
        ),
        HelperEvent::FrameBgraBytes {
            width,
            height,
            bgra_len,
        } => debug_frame_lengths(
            "FrameBgraBytes",
            FrameDebugFields {
                width: *width,
                height: *height,
                rect_count: None,
                byte_len: *bgra_len,
            },
            formatter,
        ),
        HelperEvent::FrameBgraRects {
            width,
            height,
            rects,
            bgra_len,
        } => debug_frame_lengths(
            "FrameBgraRects",
            FrameDebugFields {
                width: *width,
                height: *height,
                rect_count: Some(rects.len()),
                byte_len: *bgra_len,
            },
            formatter,
        ),
        _ => unreachable!("frame debug called for another event"),
    }
}

fn debug_frame_lengths(
    name: &str,
    fields: FrameDebugFields,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    let mut debug = formatter.debug_struct(name);
    debug
        .field("width", &fields.width)
        .field("height", &fields.height);
    if let Some(rect_count) = fields.rect_count {
        debug.field("rect_count", &rect_count);
    }
    debug.field("byte_len", &fields.byte_len).finish()
}

fn debug_message_event(event: &HelperEvent, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match event {
        HelperEvent::Status { message } => debug_message("Status", None, message.len(), formatter),
        HelperEvent::ClipboardTransferFailed {
            transfer_id,
            message,
        } => debug_message(
            "ClipboardTransferFailed",
            Some(*transfer_id),
            message.len(),
            formatter,
        ),
        HelperEvent::ConnectionFailure { message } => {
            debug_message("ConnectionFailure", None, message.len(), formatter)
        }
        HelperEvent::Terminated { message } => {
            debug_message("Terminated", None, message.len(), formatter)
        }
        _ => unreachable!("message debug called for another event"),
    }
}

fn debug_message(
    name: &str,
    transfer_id: Option<u64>,
    message_len: usize,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    let mut debug = formatter.debug_struct(name);
    if let Some(transfer_id) = transfer_id {
        debug.field("transfer_id", &transfer_id);
    }
    debug.field("message_len", &message_len).finish()
}

fn debug_data_event(event: &HelperEvent, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match event {
        HelperEvent::Connected { width, height } => formatter
            .debug_struct("Connected")
            .field("width", width)
            .field("height", height)
            .finish(),
        HelperEvent::CursorDefault => formatter.write_str("CursorDefault"),
        HelperEvent::CursorHidden => formatter.write_str("CursorHidden"),
        HelperEvent::CursorPosition { x, y } => formatter
            .debug_struct("CursorPosition")
            .field("x", x)
            .field("y", y)
            .finish(),
        HelperEvent::CursorRgbaBytes {
            width,
            height,
            hotspot_x,
            hotspot_y,
            rgba_len,
        } => formatter
            .debug_struct("CursorRgbaBytes")
            .field("width", width)
            .field("height", height)
            .field("hotspot_x", hotspot_x)
            .field("hotspot_y", hotspot_y)
            .field("byte_len", rgba_len)
            .finish(),
        HelperEvent::ClipboardText { text } => formatter
            .debug_struct("ClipboardText")
            .field("text_len", &text.len())
            .finish(),
        HelperEvent::ClipboardFilesReady { transfer_id, paths } => formatter
            .debug_struct("ClipboardFilesReady")
            .field("transfer_id", transfer_id)
            .field("path_count", &paths.len())
            .finish(),
        HelperEvent::Reconnecting { reason, delay_secs } => formatter
            .debug_struct("Reconnecting")
            .field("reason", reason)
            .field("delay_secs", delay_secs)
            .finish(),
        _ => unreachable!("data debug called for another event"),
    }
}
