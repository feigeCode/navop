use std::time::{Duration, Instant};

use vnc_client::{Rect, VncClient, VncEvent, X11Event};

use super::vnc_input::VncPointerState;
use crate::framebuffer::RgbaFramebuffer;
use crate::{RemoteDesktopCapabilities, RemoteDesktopOutput, ResizeSupport};

const VNC_REFRESH_INTERVAL: Duration = Duration::from_millis(33);

pub(crate) struct ConnectedVncSession {
    pub(crate) client: VncClient,
    pub(crate) pointer: VncPointerState,
    pub(crate) was_connected: bool,
    framebuffer: Option<RgbaFramebuffer>,
    last_refresh: Instant,
}

impl ConnectedVncSession {
    pub(crate) fn new(client: VncClient) -> Self {
        Self {
            client,
            pointer: VncPointerState::default(),
            was_connected: false,
            framebuffer: None,
            last_refresh: Instant::now(),
        }
    }

    pub(crate) async fn poll_events(
        &mut self,
        output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
    ) -> Result<(), String> {
        loop {
            match self.client.poll_event().await {
                Ok(Some(event)) => self.handle_event(event, output_tx)?,
                Ok(None) => return Ok(()),
                Err(error) => return Err(error.to_string()),
            }
        }
    }

    pub(crate) async fn refresh_if_needed(&mut self) -> Result<(), String> {
        if self.last_refresh.elapsed() < VNC_REFRESH_INTERVAL {
            return Ok(());
        }
        self.client
            .input(X11Event::Refresh)
            .await
            .map_err(|error| error.to_string())?;
        self.last_refresh = Instant::now();
        Ok(())
    }

    fn handle_event(
        &mut self,
        event: VncEvent,
        output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
    ) -> Result<(), String> {
        match event {
            VncEvent::SetResolution(screen) => self.set_resolution(screen, output_tx),
            VncEvent::RawImage(rect, data) => self.patch_rect(rect, &data, output_tx)?,
            VncEvent::Copy(dst, src) => self.copy_rect(dst, src, output_tx)?,
            VncEvent::Text(text) => send_clipboard(output_tx, text),
            VncEvent::Error(message) => return Err(message),
            VncEvent::JpegImage(_, _) => {
                send_status(output_tx, "VNC JPEG rectangles are not enabled")
            }
            VncEvent::Bell | VncEvent::SetPixelFormat(_) | VncEvent::SetCursor(_, _) => {}
            _ => {}
        }
        Ok(())
    }

    fn set_resolution(
        &mut self,
        screen: vnc_client::Screen,
        output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
    ) {
        self.framebuffer = Some(RgbaFramebuffer::new(screen.width, screen.height));
        self.was_connected = true;
        let _ = output_tx.send(RemoteDesktopOutput::Connected {
            width: screen.width,
            height: screen.height,
            capabilities: vnc_capabilities(),
        });
    }

    fn patch_rect(
        &mut self,
        rect: Rect,
        data: &[u8],
        output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
    ) -> Result<(), String> {
        let Some(framebuffer) = &mut self.framebuffer else {
            return Ok(());
        };
        framebuffer
            .patch_rgba_rect(rect.x, rect.y, rect.width, rect.height, data)
            .map_err(|error| error.to_string())?;
        send_frame(framebuffer, output_tx);
        Ok(())
    }

    fn copy_rect(
        &mut self,
        dst: Rect,
        src: Rect,
        output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
    ) -> Result<(), String> {
        let Some(framebuffer) = &mut self.framebuffer else {
            return Ok(());
        };
        framebuffer
            .copy_rect(src.x, src.y, dst.x, dst.y, dst.width, dst.height)
            .map_err(|error| error.to_string())?;
        send_frame(framebuffer, output_tx);
        Ok(())
    }
}

fn send_frame(
    framebuffer: &RgbaFramebuffer,
    output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>,
) {
    let _ = output_tx.send(RemoteDesktopOutput::Frame {
        width: framebuffer.width(),
        height: framebuffer.height(),
        rgba: framebuffer.clone_rgba(),
    });
}

fn send_clipboard(output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>, text: String) {
    let _ = output_tx.send(RemoteDesktopOutput::ClipboardText { text });
}

fn vnc_capabilities() -> RemoteDesktopCapabilities {
    RemoteDesktopCapabilities {
        resize: ResizeSupport::LocalScaleOnly,
        clipboard_text: true,
        ..RemoteDesktopCapabilities::vnc_mvp()
    }
}

fn send_status(output_tx: &std::sync::mpsc::Sender<RemoteDesktopOutput>, message: &str) {
    let _ = output_tx.send(RemoteDesktopOutput::Status(message.to_string()));
}
