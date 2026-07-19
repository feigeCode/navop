use super::*;

impl TerminalView {
    pub(super) fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bounds = self.terminal_bounds;
        self.mouse_position = Some(event.position);
        let point = self.pixel_to_point(event.position, bounds, cx);
        let screen_line = point.line.0 as usize;
        let column = point.column.0;

        if self.mouse_state.block_selecting {
            if event.dragging() {
                if let Some(selection) = &mut self.block_selection {
                    selection.update(point);
                    cx.notify();
                }
            }
            return;
        }

        if !event.dragging() {
            self.mouse_state.pending_sgr_left_press = None;
            self.finish_mouse_selection(cx);
        }

        if event.dragging() {
            self.start_selection_from_pending_sgr_press(point, bounds, cx);
        }

        let line_text = self.get_addon_line_text(screen_line, column, cx);
        let is_local = self.terminal.read(cx).connection_kind() == TerminalConnectionKind::Local;
        let hover_changed = {
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
            self.addon_manager.dispatch_mouse_move(&mut context)
        };
        if hover_changed {
            cx.notify();
        }

        if !self.mouse_state.selecting {
            return;
        }

        if !event.dragging() {
            self.finish_mouse_selection(cx);
            return;
        }

        let point = self.pixel_to_point(event.position, bounds, cx);
        let side = self.pixel_to_side(event.position, bounds);

        self.terminal.update(cx, |terminal, _| {
            terminal.update_selection(point, side);
        });
        cx.notify();
    }

    pub(super) fn start_selection_from_pending_sgr_press(
        &mut self,
        point: AlacPoint,
        bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let should_start = self
            .mouse_state
            .pending_sgr_left_press
            .as_ref()
            .map_or(false, |pending| {
                should_start_selection_from_pending_sgr_press(pending.point, point)
            });
        if !should_start {
            return;
        }

        let pending = self.mouse_state.pending_sgr_left_press.take().unwrap();
        let now = std::time::Instant::now();
        let is_double_click = self.mouse_state.last_click_point == Some(pending.point)
            && self
                .mouse_state
                .last_click_time
                .map_or(false, |t| now.duration_since(t).as_millis() < 500);

        self.mouse_state.click_count = if is_double_click {
            self.mouse_state.click_count + 1
        } else {
            1
        };
        self.mouse_state.last_click_point = Some(pending.point);
        self.mouse_state.last_click_time = Some(now);
        let selection_type = match self.mouse_state.click_count {
            1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            _ => SelectionType::Lines,
        };

        self.terminal.update(cx, |terminal, _| {
            terminal.start_selection(
                selection_type,
                pending.point,
                self.pixel_to_side(pending.position, bounds),
            );
        });
        self.mouse_state.selecting = true;
    }

    pub(super) fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mouse_state.block_selecting && event.button == MouseButton::Left {
            let point = self.pixel_to_point(event.position, self.terminal_bounds, cx);
            if let Some(selection) = &mut self.block_selection {
                selection.update(point);
            }
            self.finish_block_selection(cx);
            return;
        }
        if let Some(pending) = self.mouse_state.pending_sgr_left_press.take() {
            self.terminal.update(cx, |terminal, _| {
                terminal.clear_selection();
            });
            if sgr_mouse_mode_enabled(self.terminal.read(cx).mode()) {
                self.write_sgr_mouse_button_report(
                    MouseButton::Left,
                    pending.position,
                    pending.modifiers,
                    true,
                    cx,
                );
                self.write_sgr_mouse_button_report(
                    MouseButton::Left,
                    event.position,
                    event.modifiers,
                    false,
                    cx,
                );
            }
            return;
        }
        // SGR 鼠标模式下：先回报释放，然后跳过 selection 收尾
        if self.try_report_sgr_mouse_button(
            event.button,
            event.position,
            event.modifiers,
            false,
            cx,
        ) {
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        let bounds = self.terminal_bounds;
        let point = self.pixel_to_point(event.position, bounds, cx);
        let screen_line = point.line.0 as usize;
        let column = point.column.0;
        let line_text = self.get_addon_line_text(screen_line, column, cx);
        let is_local = self.terminal.read(cx).connection_kind() == TerminalConnectionKind::Local;
        {
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
            let _ = self.addon_manager.dispatch_mouse_up(&mut context);
        }
        self.finish_mouse_selection(cx);
    }

    pub(super) fn handle_window_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }

        if self.mouse_state.block_selecting {
            self.finish_block_selection(cx);
            return;
        }

        if self.mouse_state.pending_sgr_left_press.is_some() {
            if !self.terminal_bounds.contains(&event.position) {
                self.handle_mouse_up(event, window, cx);
            }
            return;
        }

        self.finish_mouse_selection(cx);
    }

    pub(super) fn finish_block_selection(&mut self, cx: &mut Context<Self>) {
        if !self.mouse_state.block_selecting {
            return;
        }

        self.mouse_state.block_selecting = false;
        if self
            .block_selection
            .map(|selection| selection.is_empty())
            .unwrap_or(false)
        {
            self.block_selection = None;
            cx.notify();
            return;
        }
        if self.auto_copy_on_select {
            if let Some(text) = self.block_selection_text(cx) {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
        cx.notify();
    }

    pub(super) fn finish_mouse_selection(&mut self, cx: &mut Context<Self>) {
        if !self.mouse_state.selecting {
            return;
        }

        self.mouse_state.selecting = false;
        if self.auto_copy_on_select {
            if let Some(text) = self.terminal.read(cx).selection_text() {
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
        }
        cx.notify();
    }
}
