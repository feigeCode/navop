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
                scale_factor: self.display_scale_factor,
            })
            .unwrap_or_else(failed_runtime);
        self.input_tx = Some(runtime.input_tx);
        self.output_rx = Some(runtime.output_rx);
        self.last_resize_size = Some(size);
        self.connected = false;
        self.status = SharedString::from(t!("RemoteDesktop.status_connecting").to_string());
    }

    pub(super) fn drain_output(&mut self, cx: &mut Context<Self>) {
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
            self.apply_output(output, cx);
        }
    }

    fn apply_output(&mut self, output: RemoteDesktopOutput, cx: &mut Context<Self>) {
        match output {
            RemoteDesktopOutput::Connected { width, height, .. } => {
                self.remote_size = Some((width, height));
                self.connected = true;
                self.status = SharedString::from(t!("RemoteDesktop.status_connected").to_string());
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
                self.install_bgra_frame(width, height, bgra);
            }
            RemoteDesktopOutput::FrameBgraRects {
                width,
                height,
                rects,
                bgra,
            } => {
                self.remote_size = Some((width, height));
                self.apply_bgra_rects(width, height, &rects, bgra);
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

    fn install_bgra_frame(&mut self, width: u16, height: u16, bgra: Vec<u8>) {
        match RgbaFramebuffer::from_bgra(width, height, bgra) {
            Ok(framebuffer) => {
                let image = bgra_to_render_image(width, height, framebuffer.clone_rgba());
                self.framebuffer = Some(framebuffer);
                self.install_frame(image);
            }
            Err(error) => self.status = SharedString::from(error.to_string()),
        }
    }

    fn apply_bgra_rects(
        &mut self,
        width: u16,
        height: u16,
        rects: &[RemoteDesktopFrameRect],
        bgra: Vec<u8>,
    ) {
        let Some(framebuffer) = self.framebuffer.as_mut() else {
            return;
        };
        if framebuffer.width() != width || framebuffer.height() != height {
            return;
        }
        let mut offset: usize = 0;
        for rect in rects {
            let end = offset.saturating_add(rect.byte_len);
            if end > bgra.len()
                || framebuffer
                    .patch_rgba_rect(rect.x, rect.y, rect.width, rect.height, &bgra[offset..end])
                    .is_err()
            {
                return;
            }
            offset = end;
        }
        if offset != bgra.len() {
            return;
        }
        let image = bgra_to_render_image(width, height, framebuffer.clone_rgba());
        self.install_frame(image);
    }

    fn apply_remote_clipboard(&mut self, text: String, cx: &mut Context<Self>) {
        if self.last_clipboard_text.as_deref() == Some(text.as_str()) {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        self.last_clipboard_text = Some(text);
        self.last_clipboard_files = None;
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
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let files = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::ExternalPaths(paths) => {
                let paths: Vec<String> = paths
                    .paths()
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect();
                (!paths.is_empty()).then_some(paths)
            }
            ClipboardEntry::Image(_) | ClipboardEntry::String(_) => None,
        });
        if let Some(paths) = files {
            if self.last_clipboard_files.as_ref() == Some(&paths) {
                return;
            }
            self.last_clipboard_files = Some(paths.clone());
            self.last_clipboard_text = None;
            self.send_input(RemoteDesktopInput::ClipboardFiles { paths });
            return;
        }
        let Some(text) = item.text() else {
            return;
        };
        if self.last_clipboard_text.as_deref() == Some(text.as_str()) {
            return;
        }
        self.last_clipboard_text = Some(text.clone());
        self.last_clipboard_files = None;
        self.send_input(RemoteDesktopInput::ClipboardText { text });
    }

    fn handle_disconnect_status(&mut self, message: String) {
        self.modifiers = Modifiers::default();
        self.connected = false;
        self.status = SharedString::from(message);
    }

    pub(super) fn request_reconnect(&mut self) {
        self.modifiers = Modifiers::default();
        self.connected = false;
        self.status = SharedString::from(t!("RemoteDesktop.status_reconnecting").to_string());
        self.send_input(RemoteDesktopInput::Reconnect);
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
            scale_factor: self.display_scale_factor,
        });
    }
}
