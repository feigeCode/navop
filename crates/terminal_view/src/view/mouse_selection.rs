use super::*;

impl TerminalView {
    fn try_apply_terminal_selection_action(
        &mut self,
        action: PendingTerminalSelectionAction,
        cx: &mut Context<Self>,
    ) -> bool {
        self.terminal.update(cx, |terminal, _| match action {
            PendingTerminalSelectionAction::Clear => terminal.try_clear_selection(),
            PendingTerminalSelectionAction::Start {
                selection_type,
                point,
                side,
            } => terminal.try_start_selection(selection_type, point, side),
            PendingTerminalSelectionAction::Update { point, side } => {
                terminal.try_update_selection(point, side)
            }
        })
    }

    fn enqueue_terminal_selection_action(&mut self, action: PendingTerminalSelectionAction) {
        match action {
            PendingTerminalSelectionAction::Clear
            | PendingTerminalSelectionAction::Start { .. } => {
                // Clear and Start both replace all earlier selection state.
                // Keeping only the latest prevents an unbounded mouse-move
                // queue while the parser owns the terminal lock.
                self.pending_selection_auto_copy = false;
                self.pending_terminal_selection_actions.clear();
                self.pending_terminal_selection_actions.push_back(action);
            }
            PendingTerminalSelectionAction::Update { .. } => {
                if matches!(
                    self.pending_terminal_selection_actions.back(),
                    Some(PendingTerminalSelectionAction::Update { .. })
                ) {
                    self.pending_terminal_selection_actions.pop_back();
                }
                self.pending_terminal_selection_actions.push_back(action);
            }
        }
    }

    pub(super) fn apply_or_queue_terminal_selection_action(
        &mut self,
        action: PendingTerminalSelectionAction,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            action,
            PendingTerminalSelectionAction::Clear | PendingTerminalSelectionAction::Start { .. }
        ) {
            self.pending_selection_auto_copy = false;
        }
        if self.pending_terminal_selection_actions.is_empty()
            && self.try_apply_terminal_selection_action(action, cx)
        {
            return;
        }

        self.enqueue_terminal_selection_action(action);
        self.schedule_terminal_render_retry(cx);
    }

    pub(super) fn apply_pending_terminal_selection_actions(&mut self, cx: &mut Context<Self>) {
        while let Some(action) = self.pending_terminal_selection_actions.front().copied() {
            if !self.try_apply_terminal_selection_action(action, cx) {
                self.schedule_terminal_render_retry(cx);
                return;
            }
            self.pending_terminal_selection_actions.pop_front();
        }
        self.try_finish_pending_selection_auto_copy(cx);
    }

    fn try_finish_pending_selection_auto_copy(&mut self, cx: &mut Context<Self>) {
        if !self.pending_selection_auto_copy || !self.pending_terminal_selection_actions.is_empty()
        {
            return;
        }

        let term = self.terminal.read(cx).term().clone();
        let Some(term) = term.try_lock_unfair() else {
            self.schedule_terminal_render_retry(cx);
            return;
        };
        let selection_text = term.selection_to_string();
        drop(term);

        self.pending_selection_auto_copy = false;
        if let Some(text) = selection_text.filter(|text| !text.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

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
        let is_local =
            self.terminal.read(cx).live_connection_kind() == Some(TerminalConnectionKind::Local);
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

        self.apply_or_queue_terminal_selection_action(
            PendingTerminalSelectionAction::Update { point, side },
            cx,
        );
        self.update_selection_autoscroll(event.position, cx);
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

        self.apply_or_queue_terminal_selection_action(
            PendingTerminalSelectionAction::Start {
                selection_type,
                point: pending.point,
                side: self.pixel_to_side(pending.position, bounds),
            },
            cx,
        );
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
            if self.accepts_live_terminal_input(cx) {
                self.apply_or_queue_terminal_selection_action(
                    PendingTerminalSelectionAction::Clear,
                    cx,
                );
                if sgr_mouse_mode_enabled(self.terminal_frame_snapshot.mode) {
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
        let is_local =
            self.terminal.read(cx).live_connection_kind() == Some(TerminalConnectionKind::Local);
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
        self.clear_selection_autoscroll();
        if !self.mouse_state.selecting {
            return;
        }

        self.mouse_state.selecting = false;
        if self.auto_copy_on_select {
            self.pending_selection_auto_copy = true;
            self.try_finish_pending_selection_auto_copy(cx);
        }
        cx.notify();
    }
}
