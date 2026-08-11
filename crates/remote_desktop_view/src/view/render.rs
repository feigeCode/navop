use gpui::prelude::FluentBuilder;

use super::*;
use crate::pointer::scale_filled_remote_cursor_bounds;

struct RemoteDesktopCanvasPaint {
    frame: Option<Arc<surface::RemoteDesktopSurface>>,
    cursor: Option<cursor::RemoteCursorPaint>,
}

fn remote_desktop_frame_canvas(frame: RemoteDesktopCanvasPaint) -> impl IntoElement {
    canvas(
        move |_, _, _| frame,
        move |bounds, frame, window, _| {
            paint_remote_frame(bounds, frame.frame, window);
            paint_remote_cursor(bounds, frame.cursor, window);
        },
    )
    .absolute()
    .inset_0()
    .size_full()
    .min_w_0()
    .min_h_0()
    .overflow_hidden()
}

fn paint_remote_frame(
    bounds: Bounds<Pixels>,
    frame: Option<Arc<surface::RemoteDesktopSurface>>,
    window: &mut Window,
) {
    let Some(frame) = frame else {
        return;
    };

    let renderer_resource_generation = window.renderer_resource_generation();
    let uploads = frame.pending_texture_uploads(renderer_resource_generation);
    let diagnostics_enabled = remote_desktop_diagnostics_enabled();
    let upload_started_at = diagnostics_enabled.then(Instant::now);
    let mut uploaded_count = 0;
    let mut uploaded_bytes = 0usize;
    let mut uploaded_pixels = 0u64;
    let mut largest_upload_pixels = 0u64;
    for upload in uploads.iter() {
        let update_bounds = Bounds::new(
            point(
                DevicePixels(i32::from(upload.rect.x)),
                DevicePixels(i32::from(upload.rect.y)),
            ),
            size(
                DevicePixels(i32::from(upload.rect.width)),
                DevicePixels(i32::from(upload.rect.height)),
            ),
        );
        match window.update_dynamic_texture(
            frame.texture().as_ref(),
            update_bounds,
            upload.bytes.as_slice(),
        ) {
            Ok(()) => {
                uploaded_count += 1;
                if diagnostics_enabled {
                    let pixels =
                        u64::from(upload.rect.width).saturating_mul(u64::from(upload.rect.height));
                    uploaded_bytes = uploaded_bytes.saturating_add(upload.bytes.len());
                    uploaded_pixels = uploaded_pixels.saturating_add(pixels);
                    largest_upload_pixels = largest_upload_pixels.max(pixels);
                }
            }
            Err(error) => {
                tracing::warn!(?error, "failed to update remote desktop texture");
                break;
            }
        }
    }
    if uploaded_count > 0 {
        frame.acknowledge_texture_uploads(&uploads[..uploaded_count]);
    }
    if let Some(upload_started_at) = upload_started_at
        && uploaded_count > 0
    {
        let framebuffer_pixels = u64::from(frame.width()).saturating_mul(u64::from(frame.height()));
        let ratio_per_mille = uploaded_pixels
            .saturating_mul(1000)
            .checked_div(framebuffer_pixels)
            .unwrap_or_default();
        let largest_ratio_per_mille = largest_upload_pixels
            .saturating_mul(1000)
            .checked_div(framebuffer_pixels)
            .unwrap_or_default();
        tracing::info!(
            surface_width = frame.width(),
            surface_height = frame.height(),
            upload_count = uploaded_count,
            upload_bytes = uploaded_bytes,
            upload_pixels = uploaded_pixels,
            upload_ratio_per_mille = ratio_per_mille,
            largest_upload_ratio_per_mille = largest_ratio_per_mille,
            upload_us = upload_started_at.elapsed().as_micros() as u64,
            "remote desktop dynamic texture uploads"
        );
    }

    if let Err(error) =
        window.paint_dynamic_texture(bounds, Corners::default(), frame.texture().clone(), false)
    {
        tracing::warn!(?error, "failed to paint remote desktop texture");
    }
}

fn paint_remote_cursor(
    bounds: Bounds<Pixels>,
    cursor: Option<cursor::RemoteCursorPaint>,
    window: &mut Window,
) {
    let Some(cursor) = cursor else {
        return;
    };
    let Some(bounds) = remote_cursor_bounds(bounds, cursor.geometry) else {
        return;
    };
    if let Err(error) = window.paint_image(bounds, Corners::default(), cursor.image, 0, false) {
        tracing::warn!(?error, "failed to paint remote desktop cursor");
    }
}

fn remote_cursor_bounds(
    bounds: Bounds<Pixels>,
    geometry: crate::pointer::RemoteCursorGeometry,
) -> Option<Bounds<Pixels>> {
    let local = scale_filled_remote_cursor_bounds(
        LocalBounds {
            left: bounds.left().into(),
            top: bounds.top().into(),
            width: bounds.size.width.into(),
            height: bounds.size.height.into(),
        },
        geometry,
    )?;
    Some(Bounds::new(
        point(px(local.left), px(local.top)),
        size(px(local.width), px(local.height)),
    ))
}

impl Focusable for RemoteDesktopView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for RemoteDesktopView {}

impl TabContent for RemoteDesktopView {
    fn content_key(&self) -> &'static str {
        "RemoteDesktop"
    }

    fn title(&self, _cx: &App) -> SharedString {
        SharedString::from(remote_desktop_tab_title(&self.title, self.tab_index))
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(match self.options.protocol {
            RemoteDesktopProtocol::Rdp => IconName::Rdp.color(),
            RemoteDesktopProtocol::Vnc => IconName::Vnc.color(),
        })
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Task<bool> {
        self.cursor.reset_session();
        close_runtime_once(&mut self.input_tx);
        self.output_rx.take();
        self.presentation_tx.take();
        self.presentation_queue.clear();
        self.presentation_in_flight = false;
        self.reset_presentation_pacing();
        self._initial_layout_task.take();
        self._output_ready_task.take();
        self._presentation_task.take();
        Task::ready(true)
    }
}

impl Render for RemoteDesktopView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Retired textures stay queued until no presentation state or rendered
        // frame owns them. This also retries retirement after an asynchronous
        // session reset releases its last surface.
        for texture in self.retired_textures.take_releasable() {
            if let Err(error) = window.drop_dynamic_texture(texture) {
                tracing::warn!(?error, "failed to release remote desktop texture");
            }
        }
        for cursor in self.cursor.take_pending_images() {
            if let Err(error) = window.drop_image(cursor) {
                tracing::warn!(?error, "failed to release remote desktop cursor");
            }
        }
        self.drain_output(window, cx);
        self.sync_local_clipboard(window, cx);
        self.flush_pending_start(cx);
        self.flush_pending_resize();
        if let Some(latest_frame) = self.latest_frame.take() {
            let frame_presented = self.rendered_frames.current() != Some(&latest_frame);
            if let Some(retired) = self.rendered_frames.promote(latest_frame) {
                self.retired_textures.retire(retired);
            }
            if frame_presented {
                let snapshot = self.frame_sync.snapshot();
                tracing::trace!(
                    protocol = self.options.protocol.label(),
                    session_generation = snapshot.session_generation,
                    frame_presented = 1,
                    full_frames = snapshot.full_frames,
                    deltas = snapshot.deltas,
                    "remote desktop frame presented"
                );
            }
        }
        if let Some(retired) = self.cursor.promote_latest()
            && let Err(error) = window.drop_image(retired)
        {
            tracing::warn!(?error, "failed to retire remote desktop cursor");
        }
        let rendered_frame = self.rendered_frames.current().cloned();
        let show_empty_status = rendered_frame.is_none();
        let canvas_paint = RemoteDesktopCanvasPaint {
            frame: rendered_frame,
            cursor: self.cursor.paint_state(self.remote_size),
        };
        let view = cx.entity();

        let content = div()
            .id("remote-desktop-content")
            .size_full()
            .min_w_0()
            .min_h_0()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .key_context(REMOTE_DESKTOP_CONTEXT)
            .on_action(cx.listener(Self::send_tab))
            .on_action(cx.listener(Self::send_shift_tab))
            .on_action(cx.listener(Self::remote_copy))
            .on_action(cx.listener(Self::remote_paste))
            .capture_key_down(cx.listener(Self::handle_key_down))
            .capture_key_up(cx.listener(Self::handle_key_up))
            .on_modifiers_changed(cx.listener(Self::handle_modifiers_changed))
            .on_hover(cx.listener(|this, hovered, _, _| {
                this.cursor.set_pointer_hovered(*hovered);
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.send_pointer_move(event.position, window, cx);
                cx.stop_propagation();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.send_pointer_move(event.position, window, cx);
                    this.send_mouse_button(event.button, true);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.send_pointer_move(event.position, window, cx);
                    this.send_mouse_button(event.button, true);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.send_pointer_move(event.position, window, cx);
                    this.send_mouse_button(event.button, true);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.send_pointer_move(event.position, window, cx);
                    this.send_mouse_button(event.button, false);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.send_pointer_move(event.position, window, cx);
                    this.send_mouse_button(event.button, false);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.send_pointer_move(event.position, window, cx);
                    this.send_mouse_button(event.button, false);
                    cx.stop_propagation();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                this.send_scroll(event);
                cx.stop_propagation();
            }))
            .child(remote_desktop_frame_canvas(canvas_paint))
            .when(show_empty_status, |this| {
                this.child(
                    div()
                        .min_w_0()
                        .max_w_full()
                        .flex_shrink_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_center()
                        .px_4()
                        .py_2()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.status.clone()),
                )
            });

        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .on_children_prepainted(move |bounds, window, cx| {
                if let Some(bounds) = bounds.first().copied() {
                    view.update(cx, |view, cx| {
                        view.update_content_bounds(bounds, window.scale_factor(), cx);
                    });
                }
            })
            .child(content)
    }
}
