use gpui::prelude::FluentBuilder;

use super::*;
use crate::pointer::scale_filled_remote_cursor_bounds;

struct RemoteDesktopCanvasPaint {
    frame: Option<Arc<RenderImage>>,
    cursor: Option<cursor::RemoteCursorPaint>,
}

fn remote_desktop_frame_canvas(
    frame: RemoteDesktopCanvasPaint,
    focus_handle: FocusHandle,
) -> impl IntoElement {
    canvas(
        move |_, _, _| frame,
        move |bounds, frame, window, cx| {
            window.handle_input(&focus_handle, RemoteDesktopImeGuard::new(bounds), cx);
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
    frame: Option<Arc<RenderImage>>,
    window: &mut Window,
) {
    let Some(frame) = frame else {
        return;
    };
    if let Err(error) = window.paint_image(bounds, Corners::default(), frame, 0, false) {
        tracing::warn!(?error, "failed to paint remote desktop frame");
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
        Task::ready(true)
    }
}

impl Render for RemoteDesktopView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Only release frames retired by the previous render. A reconnect can
        // reset the session while draining output below; delaying those drops
        // until the next render keeps the image alive until the scene that
        // referenced it has been replaced.
        for frame in self.pending_frame_drops.drain(..) {
            if let Err(error) = window.drop_image(frame) {
                tracing::warn!(?error, "failed to release remote desktop frame");
            }
        }
        for cursor in self.cursor.take_pending_images() {
            if let Err(error) = window.drop_image(cursor) {
                tracing::warn!(?error, "failed to release remote desktop cursor");
            }
        }
        self.drain_output(window, cx);
        self.sync_local_clipboard(window, cx);
        self.flush_pending_start();
        self.flush_pending_resize();
        if let Some(latest_frame) = self.latest_frame.clone()
            && let Some(retired) = self.rendered_frames.promote(latest_frame)
            && let Err(error) = window.drop_image(retired)
        {
            tracing::warn!(?error, "failed to retire remote desktop frame");
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
        let focus_handle = self.focus_handle.clone();

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
                this.send_pointer_move(event.position, window);
                cx.stop_propagation();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.send_pointer_move(event.position, window);
                    this.send_mouse_button(event.button, true);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.send_pointer_move(event.position, window);
                    this.send_mouse_button(event.button, true);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.send_pointer_move(event.position, window);
                    this.send_mouse_button(event.button, true);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.send_pointer_move(event.position, window);
                    this.send_mouse_button(event.button, false);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.send_pointer_move(event.position, window);
                    this.send_mouse_button(event.button, false);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.send_pointer_move(event.position, window);
                    this.send_mouse_button(event.button, false);
                    cx.stop_propagation();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                this.send_scroll(event);
                cx.stop_propagation();
            }))
            .child(remote_desktop_frame_canvas(canvas_paint, focus_handle))
            .when(show_empty_status, |this| {
                this.child(
                    div()
                        .min_w_0()
                        .max_w_full()
                        .overflow_hidden()
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
                    view.update(cx, |view, _| {
                        view.update_content_bounds(bounds, window.scale_factor());
                    });
                }
            })
            .child(content)
    }
}
