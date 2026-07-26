use gpui::prelude::FluentBuilder;

use super::*;

fn remote_desktop_frame_canvas(
    frame: Option<Arc<RenderImage>>,
    focus_handle: FocusHandle,
) -> impl IntoElement {
    canvas(
        move |_, _, _| frame,
        move |bounds, frame, window, cx| {
            window.handle_input(&focus_handle, RemoteDesktopImeGuard::new(bounds), cx);
            if let Some(frame) = frame
                && let Err(error) = window.paint_image(bounds, Corners::default(), frame, 0, false)
            {
                tracing::warn!(?error, "failed to paint remote desktop frame");
            }
        },
    )
    .absolute()
    .inset_0()
    .size_full()
    .min_w_0()
    .min_h_0()
    .overflow_hidden()
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
        let rendered_frame = self.rendered_frames.current().cloned();
        let show_empty_status = rendered_frame.is_none();
        let view = cx.entity();
        let focus_handle = self.focus_handle.clone();

        let content = div()
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
            .child(remote_desktop_frame_canvas(rendered_frame, focus_handle))
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
