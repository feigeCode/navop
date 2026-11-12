use std::io::{BufRead, Read};

use crate::{
    RemoteDesktopCursor, RemoteDesktopFrameRect,
    helper_protocol::{HelperEvent, HelperFrameRect, decode_event_line},
};

use super::helper_events::{helper_disconnect, helper_event_to_output};
use super::transport::HelperOutput;
use super::*;

const MAX_CURSOR_DIMENSION: u16 = 1024;
const MAX_EVENT_HEADER_BYTES: usize = 256 * 1024;
const MAX_FRAME_DIMENSION: u16 = 8192;
const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;
const MAX_FRAME_RECTS: usize = 65_536;

#[derive(Clone, Copy)]
struct CursorHeader {
    width: u16,
    height: u16,
    hotspot_x: u16,
    hotspot_y: u16,
    rgba_len: usize,
}

pub(super) fn read_helper_output(
    reader: &mut impl BufRead,
    protocol: RemoteDesktopProtocol,
) -> anyhow::Result<Option<HelperOutput>> {
    let Some(event) = read_event_header(reader)? else {
        return Ok(None);
    };
    let connected = matches!(event, HelperEvent::Connected { .. });
    let disconnect = helper_disconnect(&event);
    let server_hello = match &event {
        HelperEvent::ServerHello {
            protocol_version,
            provider_version,
            capabilities,
        } => Some(crate::helper_protocol::HelperServerHello {
            protocol_version: *protocol_version,
            provider_version: provider_version.clone(),
            capabilities: *capabilities,
        }),
        _ => None,
    };
    let pong = match &event {
        HelperEvent::Pong { sequence } => Some(*sequence),
        _ => None,
    };
    match event {
        HelperEvent::FrameBytes {
            width,
            height,
            rgba_len,
        } => read_binary_frame_output(reader, width, height, rgba_len).map(Some),
        HelperEvent::FrameBgraBytes {
            width,
            height,
            bgra_len,
        } => read_binary_bgra_frame_output(reader, width, height, bgra_len).map(Some),
        HelperEvent::FrameBgraRects {
            width,
            height,
            rects,
            bgra_len,
        } => read_binary_bgra_rects_output(reader, width, height, rects, bgra_len).map(Some),
        HelperEvent::CursorRgbaBytes {
            width,
            height,
            hotspot_x,
            hotspot_y,
            rgba_len,
        } => read_binary_cursor_output(
            reader,
            CursorHeader {
                width,
                height,
                hotspot_x,
                hotspot_y,
                rgba_len,
            },
        )
        .map(Some),
        event => Ok(Some(HelperOutput {
            output: helper_event_to_output(event, protocol)?,
            connected,
            disconnect,
            server_hello,
            pong,
        })),
    }
}

fn read_binary_cursor_output<R>(
    reader: &mut R,
    header: CursorHeader,
) -> anyhow::Result<HelperOutput>
where
    R: Read + ?Sized,
{
    let expected_len = validate_cursor_header(header)?;
    let mut rgba = vec![0; expected_len];
    reader.read_exact(&mut rgba)?;
    Ok(HelperOutput {
        output: Some(RemoteDesktopOutput::CursorBitmap(RemoteDesktopCursor {
            width: header.width,
            height: header.height,
            hotspot_x: header.hotspot_x,
            hotspot_y: header.hotspot_y,
            rgba,
        })),
        connected: false,
        disconnect: None,
        server_hello: None,
        pong: None,
    })
}

fn validate_cursor_header(header: CursorHeader) -> anyhow::Result<usize> {
    anyhow::ensure!(
        header.width > 0
            && header.height > 0
            && header.width <= MAX_CURSOR_DIMENSION
            && header.height <= MAX_CURSOR_DIMENSION,
        "invalid cursor dimensions"
    );
    anyhow::ensure!(
        header.hotspot_x < header.width && header.hotspot_y < header.height,
        "cursor hotspot is outside bitmap"
    );
    let expected_len = usize::from(header.width)
        .checked_mul(usize::from(header.height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("cursor payload length overflow"))?;
    anyhow::ensure!(
        header.rgba_len == expected_len,
        "invalid cursor payload length: expected {expected_len}, got {}",
        header.rgba_len
    );
    Ok(expected_len)
}

fn read_event_header(reader: &mut impl BufRead) -> anyhow::Result<Option<HelperEvent>> {
    let mut line = Vec::new();
    let read = reader
        .by_ref()
        .take((MAX_EVENT_HEADER_BYTES + 1) as u64)
        .read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(None);
    }
    anyhow::ensure!(
        read <= MAX_EVENT_HEADER_BYTES,
        "helper event header exceeds {MAX_EVENT_HEADER_BYTES} bytes"
    );
    Ok(Some(decode_event_line(std::str::from_utf8(&line)?)?))
}

fn read_binary_bgra_rects_output<R>(
    reader: &mut R,
    width: u16,
    height: u16,
    rects: Vec<HelperFrameRect>,
    bgra_len: usize,
) -> anyhow::Result<HelperOutput>
where
    R: Read + ?Sized,
{
    validate_rects(width, height, &rects, bgra_len)?;
    let mut bgra = vec![0; bgra_len];
    reader.read_exact(&mut bgra)?;
    Ok(HelperOutput {
        output: Some(RemoteDesktopOutput::FrameBgraRects {
            width,
            height,
            rects: rects.into_iter().map(remote_frame_rect).collect(),
            bgra,
        }),
        connected: false,
        disconnect: None,
        server_hello: None,
        pong: None,
    })
}

fn validate_rects(
    width: u16,
    height: u16,
    rects: &[HelperFrameRect],
    bgra_len: usize,
) -> anyhow::Result<()> {
    validate_frame_extent(width, height)?;
    anyhow::ensure!(
        rects.len() <= MAX_FRAME_RECTS,
        "BGRA rectangle count exceeds {MAX_FRAME_RECTS}"
    );
    anyhow::ensure!(
        bgra_len <= MAX_FRAME_BYTES,
        "BGRA payload exceeds {MAX_FRAME_BYTES} bytes"
    );
    let mut expected_len = 0usize;
    for rect in rects {
        let expected_rect_len = usize::from(rect.width)
            .checked_mul(usize::from(rect.height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| anyhow::anyhow!("BGRA rectangle byte length overflow"))?;
        expected_len = expected_len
            .checked_add(expected_rect_len)
            .ok_or_else(|| anyhow::anyhow!("BGRA rectangle payload length overflow"))?;
        anyhow::ensure!(
            expected_len <= MAX_FRAME_BYTES,
            "BGRA rectangle payload exceeds {MAX_FRAME_BYTES} bytes"
        );
        anyhow::ensure!(rect.width > 0 && rect.height > 0, "BGRA rectangle is empty");
        anyhow::ensure!(
            u32::from(rect.x) + u32::from(rect.width) <= u32::from(width)
                && u32::from(rect.y) + u32::from(rect.height) <= u32::from(height),
            "BGRA rectangle is outside framebuffer"
        );
        anyhow::ensure!(
            rect.byte_len == expected_rect_len,
            "invalid BGRA rectangle byte length"
        );
    }
    anyhow::ensure!(
        bgra_len == expected_len,
        "invalid BGRA rectangle payload length"
    );
    Ok(())
}

fn remote_frame_rect(rect: HelperFrameRect) -> RemoteDesktopFrameRect {
    RemoteDesktopFrameRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        byte_len: rect.byte_len,
    }
}

fn read_binary_frame_output<R>(
    reader: &mut R,
    width: u16,
    height: u16,
    rgba_len: usize,
) -> anyhow::Result<HelperOutput>
where
    R: Read + ?Sized,
{
    validate_full_frame_len(width, height, rgba_len, "binary")?;
    let mut rgba = vec![0; rgba_len];
    reader.read_exact(&mut rgba)?;
    Ok(HelperOutput {
        output: Some(RemoteDesktopOutput::Frame {
            width,
            height,
            rgba,
        }),
        connected: false,
        disconnect: None,
        server_hello: None,
        pong: None,
    })
}

fn read_binary_bgra_frame_output<R>(
    reader: &mut R,
    width: u16,
    height: u16,
    bgra_len: usize,
) -> anyhow::Result<HelperOutput>
where
    R: Read + ?Sized,
{
    validate_full_frame_len(width, height, bgra_len, "BGRA")?;
    let mut bgra = vec![0; bgra_len];
    reader.read_exact(&mut bgra)?;
    Ok(HelperOutput {
        output: Some(RemoteDesktopOutput::FrameBgra {
            width,
            height,
            bgra,
        }),
        connected: false,
        disconnect: None,
        server_hello: None,
        pong: None,
    })
}

fn validate_full_frame_len(
    width: u16,
    height: u16,
    actual_len: usize,
    format: &str,
) -> anyhow::Result<()> {
    let expected_len = validate_frame_extent(width, height)?;
    anyhow::ensure!(
        actual_len == expected_len,
        "invalid {format} frame payload length: expected {expected_len}, got {actual_len}"
    );
    Ok(())
}

fn validate_frame_extent(width: u16, height: u16) -> anyhow::Result<usize> {
    anyhow::ensure!(width > 0 && height > 0, "frame dimensions must be non-zero");
    anyhow::ensure!(
        width <= MAX_FRAME_DIMENSION && height <= MAX_FRAME_DIMENSION,
        "frame dimensions exceed {MAX_FRAME_DIMENSION}"
    );
    let len = usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("frame payload length overflow"))?;
    anyhow::ensure!(
        len <= MAX_FRAME_BYTES,
        "frame payload exceeds {MAX_FRAME_BYTES} bytes"
    );
    Ok(len)
}

#[cfg(test)]
#[path = "transport_frames_tests.rs"]
mod tests;
