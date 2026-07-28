use super::*;
use remote_desktop::RemoteDesktopCursor;

impl RemoteDesktopView {
    pub(super) fn start_runtime(&mut self, size: (u16, u16)) {
        if self.input_tx.is_some() {
            return;
        }
        self.frame_sync.reset_session();
        self.capabilities = None;
        self.framebuffer = None;
        self.remote_size = None;
        self.cursor.reset_session();
        let runtime = create_backend(self.options.clone())
            .start(RemoteDesktopSize {
                width: size.0,
                height: size.1,
                scale_factor: self.display_scale_factor,
            })
            .unwrap_or_else(failed_runtime);
        self.input_tx = Some(runtime.input_tx);
        self.output_rx = Some(runtime.output_rx);
        self.last_resize_size = Some(size);
        self.connected = false;
        self.status = SharedString::from(t!("RemoteDesktop.status_connecting").to_string());
    }

    pub(super) fn drain_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(output_rx) = self.output_rx.as_ref() else {
            return;
        };
        let batch = output_rx.drain();
        for output in batch
            .control
            .into_iter()
            .chain(batch.latest_frame)
            .chain(batch.latest_delta)
        {
            self.apply_output(output, window, cx);
        }
    }

    fn apply_output(
        &mut self,
        output: RemoteDesktopOutput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match output {
            RemoteDesktopOutput::Connected {
                width,
                height,
                capabilities,
            } => {
                self.remote_size = Some((width, height));
                self.capabilities = Some(capabilities);
                self.connected = true;
                self.frame_sync.connected();
                self.status = SharedString::from(t!("RemoteDesktop.status_connected").to_string());
            }
            RemoteDesktopOutput::Frame {
                width,
                height,
                rgba,
            } => {
                if !self.connected {
                    return;
                }
                if self.install_rgba_frame(width, height, rgba) {
                    self.remote_size = Some((width, height));
                    self.frame_sync.accept_base((width, height));
                }
            }
            RemoteDesktopOutput::FrameBgra {
                width,
                height,
                bgra,
            } => {
                if !self.connected {
                    return;
                }
                if self.install_bgra_frame(width, height, bgra) {
                    self.remote_size = Some((width, height));
                    self.frame_sync.accept_base((width, height));
                }
            }
            RemoteDesktopOutput::FrameBgraRects {
                width,
                height,
                rects,
                bgra,
            } => {
                if !self.connected {
                    return;
                }
                self.apply_bgra_rects(width, height, &rects, bgra);
            }
            RemoteDesktopOutput::Reconnecting(reconnect) => {
                self.reset_session_state(None, SessionResetReason::Reconnecting);
                self.notify_reconnecting(reconnect, window, cx);
            }
            RemoteDesktopOutput::Status(message) => self.status = SharedString::from(message),
            RemoteDesktopOutput::ConnectionFailure(message) => {
                self.reset_session_state(Some(message), SessionResetReason::ConnectionFailure)
            }
            RemoteDesktopOutput::Terminated(message) => {
                self.reset_session_state(Some(message), SessionResetReason::Terminated)
            }
            output @ (RemoteDesktopOutput::CursorDefault
            | RemoteDesktopOutput::CursorHidden
            | RemoteDesktopOutput::CursorPosition { .. }
            | RemoteDesktopOutput::CursorBitmap(_)) => self.apply_cursor_output(output),
            RemoteDesktopOutput::ClipboardText { text } => self.apply_remote_clipboard(text, cx),
            RemoteDesktopOutput::ClipboardFilesReady { transfer_id, paths } => {
                self.apply_remote_clipboard_files(transfer_id, paths, window, cx)
            }
            RemoteDesktopOutput::ClipboardTransferFailed {
                transfer_id,
                message,
            } => {
                tracing::warn!(
                    transfer_id,
                    error = %message,
                    "remote desktop clipboard transfer failed"
                );
                self.notify_clipboard_transfer_failed(window, cx);
            }
        }
    }

    fn reset_session_state(&mut self, message: Option<String>, reason: SessionResetReason) {
        self.modifiers = Modifiers::default();
        self.connected = false;
        if let Some(message) = message {
            self.status = SharedString::from(message);
        }
        self.capabilities = None;
        self.frame_sync.reset_session();
        self.remote_size = None;
        self.framebuffer = None;
        self.cursor.reset_session();
        if !preserve_presented_frame_during_session_reset(reason) {
            self.pending_frame_drops.extend(
                self.rendered_frames
                    .take_all_distinct(self.latest_frame.take()),
            );
        }
        self.last_resize_size = None;
        self.pending_resize_size = None;
        self.pending_resize_updated_at = None;
        self.last_resize_sent_at = None;
    }

    fn apply_cursor_output(&mut self, output: RemoteDesktopOutput) {
        if !should_apply_remote_cursor_output(self.options.protocol) {
            return;
        }
        match output {
            RemoteDesktopOutput::CursorDefault => self.apply_cursor_default(),
            RemoteDesktopOutput::CursorHidden => self.apply_cursor_hidden(),
            RemoteDesktopOutput::CursorPosition { x, y } => self.apply_cursor_position(x, y),
            RemoteDesktopOutput::CursorBitmap(cursor) => self.apply_cursor_bitmap(cursor),
            _ => unreachable!("apply_cursor_output only accepts cursor outputs"),
        }
    }

    fn apply_cursor_default(&mut self) {
        if self.connected {
            self.cursor.show_default();
        }
    }

    fn apply_cursor_hidden(&mut self) {
        if self.connected {
            self.cursor.hide();
        }
    }

    fn apply_cursor_position(&mut self, x: u16, y: u16) {
        if self.connected {
            self.cursor.set_position(x, y);
        }
    }

    fn apply_cursor_bitmap(&mut self, cursor: RemoteDesktopCursor) {
        if !self.connected {
            return;
        }
        if let Err(error) = self.cursor.install(cursor) {
            tracing::warn!(?error, "failed to install remote desktop cursor");
        }
    }

    pub(super) fn update_content_bounds(
        &mut self,
        bounds: Bounds<Pixels>,
        display_scale_factor: f32,
    ) {
        self.content_bounds = Some(bounds);
        self.display_scale_factor = resize::scale_factor_percent(display_scale_factor);
        let Some(size) = resize::resize_dimensions(bounds, display_scale_factor) else {
            return;
        };
        if self.input_tx.is_none() && self.options.protocol == RemoteDesktopProtocol::Rdp {
            self.initial_size
                .observe(size, self.display_scale_factor, Instant::now());
            return;
        }
        self.start_runtime(size);
        if !resize::is_meaningful_delta(self.last_resize_size, size)
            || self.pending_resize_size == Some(size)
        {
            return;
        }
        self.pending_resize_size = Some(size);
        self.pending_resize_updated_at = Some(Instant::now());
    }

    pub(super) fn flush_pending_start(&mut self) {
        if self.input_tx.is_some() {
            return;
        }
        let Some((size, scale_factor)) = self
            .initial_size
            .take_ready(Instant::now(), RDP_INITIAL_LAYOUT_DEBOUNCE)
        else {
            return;
        };
        self.display_scale_factor = scale_factor;
        self.start_runtime(size);
    }

    pub(super) fn flush_pending_resize(&mut self) {
        if !resize::can_flush_pending_resize(self.connected, self.remote_size, self.capabilities) {
            if resize::should_consume_local_resize(
                self.connected,
                self.remote_size,
                self.capabilities,
            ) {
                if let (Some(size), Some(updated_at)) =
                    (self.pending_resize_size, self.pending_resize_updated_at)
                {
                    if updated_at.elapsed() >= RESIZE_DEBOUNCE {
                        self.pending_resize_size = None;
                        self.pending_resize_updated_at = None;
                        self.last_resize_size = Some(size);
                    }
                }
            }
            return;
        }
        let (Some(size), Some(updated_at)) =
            (self.pending_resize_size, self.pending_resize_updated_at)
        else {
            return;
        };
        if updated_at.elapsed() < RESIZE_DEBOUNCE
            || self
                .last_resize_sent_at
                .is_some_and(|sent_at| sent_at.elapsed() < RESIZE_MIN_INTERVAL)
        {
            return;
        }
        self.pending_resize_size = None;
        self.pending_resize_updated_at = None;
        self.last_resize_size = Some(size);
        self.last_resize_sent_at = Some(Instant::now());
        self.send_input(RemoteDesktopInput::Resize {
            width: size.0,
            height: size.1,
            scale_factor: self.display_scale_factor,
        });
    }
}

fn should_apply_remote_cursor_output(protocol: RemoteDesktopProtocol) -> bool {
    protocol == RemoteDesktopProtocol::Rdp
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
