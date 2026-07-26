use super::*;

impl RemoteDesktopView {
    pub(super) fn start_runtime(&mut self, size: (u16, u16)) {
        if self.input_tx.is_some() {
            return;
        }
        self.frame_sync.reset_session();
        self.capabilities = None;
        self.framebuffer = None;
        self.remote_size = None;
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
                let status = localized_reconnect_status(self.options.protocol);
                let notification =
                    localized_reconnect_notification(self.options.protocol, reconnect);
                self.reset_session_state(status, SessionResetReason::Reconnecting);
                self.notify_reconnecting(notification, window, cx);
            }
            RemoteDesktopOutput::Status(message) => self.status = SharedString::from(message),
            RemoteDesktopOutput::ConnectionFailure(message) => {
                self.reset_session_state(message, SessionResetReason::ConnectionFailure)
            }
            RemoteDesktopOutput::Terminated(message) => {
                self.reset_session_state(message, SessionResetReason::Terminated)
            }
            RemoteDesktopOutput::CursorDefault
            | RemoteDesktopOutput::CursorHidden
            | RemoteDesktopOutput::CursorPosition { .. } => {}
            RemoteDesktopOutput::ClipboardText { text } => self.apply_remote_clipboard(text, cx),
        }
    }

    fn notify_reconnecting(&self, message: String, window: &mut Window, cx: &mut Context<Self>) {
        let notification_id = ("remote-desktop-reconnect", cx.entity_id());
        window.defer(cx, move |window, cx| {
            window.push_notification(
                Notification::info(message)
                    .id1::<RemoteDesktopReconnectNotification>(notification_id)
                    .autohide(true),
                cx,
            );
        });
    }

    fn install_frame(&mut self, image: anyhow::Result<RenderImage>) -> bool {
        match image {
            Ok(image) => {
                self.latest_frame = Some(Arc::new(image));
                true
            }
            Err(error) => {
                self.status = SharedString::from(error.to_string());
                false
            }
        }
    }

    fn install_rgba_frame(&mut self, width: u16, height: u16, rgba: Vec<u8>) -> bool {
        self.install_bgra_frame(width, height, crate::pixels::rgba_to_bgra(rgba))
    }

    fn install_bgra_frame(&mut self, width: u16, height: u16, bgra: Vec<u8>) -> bool {
        let framebuffer = match RgbaFramebuffer::from_bgra(width, height, bgra) {
            Ok(framebuffer) => framebuffer,
            Err(error) => {
                self.status = SharedString::from(error.to_string());
                return false;
            }
        };
        let image = match bgra_to_render_image(width, height, framebuffer.clone_rgba()) {
            Ok(image) => image,
            Err(error) => {
                self.status = SharedString::from(error.to_string());
                return false;
            }
        };
        self.framebuffer = Some(framebuffer);
        self.install_frame(Ok(image))
    }

    fn apply_bgra_rects(
        &mut self,
        width: u16,
        height: u16,
        rects: &[RemoteDesktopFrameRect],
        bgra: Vec<u8>,
    ) {
        if !self.frame_sync.can_apply_delta((width, height)) {
            self.record_rejected_delta(width, height, "delta has no matching base frame");
            return;
        }
        let Some(framebuffer) = self.framebuffer.as_ref() else {
            self.record_rejected_delta(width, height, "missing base framebuffer");
            return;
        };
        let patched = match patched_bgra_framebuffer(framebuffer, width, height, rects, &bgra) {
            Ok(patched) => patched,
            Err(error) => {
                let reason = error.to_string();
                self.record_rejected_delta(width, height, &reason);
                return;
            }
        };
        let image = match bgra_to_render_image(width, height, patched.clone_rgba()) {
            Ok(image) => image,
            Err(error) => {
                self.record_rejected_delta(width, height, "failed to build delta frame");
                self.status = SharedString::from(error.to_string());
                return;
            }
        };
        match self.frame_sync.accept_delta((width, height)) {
            frame_sync::DeltaDisposition::Applied => {
                self.framebuffer = Some(patched);
                self.latest_frame = Some(Arc::new(image));
                self.remote_size = Some((width, height));
            }
            disposition @ frame_sync::DeltaDisposition::Rejected { .. } => {
                // All presentation state is owned by this `&mut self`, so the
                // preflight above should make this unreachable. Keep the
                // branch defensive and, critically, do not install `patched`.
                self.log_rejected_delta(
                    width,
                    height,
                    "delta synchronization changed before commit",
                    disposition,
                );
            }
        }
    }

    fn record_rejected_delta(&mut self, width: u16, height: u16, reason: &str) {
        let disposition = self.frame_sync.reject_delta();
        self.log_rejected_delta(width, height, reason, disposition);
    }

    fn log_rejected_delta(
        &self,
        width: u16,
        height: u16,
        reason: &str,
        disposition: frame_sync::DeltaDisposition,
    ) {
        if let frame_sync::DeltaDisposition::Rejected { recovery_started } = disposition {
            let snapshot = self.frame_sync.snapshot();
            let resize_capability = self.capabilities.map(|capabilities| capabilities.resize);
            if recovery_started {
                tracing::warn!(
                    protocol = self.options.protocol.label(),
                    session_generation = snapshot.session_generation,
                    phase = ?snapshot.phase,
                    base_size = ?snapshot.base_size,
                    remote_size = ?self.remote_size,
                    viewport_size = ?self.last_resize_size,
                    resize_capability = ?resize_capability,
                    width,
                    height,
                    full_frames = snapshot.full_frames,
                    deltas = snapshot.deltas,
                    dropped_deltas = snapshot.dropped_deltas,
                    recoveries = snapshot.recoveries,
                    reason,
                    "remote desktop frame recovery required"
                );
            } else {
                tracing::debug!(
                    protocol = self.options.protocol.label(),
                    session_generation = snapshot.session_generation,
                    phase = ?snapshot.phase,
                    base_size = ?snapshot.base_size,
                    remote_size = ?self.remote_size,
                    viewport_size = ?self.last_resize_size,
                    resize_capability = ?resize_capability,
                    width,
                    height,
                    full_frames = snapshot.full_frames,
                    deltas = snapshot.deltas,
                    dropped_deltas = snapshot.dropped_deltas,
                    recoveries = snapshot.recoveries,
                    reason,
                    "dropping remote desktop delta while awaiting a base frame"
                );
            }
        }
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

    fn reset_session_state(&mut self, message: String, reason: SessionResetReason) {
        self.modifiers = Modifiers::default();
        self.connected = false;
        self.status = SharedString::from(message);
        self.capabilities = None;
        self.frame_sync.reset_session();
        self.remote_size = None;
        self.framebuffer = None;
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

fn localized_reconnect_status(protocol: RemoteDesktopProtocol) -> String {
    let locale = rust_i18n::locale();
    localized_reconnect_status_for_locale(locale.as_ref(), protocol)
}

fn localized_reconnect_status_for_locale(locale: &str, protocol: RemoteDesktopProtocol) -> String {
    t!(
        "RemoteDesktop.status_reconnecting",
        locale = locale,
        protocol = protocol.label()
    )
    .to_string()
}

fn localized_reconnect_notification(
    protocol: RemoteDesktopProtocol,
    reconnect: RemoteDesktopReconnect,
) -> String {
    let locale = rust_i18n::locale();
    localized_reconnect_notification_for_locale(locale.as_ref(), protocol, reconnect)
}

fn localized_reconnect_notification_for_locale(
    locale: &str,
    protocol: RemoteDesktopProtocol,
    reconnect: RemoteDesktopReconnect,
) -> String {
    let reason = match reconnect.reason {
        RemoteDesktopReconnectReason::DisplayUpdate => t!(
            "RemoteDesktop.reconnect_reason_display_update",
            locale = locale
        ),
        RemoteDesktopReconnectReason::SessionError => t!(
            "RemoteDesktop.reconnect_reason_session_error",
            locale = locale
        ),
        RemoteDesktopReconnectReason::ConnectionLost => t!(
            "RemoteDesktop.reconnect_reason_connection_lost",
            locale = locale
        ),
        RemoteDesktopReconnectReason::Manual => {
            return t!(
                "RemoteDesktop.reconnect_notification_manual",
                locale = locale,
                protocol = protocol.label()
            )
            .to_string();
        }
    };
    let Some(seconds) = reconnect.delay_secs else {
        return t!(
            "RemoteDesktop.reconnect_notification_manual",
            locale = locale,
            protocol = protocol.label()
        )
        .to_string();
    };

    t!(
        "RemoteDesktop.reconnect_notification",
        locale = locale,
        protocol = protocol.label(),
        reason = reason,
        seconds = seconds
    )
    .to_string()
}

fn patched_bgra_framebuffer(
    framebuffer: &RgbaFramebuffer,
    width: u16,
    height: u16,
    rects: &[RemoteDesktopFrameRect],
    bgra: &[u8],
) -> anyhow::Result<RgbaFramebuffer> {
    anyhow::ensure!(
        framebuffer.width() == width && framebuffer.height() == height,
        "base framebuffer dimensions changed"
    );

    let mut patched = framebuffer.clone();
    let mut offset = 0usize;
    for rect in rects {
        anyhow::ensure!(
            rect.width > 0 && rect.height > 0,
            "dirty rectangle is empty"
        );
        let end = offset
            .checked_add(rect.byte_len)
            .ok_or_else(|| anyhow::anyhow!("dirty rectangle payload length overflow"))?;
        anyhow::ensure!(end <= bgra.len(), "dirty rectangle payload is truncated");
        patched
            .patch_rgba_rect(rect.x, rect.y, rect.width, rect.height, &bgra[offset..end])
            .map_err(|_| anyhow::anyhow!("invalid dirty rectangle"))?;
        offset = end;
    }
    anyhow::ensure!(
        offset == bgra.len(),
        "dirty rectangle payload has trailing bytes"
    );
    Ok(patched)
}

#[cfg(test)]
mod tests {
    use remote_desktop::{
        RemoteDesktopFrameRect, RemoteDesktopProtocol, RemoteDesktopReconnect,
        RemoteDesktopReconnectReason, RgbaFramebuffer,
    };

    use super::{
        localized_reconnect_notification_for_locale, localized_reconnect_status_for_locale,
        patched_bgra_framebuffer,
    };

    #[test]
    fn localizes_rdp_reconnect_notification_in_english() {
        let notification = localized_reconnect_notification_for_locale(
            "en",
            RemoteDesktopProtocol::Rdp,
            RemoteDesktopReconnect {
                reason: RemoteDesktopReconnectReason::DisplayUpdate,
                delay_secs: Some(1),
            },
        );

        assert_eq!(
            "RDP disconnected: display update error. Reconnecting in 1s",
            notification
        );
        assert!(!notification.contains("VNC"));
    }

    #[test]
    fn localizes_vnc_reconnect_notification_in_simplified_chinese() {
        let notification = localized_reconnect_notification_for_locale(
            "zh-CN",
            RemoteDesktopProtocol::Vnc,
            RemoteDesktopReconnect {
                reason: RemoteDesktopReconnectReason::ConnectionLost,
                delay_secs: Some(2),
            },
        );

        assert_eq!(
            "VNC 连接已断开：连接丢失。将在 2 秒后重新连接",
            notification
        );
        assert!(!notification.contains("RDP"));
    }

    #[test]
    fn localizes_manual_reconnect_and_status_in_traditional_chinese() {
        let reconnect = RemoteDesktopReconnect {
            reason: RemoteDesktopReconnectReason::Manual,
            delay_secs: None,
        };

        assert_eq!(
            "正在重新連線 RDP 工作階段",
            localized_reconnect_notification_for_locale(
                "zh-HK",
                RemoteDesktopProtocol::Rdp,
                reconnect
            )
        );
        assert_eq!(
            "正在重新連線 VNC 工作階段",
            localized_reconnect_status_for_locale("zh-HK", RemoteDesktopProtocol::Vnc)
        );
    }

    #[test]
    fn localizes_session_error_without_accepting_backend_details() {
        let notification = localized_reconnect_notification_for_locale(
            "zh-CN",
            RemoteDesktopProtocol::Rdp,
            RemoteDesktopReconnect {
                reason: RemoteDesktopReconnectReason::SessionError,
                delay_secs: Some(5),
            },
        );

        assert_eq!(
            "RDP 连接已断开：会话错误。将在 5 秒后重新连接",
            notification
        );
        assert!(!notification.contains("/Users/"));
        assert!(!notification.contains(".cargo/git/checkouts"));
    }

    #[test]
    fn patches_dirty_rectangles_without_mutating_the_base() {
        let base =
            RgbaFramebuffer::from_bgra(2, 1, vec![0x03, 0x02, 0x01, 0xff, 0x06, 0x05, 0x04, 0xff])
                .unwrap();
        let rects = [RemoteDesktopFrameRect {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        }];

        let patched =
            patched_bgra_framebuffer(&base, 2, 1, &rects, &[0x30, 0x20, 0x10, 0xff]).unwrap();

        assert_eq!(
            base.as_rgba(),
            &[0x03, 0x02, 0x01, 0xff, 0x06, 0x05, 0x04, 0xff]
        );
        assert_eq!(
            patched.as_rgba(),
            &[0x03, 0x02, 0x01, 0xff, 0x30, 0x20, 0x10, 0xff]
        );
    }

    #[test]
    fn rejects_an_invalid_delta_atomically() {
        let base =
            RgbaFramebuffer::from_bgra(2, 1, vec![0x03, 0x02, 0x01, 0xff, 0, 0, 0, 0]).unwrap();
        let rects = [
            RemoteDesktopFrameRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                byte_len: 4,
            },
            RemoteDesktopFrameRect {
                x: 2,
                y: 0,
                width: 1,
                height: 1,
                byte_len: 4,
            },
        ];

        let result = patched_bgra_framebuffer(
            &base,
            2,
            1,
            &rects,
            &[0x30, 0x20, 0x10, 0xff, 0x60, 0x50, 0x40, 0xff],
        );

        assert!(result.is_err());
        assert_eq!(
            base.as_rgba(),
            &[0x03, 0x02, 0x01, 0xff, 0, 0, 0, 0],
            "a rejected delta must not partially patch its base"
        );
    }

    #[test]
    fn rejects_delta_payload_with_trailing_bytes() {
        let base = RgbaFramebuffer::from_bgra(1, 1, vec![0, 0, 0, 0]).unwrap();
        let rects = [RemoteDesktopFrameRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            byte_len: 4,
        }];

        assert!(patched_bgra_framebuffer(&base, 1, 1, &rects, &[1, 2, 3, 4, 5]).is_err());
        assert_eq!(base.as_rgba(), &[0, 0, 0, 0]);
    }
}
