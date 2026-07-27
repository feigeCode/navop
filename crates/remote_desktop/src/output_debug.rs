use std::fmt;

use crate::RemoteDesktopOutput;

struct FrameDebugFields {
    width: u16,
    height: u16,
    rect_count: Option<usize>,
    byte_len: usize,
}

struct MessageDebugFields {
    transfer_id: Option<u64>,
    message_len: usize,
}

impl fmt::Debug for RemoteDesktopOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connected {
                width,
                height,
                capabilities,
            } => formatter
                .debug_struct("Connected")
                .field("width", width)
                .field("height", height)
                .field("capabilities", capabilities)
                .finish(),
            Self::Frame { .. } | Self::FrameBgra { .. } | Self::FrameBgraRects { .. } => {
                debug_frame(self, formatter)
            }
            Self::ClipboardText { text } => formatter
                .debug_struct("ClipboardText")
                .field("text_len", &text.len())
                .finish(),
            Self::ClipboardFilesReady { transfer_id, paths } => formatter
                .debug_struct("ClipboardFilesReady")
                .field("transfer_id", transfer_id)
                .field("path_count", &paths.len())
                .finish(),
            Self::ClipboardTransferFailed {
                transfer_id,
                message,
            } => debug_message(
                "ClipboardTransferFailed",
                MessageDebugFields {
                    transfer_id: Some(*transfer_id),
                    message_len: message.len(),
                },
                formatter,
            ),
            Self::Reconnecting(reconnect) => formatter
                .debug_tuple("Reconnecting")
                .field(reconnect)
                .finish(),
            Self::Status(message)
            | Self::ConnectionFailure(message)
            | Self::Terminated(message) => debug_message(
                output_name(self),
                MessageDebugFields {
                    transfer_id: None,
                    message_len: message.len(),
                },
                formatter,
            ),
            Self::CursorDefault | Self::CursorHidden => formatter.write_str(output_name(self)),
            Self::CursorPosition { x, y } => formatter
                .debug_struct("CursorPosition")
                .field("x", x)
                .field("y", y)
                .finish(),
            Self::CursorBitmap(cursor) => formatter
                .debug_struct("CursorBitmap")
                .field("width", &cursor.width)
                .field("height", &cursor.height)
                .field("hotspot_x", &cursor.hotspot_x)
                .field("hotspot_y", &cursor.hotspot_y)
                .field("byte_len", &cursor.rgba.len())
                .finish(),
        }
    }
}

fn debug_frame(output: &RemoteDesktopOutput, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match output {
        RemoteDesktopOutput::Frame {
            width,
            height,
            rgba,
        } => debug_frame_fields(
            "Frame",
            FrameDebugFields {
                width: *width,
                height: *height,
                rect_count: None,
                byte_len: rgba.len(),
            },
            formatter,
        ),
        RemoteDesktopOutput::FrameBgra {
            width,
            height,
            bgra,
        } => debug_frame_fields(
            "FrameBgra",
            FrameDebugFields {
                width: *width,
                height: *height,
                rect_count: None,
                byte_len: bgra.len(),
            },
            formatter,
        ),
        RemoteDesktopOutput::FrameBgraRects {
            width,
            height,
            rects,
            bgra,
        } => debug_frame_fields(
            "FrameBgraRects",
            FrameDebugFields {
                width: *width,
                height: *height,
                rect_count: Some(rects.len()),
                byte_len: bgra.len(),
            },
            formatter,
        ),
        _ => unreachable!("frame debug called for another output"),
    }
}

fn debug_frame_fields(
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

fn debug_message(
    name: &str,
    fields: MessageDebugFields,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    let mut debug = formatter.debug_struct(name);
    if let Some(transfer_id) = fields.transfer_id {
        debug.field("transfer_id", &transfer_id);
    }
    debug.field("message_len", &fields.message_len).finish()
}

fn output_name(output: &RemoteDesktopOutput) -> &'static str {
    match output {
        RemoteDesktopOutput::Status(_) => "Status",
        RemoteDesktopOutput::ConnectionFailure(_) => "ConnectionFailure",
        RemoteDesktopOutput::Terminated(_) => "Terminated",
        RemoteDesktopOutput::CursorDefault => "CursorDefault",
        RemoteDesktopOutput::CursorHidden => "CursorHidden",
        _ => unreachable!("output name called for unsupported output"),
    }
}
