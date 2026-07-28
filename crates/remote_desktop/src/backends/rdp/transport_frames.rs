use std::io::{BufRead, Read};

use crate::{
    RemoteDesktopCursor, RemoteDesktopFrameRect,
    helper_protocol::{HelperEvent, HelperFrameRect, decode_event_line},
};

use super::helper_events::{helper_disconnect, helper_event_to_output};
use super::transport::HelperOutput;
use super::*;

const MAX_CURSOR_DIMENSION: u16 = 1024;

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
    if reader.read_until(b'\n', &mut line)? == 0 {
        return Ok(None);
    }
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
    })
}

fn validate_rects(
    width: u16,
    height: u16,
    rects: &[HelperFrameRect],
    bgra_len: usize,
) -> anyhow::Result<()> {
    let expected_len: usize = rects.iter().map(|rect| rect.byte_len).sum();
    anyhow::ensure!(
        bgra_len == expected_len,
        "invalid BGRA rectangle payload length"
    );
    for rect in rects {
        anyhow::ensure!(rect.width > 0 && rect.height > 0, "BGRA rectangle is empty");
        anyhow::ensure!(
            rect.x.saturating_add(rect.width) <= width
                && rect.y.saturating_add(rect.height) <= height,
            "BGRA rectangle is outside framebuffer"
        );
        anyhow::ensure!(
            rect.byte_len == usize::from(rect.width) * usize::from(rect.height) * 4,
            "invalid BGRA rectangle byte length"
        );
    }
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
    })
}

fn validate_full_frame_len(
    width: u16,
    height: u16,
    actual_len: usize,
    format: &str,
) -> anyhow::Result<()> {
    let expected_len = usize::from(width) * usize::from(height) * 4;
    anyhow::ensure!(
        actual_len == expected_len,
        "invalid {format} frame payload length: expected {expected_len}, got {actual_len}"
    );
    Ok(())
}

#[cfg(test)]
#[path = "transport_frames_tests.rs"]
mod tests;
