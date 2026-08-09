use super::notifications::localized_failure_message;
use super::*;
use remote_desktop::RemoteDesktopCursor;
use std::sync::atomic::Ordering;

impl RemoteDesktopView {
    pub(super) fn start_runtime(&mut self, size: (u16, u16), cx: &mut Context<Self>) {
        if self.input_tx.is_some() {
            return;
        }
        let started_at = Instant::now();
        self.runtime_started_at = Some(started_at);
        self.startup_connected_logged = false;
        self.startup_frame_logged = false;
        tracing::info!(
            protocol = self.options.protocol.label(),
            width = size.0,
            height = size.1,
            scale_factor = self.display_scale_factor,
            startup_elapsed_ms = started_at
                .saturating_duration_since(self.startup_started_at)
                .as_millis() as u64,
            "starting remote desktop runtime"
        );
        self.frame_sync.reset_session();
        let generation = self.frame_sync.snapshot().session_generation;
        self.capabilities = None;
        self.remote_size = None;
        self.cursor.reset_session();
        self.start_presentation_worker(cx);
        self.supersede_pending_presentation_frames();
        self.enqueue_presentation(presentation::PresentationCommand::Reset { generation }, cx);
        let runtime = create_backend(self.options.clone())
            .start(RemoteDesktopSize {
                width: size.0,
                height: size.1,
                scale_factor: self.display_scale_factor,
            })
            .unwrap_or_else(failed_runtime);
        let mut output_ready = runtime.output_rx.subscribe();
        let output_ready_task = cx.spawn(async move |this, cx| {
            while output_ready.wait().await.is_ok() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        self.input_tx = Some(runtime.input_tx);
        self.output_rx = Some(runtime.output_rx);
        self._output_ready_task = Some(output_ready_task);
        self.last_resize_size = Some(size);
        self.connected = false;
        self.status = SharedString::from(t!("RemoteDesktop.status_connecting").to_string());
    }

    fn start_presentation_worker(&mut self, cx: &mut Context<Self>) {
        if self.presentation_tx.is_some() {
            return;
        }

        let (presentation_tx, mut presentation_rx) =
            tokio::sync::mpsc::unbounded_channel::<presentation::PresentationCommand>();
        let latest_frame_ticket = self.latest_presentation_frame_ticket.clone();
        let presentation_task = cx.spawn(async move |this, cx| {
            let mut state = presentation::PresentationState::default();
            while let Some(command) = presentation_rx.recv().await {
                let latest_frame_ticket = latest_frame_ticket.clone();
                let processed = cx
                    .background_executor()
                    .spawn(async move { state.process(command, latest_frame_ticket.as_ref()) })
                    .await;
                state = processed.state;
                if this
                    .update(cx, |view, cx| {
                        view.finish_presentation(processed.result, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        self.presentation_tx = Some(presentation_tx);
        self._presentation_task = Some(presentation_task);
    }

    fn enqueue_presentation(
        &mut self,
        command: presentation::PresentationCommand,
        cx: &mut Context<Self>,
    ) {
        self.presentation_queue.push(command);
        self.pump_presentation_queue(cx);
    }

    fn pump_presentation_queue(&mut self, _cx: &mut Context<Self>) {
        if self.presentation_in_flight {
            return;
        }
        let Some(command) = self.presentation_queue.pop_front() else {
            return;
        };
        let Some(presentation_tx) = self.presentation_tx.as_ref() else {
            return;
        };
        if presentation_tx.send(command).is_ok() {
            self.presentation_in_flight = true;
        }
    }

    fn finish_presentation(
        &mut self,
        result: presentation::PresentationResult,
        cx: &mut Context<Self>,
    ) {
        self.presentation_in_flight = false;
        let generation = self.frame_sync.snapshot().session_generation;
        let mut should_notify = false;
        match result {
            presentation::PresentationResult::Acknowledged
            | presentation::PresentationResult::Skipped => {}
            presentation::PresentationResult::RejectedFrame {
                generation: result_generation,
                width,
                height,
                reason,
            } => {
                if result_generation == generation {
                    self.status = SharedString::from(format!(
                        "remote desktop frame {width}x{height} rejected: {reason}"
                    ));
                    should_notify = true;
                }
            }
            presentation::PresentationResult::RejectedDelta {
                generation: result_generation,
                width,
                height,
                reason,
            } => {
                if result_generation == generation {
                    self.record_rejected_delta(width, height, &reason);
                    should_notify = true;
                }
            }
            presentation::PresentationResult::Prepared(frame) => {
                if frame.generation == generation {
                    let has_newer_frame =
                        self.presentation_queue.has_pending_frame(frame.generation);
                    match frame.kind {
                        presentation::PreparedFrameKind::Base { encoding } => {
                            self.remote_size = Some((frame.width, frame.height));
                            self.frame_sync.accept_base((frame.width, frame.height));
                            if !has_newer_frame {
                                if let Some(surface) = frame.surface {
                                    self.latest_frame = Some(surface);
                                    should_notify = true;
                                }
                            }
                            self.log_startup_frame(frame.width, frame.height, encoding);
                        }
                        presentation::PreparedFrameKind::Delta => {
                            match self.frame_sync.accept_delta((frame.width, frame.height)) {
                                frame_sync::DeltaDisposition::Applied if !has_newer_frame => {
                                    if let Some(surface) = frame.surface {
                                        self.remote_size = Some((frame.width, frame.height));
                                        self.latest_frame = Some(surface);
                                        should_notify = true;
                                    }
                                }
                                frame_sync::DeltaDisposition::Applied => {}
                                disposition @ frame_sync::DeltaDisposition::Rejected { .. } => {
                                    self.log_rejected_delta(
                                        frame.width,
                                        frame.height,
                                        "delta synchronization changed before commit",
                                        disposition,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        self.pump_presentation_queue(cx);
        if should_notify {
            cx.notify();
        }
    }

    pub(super) fn drain_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(output_rx) = self.output_rx.as_ref() else {
            return;
        };
        let drain_started_at = Instant::now();
        let batch = output_rx.drain();
        let stats = batch.stats;
        let before = self.frame_sync.snapshot();
        for output in batch
            .control
            .into_iter()
            .chain(batch.latest_frame)
            .chain(batch.latest_delta)
        {
            self.apply_output(output, window, cx);
        }
        if stats.outputs_received > 0 {
            let after = self.frame_sync.snapshot();
            let apply_elapsed = drain_started_at.elapsed();
            tracing::debug!(
                protocol = self.options.protocol.label(),
                session_generation = after.session_generation,
                outputs_received = stats.outputs_received,
                frame_received = stats.full_frames_received,
                delta_received = stats.delta_frames_received,
                frame_coalesced = stats.full_frames_coalesced,
                delta_merged = stats.delta_frames_merged,
                frame_dropped = stats.frames_dropped,
                frame_applied = after.full_frames.saturating_sub(before.full_frames),
                delta_applied = after.deltas.saturating_sub(before.deltas),
                delta_rejected = after.dropped_deltas.saturating_sub(before.dropped_deltas),
                payload_bytes = stats.payload_bytes,
                dirty_rects = stats.dirty_rects,
                output_wakeups = stats.wakeups,
                apply_elapsed_us = apply_elapsed.as_micros() as u64,
                over_frame_budget = apply_elapsed >= Duration::from_millis(16),
                "remote desktop output batch applied"
            );
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
                let generation = self.frame_sync.snapshot().session_generation;
                self.enqueue_presentation(
                    presentation::PresentationCommand::Connected { generation },
                    cx,
                );
                self.status = SharedString::from(t!("RemoteDesktop.status_connected").to_string());
                if !self.startup_connected_logged {
                    self.startup_connected_logged = true;
                    let now = Instant::now();
                    tracing::info!(
                        protocol = self.options.protocol.label(),
                        width,
                        height,
                        connect_elapsed_ms = self
                            .runtime_started_at
                            .map(|started_at| {
                                now.saturating_duration_since(started_at).as_millis() as u64
                            })
                            .unwrap_or_default(),
                        startup_elapsed_ms = now
                            .saturating_duration_since(self.startup_started_at)
                            .as_millis() as u64,
                        "remote desktop connected"
                    );
                }
            }
            RemoteDesktopOutput::Frame {
                width,
                height,
                rgba,
            } => {
                if !self.connected {
                    return;
                }
                let generation = self.frame_sync.snapshot().session_generation;
                let ticket = self.next_presentation_frame_ticket();
                self.enqueue_presentation(
                    presentation::PresentationCommand::Frame {
                        generation,
                        ticket,
                        frame: presentation::PresentationFrame::Rgba {
                            width,
                            height,
                            rgba,
                        },
                    },
                    cx,
                );
            }
            RemoteDesktopOutput::FrameBgra {
                width,
                height,
                bgra,
            } => {
                if !self.connected {
                    return;
                }
                let generation = self.frame_sync.snapshot().session_generation;
                let ticket = self.next_presentation_frame_ticket();
                self.enqueue_presentation(
                    presentation::PresentationCommand::Frame {
                        generation,
                        ticket,
                        frame: presentation::PresentationFrame::Bgra {
                            width,
                            height,
                            bgra,
                        },
                    },
                    cx,
                );
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
                let generation = self.frame_sync.snapshot().session_generation;
                let ticket = self.next_presentation_frame_ticket();
                self.enqueue_presentation(
                    presentation::PresentationCommand::Frame {
                        generation,
                        ticket,
                        frame: presentation::PresentationFrame::BgraRects {
                            width,
                            height,
                            rects,
                            bgra,
                        },
                    },
                    cx,
                );
            }
            RemoteDesktopOutput::Reconnecting(reconnect) => {
                self.reset_session_state(None, SessionResetReason::Reconnecting, cx);
                self.notify_reconnecting(reconnect, window, cx);
            }
            RemoteDesktopOutput::Status(message) => self.status = SharedString::from(message),
            RemoteDesktopOutput::ConnectionFailure(failure) => {
                self.apply_terminal_failure(
                    failure,
                    SessionResetReason::ConnectionFailure,
                    window,
                    cx,
                );
            }
            RemoteDesktopOutput::Terminated(failure) => {
                self.apply_terminal_failure(failure, SessionResetReason::Terminated, window, cx);
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

    fn apply_terminal_failure(
        &mut self,
        failure: RemoteDesktopFailure,
        reason: SessionResetReason,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if failure == RemoteDesktopFailure::SessionTakenOver {
            self.reset_session_state(None, SessionResetReason::Terminated, cx);
            self.notify_session_taken_over(window, cx);
            cx.emit(TabContentEvent::CloseRequested);
            return;
        }

        let message = localized_failure_message(&failure);
        self.reset_session_state(Some(message), reason, cx);
    }

    fn reset_session_state(
        &mut self,
        message: Option<String>,
        reason: SessionResetReason,
        cx: &mut Context<Self>,
    ) {
        self.keyboard_state = RdpKeyboardState::default();
        self.connected = false;
        if let Some(message) = message {
            self.status = SharedString::from(message);
        }
        self.capabilities = None;
        self.frame_sync.reset_session();
        let generation = self.frame_sync.snapshot().session_generation;
        self.remote_size = None;
        self.cursor.reset_session();
        self.supersede_pending_presentation_frames();
        self.enqueue_presentation(presentation::PresentationCommand::Reset { generation }, cx);
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

    fn next_presentation_frame_ticket(&self) -> u64 {
        self.latest_presentation_frame_ticket
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }

    fn supersede_pending_presentation_frames(&self) {
        let _ = self.next_presentation_frame_ticket();
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

    fn log_startup_frame(&mut self, width: u16, height: u16, encoding: &'static str) {
        if self.startup_frame_logged {
            return;
        }
        self.startup_frame_logged = true;
        let now = Instant::now();
        tracing::info!(
            protocol = self.options.protocol.label(),
            width,
            height,
            encoding,
            first_frame_elapsed_ms = self
                .runtime_started_at
                .map(|started_at| now.saturating_duration_since(started_at).as_millis() as u64)
                .unwrap_or_default(),
            startup_elapsed_ms = now
                .saturating_duration_since(self.startup_started_at)
                .as_millis() as u64,
            "remote desktop first frame received"
        );
    }

    pub(super) fn update_content_bounds(
        &mut self,
        bounds: Bounds<Pixels>,
        display_scale_factor: f32,
        cx: &mut Context<Self>,
    ) {
        self.content_bounds = Some(bounds);
        self.display_scale_factor = resize::scale_factor_percent(display_scale_factor);
        let Some(size) = resize::resize_dimensions(bounds, display_scale_factor) else {
            return;
        };
        if self.input_tx.is_none() && self.options.protocol == RemoteDesktopProtocol::Rdp {
            let observed_at = Instant::now();
            if let Some(generation) =
                self.initial_size
                    .observe(size, self.display_scale_factor, observed_at)
            {
                tracing::debug!(
                    protocol = self.options.protocol.label(),
                    generation,
                    width = size.0,
                    height = size.1,
                    scale_factor = self.display_scale_factor,
                    debounce_ms = RDP_INITIAL_LAYOUT_DEBOUNCE.as_millis() as u64,
                    startup_elapsed_ms = observed_at
                        .saturating_duration_since(self.startup_started_at)
                        .as_millis() as u64,
                    "remote desktop initial layout observed"
                );
                self.schedule_initial_start(generation, cx);
            }
            return;
        }
        self.start_runtime(size, cx);
        if !resize::is_meaningful_delta(self.last_resize_size, size)
            || self.pending_resize_size == Some(size)
        {
            return;
        }
        self.pending_resize_size = Some(size);
        self.pending_resize_updated_at = Some(Instant::now());
    }

    pub(super) fn flush_pending_start(&mut self, cx: &mut Context<Self>) {
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
        self.start_runtime(size, cx);
    }

    fn schedule_initial_start(&mut self, generation: u64, cx: &mut Context<Self>) {
        self._initial_layout_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(RDP_INITIAL_LAYOUT_DEBOUNCE)
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.input_tx.is_some() {
                    return;
                }
                let Some((size, scale_factor, observed_at)) =
                    view.initial_size.take_generation(generation)
                else {
                    tracing::trace!(
                        protocol = view.options.protocol.label(),
                        generation,
                        "ignored superseded remote desktop initial layout"
                    );
                    return;
                };
                let now = Instant::now();
                view.display_scale_factor = scale_factor;
                tracing::info!(
                    protocol = view.options.protocol.label(),
                    generation,
                    width = size.0,
                    height = size.1,
                    scale_factor,
                    layout_stable_ms =
                        now.saturating_duration_since(observed_at).as_millis() as u64,
                    startup_elapsed_ms = now
                        .saturating_duration_since(view.startup_started_at)
                        .as_millis() as u64,
                    "remote desktop initial layout stabilized"
                );
                view.start_runtime(size, cx);
                cx.notify();
            });
        }));
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
