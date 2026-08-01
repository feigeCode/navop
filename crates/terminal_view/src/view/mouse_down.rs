use super::*;

impl TerminalView {
    pub(super) fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let accepts_live_input = self.accepts_live_terminal_input(cx);
        if self.terminal.read(cx).ssh_mfa_request().is_none() {
            window.focus(&self.focus_handle, cx);
        }
        if should_start_block_selection(event.button, event.modifiers) {
            let point = self.pixel_to_point(event.position, self.terminal_bounds, cx);
            self.block_selection = Some(BlockSelection::new(point));
            self.mouse_state.block_selecting = true;
            self.mouse_state.pending_sgr_left_press = None;
            self.mouse_state.selecting = false;
            self.terminal.update(cx, |terminal, _| {
                terminal.clear_selection();
            });
            self.dismiss_history_prompt();
            cx.notify();
            return;
        }
        let mode = self.terminal.read(cx).mode();
        if accepts_live_input && should_defer_sgr_left_press(event.button, event.modifiers, mode) {
            self.mouse_state.pending_sgr_left_press = Some(PendingSgrMousePress {
                point: self.pixel_to_point(event.position, self.terminal_bounds, cx),
                position: event.position,
                modifiers: event.modifiers,
            });
            return;
        }
        // SGR 鼠标模式下把按钮按下事件交给 TUI，跳过 selection/URL/dismiss
        if self.try_report_sgr_mouse_button(event.button, event.position, event.modifiers, true, cx)
        {
            return;
        }
        tracing::debug!(
            target: "terminal.history_prompt",
            reason = "mouse_down",
            button = ?event.button,
            position = ?event.position,
            "terminal mouse down"
        );

        if should_dismiss_history_prompt_for_mouse(event.button) {
            self.dismiss_history_prompt();
        }

        if event.button != MouseButton::Left {
            return;
        }

        let bounds = self.terminal_bounds;
        let cleared_block_selection = self.block_selection.take().is_some();
        self.mouse_state.block_selecting = false;

        let point = self.pixel_to_point(event.position, bounds, cx);
        let has_selection = self.terminal.read(cx).term().lock().selection.is_some();
        if should_extend_selection_on_shift_click(event.button, event.modifiers, has_selection) {
            let side = self.pixel_to_side(event.position, bounds);
            self.terminal.update(cx, |terminal, _| {
                terminal.update_selection(point, side);
            });
            self.mouse_state.selecting = true;
            cx.notify();
            return;
        }

        let screen_line = point.line.0 as usize;
        let column = point.column.0;
        let line_text = self.get_addon_line_text(screen_line, column, cx);
        let is_local =
            self.terminal.read(cx).live_connection_kind() == Some(TerminalConnectionKind::Local);
        let consumed = {
            let mut open_url = |url: &str| cx.open_url(url);
            let mut context = TerminalAddonMouseContext::new(
                line_text.screen_line,
                line_text.column,
                &line_text.text,
                event.modifiers,
                event.position,
                is_local,
                self.local_working_dir.as_deref(),
                &mut open_url,
            );
            self.addon_manager.dispatch_mouse_down(&mut context)
        };

        if consumed {
            if cleared_block_selection {
                cx.notify();
            }
            return;
        }

        let now = std::time::Instant::now();
        let is_double_click = self.mouse_state.last_click_point == Some(point)
            && self
                .mouse_state
                .last_click_time
                .map_or(false, |t| now.duration_since(t).as_millis() < 500);

        if is_double_click {
            self.mouse_state.click_count += 1;
        } else {
            self.mouse_state.click_count = 1;
        }

        self.mouse_state.last_click_point = Some(point);
        self.mouse_state.last_click_time = Some(now);

        let selection_type = match self.mouse_state.click_count {
            1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            _ => SelectionType::Lines,
        };

        self.terminal.update(cx, |terminal, _| {
            terminal.start_selection(
                selection_type,
                point,
                self.pixel_to_side(event.position, bounds),
            );
        });

        self.mouse_state.selecting = true;
        cx.notify();
    }

    pub(super) fn handle_middle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        // SGR 鼠标模式下中键按下走 TUI 报告而不是 middle-click paste
        if self.try_report_sgr_mouse_button(
            MouseButton::Middle,
            event.position,
            event.modifiers,
            true,
            cx,
        ) {
            return;
        }
        if !self.middle_click_paste {
            return;
        }
        if let Some(clipboard) = cx.read_from_clipboard() {
            if let Some(text) = clipboard.text() {
                self.paste_text(&text, window, cx);
            }
        }
    }

    pub(super) fn handle_right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        if !should_direct_paste_on_right_click(self.right_click_paste, event.button) {
            return;
        }
        cx.stop_propagation();
        if self.terminal.read(cx).ssh_mfa_request().is_none() {
            window.focus(&self.focus_handle, cx);
        }
        self.dismiss_history_prompt();
        self.paste(&Paste, window, cx);
    }
}
