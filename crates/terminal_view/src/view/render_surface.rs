use super::*;

struct TerminalViewportState {
    font_family: SharedString,
    connection_state: ConnectionState,
    can_reconnect: bool,
    has_selection: bool,
    selection_text: Option<String>,
    accepts_live_input: bool,
    right_click_paste: bool,
    show_scrollbar: bool,
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
        let block_selection_text = self.block_selection_text(cx);
        let accepts_live_input = self.accepts_live_terminal_input(cx);
        let terminal = self.terminal.read(cx);
        let connection_state = terminal.connection_state().clone();
        let can_reconnect = terminal.can_reconnect();
        let terminal_has_selection = terminal.term().lock().selection.is_some();
        let has_selection = terminal_has_selection || block_selection_text.is_some();
        let selection_text = block_selection_text.or_else(|| terminal.selection_text());
        let terminal_mode = terminal.mode();
        let history_size = terminal.term().lock().history_size();
        TerminalViewportState {
            font_family,
            connection_state,
            can_reconnect,
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
        let disconnected = matches!(
            &state.connection_state,
            ConnectionState::Disconnected { .. } | ConnectionState::Connecting
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
            .when(disconnected, |this| {
                this.child(self.render_connection_overlay(state.can_reconnect, cx))
            })
            .into_any_element()
    }

    fn render_input_canvas(&self, cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity().downgrade();
        let focus_handle = self.focus_handle.clone();
        canvas(
            move |bounds, _window, cx| {
                if let Some(entity) = entity.upgrade() {
                    entity.update(cx, |this, cx| {
                        this.terminal_bounds = bounds;
                        let mut metrics = this.scrollbar_metrics.borrow_mut();
                        metrics.viewport_size = bounds.size;
                        metrics.line_height = this.line_height;
                        metrics.cell_width = this.cell_width;
                        drop(metrics);
                        this.resize_if_needed(bounds, cx);
                    });
                }
            },
            {
                let entity = cx.entity().downgrade();
                let focus_handle = focus_handle.clone();
                move |bounds, _state, window, cx| {
                    if let Some(entity) = entity.upgrade() {
                        let input_handler = ElementInputHandler::new(bounds, entity);
                        window.handle_input(&focus_handle, input_handler, cx);
                    }
                }
            },
        )
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
        let sidebar = self.sidebar.clone();
        let terminal_surface = div()
            .absolute()
            .left(px(12.0))
            .right(px(12.0))
            .top(px(12.0))
            .bottom(px(12.0))
            .bg(self.current_theme.background)
            .overflow_hidden()
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
