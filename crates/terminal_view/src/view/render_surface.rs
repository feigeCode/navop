use super::*;

struct TerminalViewportState {
    font_family: SharedString,
    connection_state: ConnectionState,
    can_reconnect: bool,
    has_pending_host_key_verification: bool,
    has_ssh_credential_request: bool,
    has_ssh_mfa_request: bool,
    has_selection: bool,
    selection_text: Option<String>,
    accepts_live_input: bool,
    right_click_paste: bool,
    show_scrollbar: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConnectionStatusPresentation {
    Banner,
    Dialog,
}

pub(super) fn connection_status_presentation(
    connection_state: &ConnectionState,
    has_pending_host_key_verification: bool,
    has_ssh_credential_request: bool,
    has_ssh_mfa_request: bool,
) -> Option<ConnectionStatusPresentation> {
    if has_pending_host_key_verification || matches!(connection_state, ConnectionState::Connected) {
        return None;
    }
    if has_ssh_credential_request || has_ssh_mfa_request {
        Some(ConnectionStatusPresentation::Dialog)
    } else {
        Some(ConnectionStatusPresentation::Banner)
    }
}

pub(super) fn terminal_viewport_bounds(
    surface_bounds: Bounds<Pixels>,
    content_mask_bounds: Bounds<Pixels>,
) -> Bounds<Pixels> {
    let intersection = surface_bounds.intersect(&content_mask_bounds);
    if intersection.size.width <= px(0.0) || intersection.size.height <= px(0.0) {
        return Bounds::new(surface_bounds.origin, size(px(0.0), px(0.0)));
    }

    Bounds::new(
        surface_bounds.origin,
        size(
            (intersection.right() - surface_bounds.origin.x).max(px(0.0)),
            (intersection.bottom() - surface_bounds.origin.y).max(px(0.0)),
        ),
    )
}

#[cfg(test)]
pub(super) fn should_show_connection_overlay(
    connection_state: &ConnectionState,
    has_pending_host_key_verification: bool,
) -> bool {
    connection_status_presentation(
        connection_state,
        has_pending_host_key_verification,
        false,
        false,
    )
    .is_some()
}

impl TerminalView {
    pub(super) fn render_terminal_viewport(
        &mut self,
        font_family: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.terminal_viewport_state(font_family, cx);
        let show_scrollbar = state.show_scrollbar;
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(self.render_terminal_core(state, cx))
            .when(show_scrollbar, |this| {
                this.child(self.render_terminal_scrollbar())
            })
            .into_any_element()
    }

    fn terminal_viewport_state(
        &self,
        font_family: SharedString,
        cx: &App,
    ) -> TerminalViewportState {
        let accepts_live_input = self.accepts_live_terminal_input(cx);
        let terminal = self.terminal.read(cx);
        let connection_state = terminal.connection_state().clone();
        let can_reconnect = terminal.can_reconnect();
        let selection_text = self
            .terminal_frame_snapshot
            .block_selection_text
            .clone()
            .or_else(|| self.terminal_frame_snapshot.selection_text.clone());
        let has_selection =
            self.terminal_frame_snapshot.selection_present || selection_text.is_some();
        let terminal_mode = self.terminal_frame_snapshot.mode;
        let history_size = self.terminal_frame_snapshot.history_size;
        TerminalViewportState {
            font_family,
            connection_state,
            can_reconnect,
            has_pending_host_key_verification: terminal.host_key_verification_request().is_some(),
            has_ssh_credential_request: terminal.ssh_credential_request().is_some(),
            has_ssh_mfa_request: terminal.ssh_mfa_request().is_some(),
            has_selection,
            selection_text,
            accepts_live_input,
            right_click_paste: self.right_click_paste && accepts_live_input,
            show_scrollbar: (!accepts_live_input || !terminal_mode.contains(TermMode::ALT_SCREEN))
                && history_size > 0,
        }
    }

    fn render_terminal_core(
        &mut self,
        state: TerminalViewportState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let focus_handle = self.focus_handle.clone();
        let connection_status = connection_status_presentation(
            &state.connection_state,
            state.has_pending_host_key_verification,
            state.has_ssh_credential_request,
            state.has_ssh_mfa_request,
        );
        div()
            .track_focus(&focus_handle)
            .key_context(TERMINAL_CONTEXT)
            .on_action(cx.listener(Self::send_tab))
            .on_action(cx.listener(Self::send_shift_tab))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::clear_screen))
            .on_action(cx.listener(Self::clear_selection))
            .on_action(cx.listener(Self::search_forward))
            .on_action(cx.listener(Self::search_backward))
            .on_action(cx.listener(Self::toggle_vi_mode))
            .on_action(cx.listener(Self::increase_font))
            .on_action(cx.listener(Self::decrease_font))
            .on_action(cx.listener(Self::reset_font))
            .on_key_down(cx.listener(Self::handle_key_event))
            .flex_1()
            .relative()
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(Self::handle_scroll))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(Self::handle_middle_mouse_down),
            )
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .child(self.render_input_canvas(cx))
            .child(self.render_terminal_surface(&state, cx))
            .when_some(self.render_addon_tooltip(), |this, tooltip| {
                this.child(tooltip)
            })
            .when(
                connection_status == Some(ConnectionStatusPresentation::Banner),
                |this| this.child(self.render_connection_banner(state.can_reconnect, cx)),
            )
            .when(
                connection_status == Some(ConnectionStatusPresentation::Dialog),
                |this| this.child(self.render_connection_dialog(cx)),
            )
            .into_any_element()
    }

    fn render_input_canvas(&self, cx: &mut Context<Self>) -> AnyElement {
        let focus_handle = self.focus_handle.clone();
        canvas(|_bounds, _window, _cx| {}, {
            let entity = cx.entity().downgrade();
            let focus_handle = focus_handle.clone();
            move |bounds, _state, window, cx| {
                if let Some(entity) = entity.upgrade() {
                    let input_handler = ElementInputHandler::new(bounds, entity);
                    window.handle_input(&focus_handle, input_handler, cx);
                }
            }
        })
        .absolute()
        .left(px(12.0))
        .right(px(12.0))
        .top(px(12.0))
        .bottom(px(12.0))
        .into_any_element()
    }

    fn render_terminal_surface(
        &mut self,
        state: &TerminalViewportState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity().clone();
        let bounds_view = cx.entity().downgrade();
        let sidebar = self.sidebar.clone();
        let terminal_surface = div()
            .absolute()
            .left(px(12.0))
            .right(px(12.0))
            .top(px(12.0))
            .bottom(px(12.0))
            .bg(self.current_theme.background)
            .overflow_hidden()
            .on_prepaint(move |bounds, window, cx| {
                let viewport_bounds =
                    terminal_viewport_bounds(bounds, window.content_mask().bounds);
                let _ = bounds_view.update(cx, |this, cx| {
                    this.terminal_bounds = viewport_bounds;
                    let mut metrics = this.scrollbar_metrics.borrow_mut();
                    metrics.viewport_size = viewport_bounds.size;
                    metrics.line_height = this.line_height;
                    metrics.cell_width = this.cell_width;
                    drop(metrics);
                    this.resize_if_needed(viewport_bounds, cx);
                });
            })
            .child(self.render_terminal(state.font_family.clone(), cx))
            .when_some(self.render_history_prompt_overlay(cx), |this, overlay| {
                this.child(overlay)
            });
        if state.right_click_paste {
            return terminal_surface
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(Self::handle_right_mouse_down),
                )
                .into_any_element();
        }
        let has_selection = state.has_selection;
        let selection_text = state.selection_text.clone();
        let accepts_live_input = state.accepts_live_input;
        terminal_surface
            .context_menu(move |menu, window, cx| {
                Self::build_context_menu(
                    menu,
                    has_selection,
                    selection_text.clone(),
                    accepts_live_input,
                    &view,
                    &sidebar,
                    window,
                    cx,
                )
            })
            .into_any_element()
    }

    fn render_addon_tooltip(&self) -> Option<AnyElement> {
        let (tooltip, position) = self.addon_manager.tooltip().zip(self.mouse_position)?;
        let colors = self.current_theme.colors();
        let relative_x = position.x - self.terminal_bounds.origin.x;
        let relative_y = position.y - self.terminal_bounds.origin.y;
        Some(
            div()
                .absolute()
                .left(relative_x + px(10.0))
                .top(relative_y + px(20.0))
                .px_2()
                .py_1()
                .bg(colors.muted)
                .rounded_md()
                .shadow_md()
                .text_size(px(11.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .px_1()
                                .bg(colors.background)
                                .rounded_sm()
                                .text_color(colors.foreground)
                                .child(tooltip.action_hint),
                        )
                        .child(
                            div()
                                .text_color(colors.muted_foreground)
                                .child(tooltip.action_text),
                        ),
                )
                .child(
                    div()
                        .text_color(tooltip.display_color)
                        .overflow_hidden()
                        .max_w(px(400.0))
                        .text_ellipsis()
                        .child(tooltip.display_text),
                )
                .into_any_element(),
        )
    }

    fn render_terminal_scrollbar(&self) -> AnyElement {
        div()
            .absolute()
            .top(px(12.0))
            .right(px(4.0))
            .bottom(px(12.0))
            .w(px(12.0))
            .child(
                Scrollbar::vertical(&self.scrollbar_handle).scrollbar_show(ScrollbarShow::Always),
            )
            .into_any_element()
    }
}
