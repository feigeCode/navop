use super::*;

impl RemoteDesktopView {
    pub(super) fn start_runtime(&mut self, size: (u16, u16)) {
        if self.input_tx.is_some() {
            return;
        }
        let runtime = create_backend(self.options.clone())
            .start(RemoteDesktopSize {
                width: size.0,
                height: size.1,
            })
            .unwrap_or_else(failed_runtime);
        self.input_tx = Some(runtime.input_tx);
        self.output_rx = Some(runtime.output_rx);
        self.last_resize_size = Some(size);
        self.status = SharedString::from("Connecting");
    }

    pub(super) fn drain_output(&mut self, cx: &mut Context<Self>) {
        let Some(output_rx) = self.output_rx.as_ref() else {
            return;
        };
        let batch = output_rx.drain();
        for output in batch.control.into_iter().chain(batch.latest_frame) {
            self.apply_output(output, cx);
        }
    }

    fn apply_output(&mut self, output: RemoteDesktopOutput, cx: &mut Context<Self>) {
        match output {
            RemoteDesktopOutput::Connected { width, height, .. } => {
                self.remote_size = Some((width, height));
                self.status = SharedString::from("Connected");
            }
            RemoteDesktopOutput::Frame {
                width,
                height,
                rgba,
            } => {
                self.remote_size = Some((width, height));
                self.install_frame(rgba_to_render_image(width, height, rgba));
            }
            RemoteDesktopOutput::FrameBgra {
                width,
                height,
                bgra,
            } => {
                self.remote_size = Some((width, height));
                self.install_frame(bgra_to_render_image(width, height, bgra));
            }
            RemoteDesktopOutput::Status(message) => self.status = SharedString::from(message),
            RemoteDesktopOutput::ConnectionFailure(message)
            | RemoteDesktopOutput::Terminated(message) => self.handle_disconnect_status(message),
            RemoteDesktopOutput::CursorDefault
            | RemoteDesktopOutput::CursorHidden
            | RemoteDesktopOutput::CursorPosition { .. } => {}
            RemoteDesktopOutput::ClipboardText { text } => self.apply_remote_clipboard(text, cx),
        }
    }

    fn install_frame(&mut self, image: anyhow::Result<RenderImage>) {
        match image {
            Ok(image) => self.latest_frame = Some(Arc::new(image)),
            Err(error) => self.status = SharedString::from(error.to_string()),
        }
    }

    fn apply_remote_clipboard(&mut self, text: String, cx: &mut Context<Self>) {
        if self.last_clipboard_text.as_deref() == Some(text.as_str()) {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        self.last_clipboard_text = Some(text);
        self.last_clipboard_sync_at = Some(Instant::now());
    }

    pub(super) fn sync_local_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            return;
        }
        if self
            .last_clipboard_sync_at
            .is_some_and(|synced_at| synced_at.elapsed() < CLIPBOARD_SYNC_INTERVAL)
        {
            return;
        }
        self.last_clipboard_sync_at = Some(Instant::now());
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if self.last_clipboard_text.as_deref() == Some(text.as_str()) {
            return;
        }
        self.last_clipboard_text = Some(text.clone());
        self.send_input(RemoteDesktopInput::ClipboardText { text });
    }

    fn handle_disconnect_status(&mut self, message: String) {
        self.modifiers = Modifiers::default();
        self.status = SharedString::from(message);
    }

    pub(super) fn request_reconnect(&mut self) {
        self.modifiers = Modifiers::default();
        self.status = SharedString::from("reconnecting RDP session");
        self.send_input(RemoteDesktopInput::Reconnect);
    }

    pub(super) fn update_content_bounds(
        &mut self,
        bounds: Bounds<Pixels>,
        display_scale_factor: f32,
    ) {
        self.content_bounds = Some(bounds);
        let Some(size) = resize::resize_dimensions(bounds, display_scale_factor) else {
            return;
        };
        self.start_runtime(size);
        if !resize::is_meaningful_delta(self.last_resize_size, size)
            || self.pending_resize_size == Some(size)
        {
            return;
        }
        self.pending_resize_size = Some(size);
        self.pending_resize_updated_at = Some(Instant::now());
    }

    pub(super) fn flush_pending_resize(&mut self) {
        if self.remote_size.is_none() {
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
        });
    }
}
