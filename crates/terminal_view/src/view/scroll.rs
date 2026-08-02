use super::*;

impl TerminalView {
    fn try_scroll_vi_mode(&self, lines: i32, cx: &Context<Self>) -> bool {
        let term = self.terminal.read(cx).term().clone();
        let Some(mut term) = term.try_lock_unfair() else {
            return false;
        };
        if !term.mode().contains(TermMode::VI) {
            return true;
        }

        // 沿用 Alacritty `ViModeCursor::scroll` 的符号语义，直接传入离散后的行数
        let vi_cursor = term.vi_mode_cursor.scroll(&term, lines);
        term.vi_goto_point(vi_cursor.point);

        let display_offset = term.grid().display_offset();
        let cursor_line = vi_cursor.point.line.0;
        let screen_lines = term.screen_lines() as i32;

        if cursor_line < -(display_offset as i32) {
            let delta = cursor_line + display_offset as i32;
            term.scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
        } else if cursor_line >= screen_lines - (display_offset as i32) {
            let delta = cursor_line - screen_lines + 1 + display_offset as i32;
            term.scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
        }
        true
    }

    pub(super) fn apply_pending_vi_scroll(&mut self, cx: &mut Context<Self>) {
        let lines = std::mem::take(&mut self.pending_vi_scroll_lines);
        if lines == 0 {
            return;
        }
        if !self.try_scroll_vi_mode(lines, cx) {
            self.pending_vi_scroll_lines = self.pending_vi_scroll_lines.saturating_add(lines);
            self.schedule_terminal_render_retry(cx);
        }
    }

    pub(super) fn handle_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta_pixels = event.delta.pixel_delta(self.line_height);
        let delta_lines = delta_pixels.y / self.line_height;
        self.scroll_lines_accumulated += delta_lines;

        let accepts_live_input = self.accepts_live_terminal_input(cx);
        let mode = self.terminal_frame_snapshot.mode;
        let lines = take_whole_scroll_lines(&mut self.scroll_lines_accumulated);
        tracing::debug!(
            target: "terminal.history_prompt",
            reason = "scroll_event",
            lines,
            shell_mode = ?mode,
            "terminal scroll event"
        );

        if should_dismiss_history_prompt_for_scroll(lines) {
            self.dismiss_history_prompt();
        }

        if accepts_live_input && mode.contains(TermMode::ALT_SCREEN) {
            if sgr_mouse_mode_enabled(mode) {
                let point = self.pixel_to_point(event.position, self.terminal_bounds, cx);
                if let Some(report) =
                    sgr_mouse_wheel_report(lines, point.column.0, point.line.0 as usize)
                {
                    for _ in 0..lines.unsigned_abs() {
                        self.write_to_pty(report.as_bytes().to_vec(), cx);
                    }
                }
            } else if self.vim_scroll_to_arrow_keys && lines != 0 {
                // alt-screen TUI(vim/less/man 等)未启用鼠标报告:
                // 把滚轮转为方向键发给 PTY,既能滚动又不会触发 vim 的 VISUAL 选区
                let seq: &[u8] = if mode.contains(TermMode::APP_CURSOR) {
                    if lines > 0 { b"\x1bOA" } else { b"\x1bOB" }
                } else if lines > 0 {
                    b"\x1b[A"
                } else {
                    b"\x1b[B"
                };
                for _ in 0..lines.unsigned_abs() {
                    self.write_to_pty(seq.to_vec(), cx);
                }
            }
            return;
        }

        if lines != 0 {
            if accepts_live_input && mode.contains(TermMode::VI) {
                if self.pending_vi_scroll_lines != 0 {
                    self.pending_vi_scroll_lines =
                        self.pending_vi_scroll_lines.saturating_add(lines);
                    self.schedule_terminal_render_retry(cx);
                    return;
                }
                if !self.try_scroll_vi_mode(lines, cx) {
                    self.pending_vi_scroll_lines = lines;
                    self.schedule_terminal_render_retry(cx);
                    return;
                }
            } else {
                // 沿用终端 display scroll 的符号语义，直接传入离散后的行数
                if !self.scrollbar_handle.try_scroll_display_delta(lines) {
                    let base = self
                        .scrollbar_handle
                        .future_display_offset()
                        .unwrap_or(self.terminal_frame_snapshot.display_offset);
                    let target = (base as i64 + lines as i64)
                        .clamp(0, self.terminal_frame_snapshot.history_size as i64)
                        as usize;
                    self.scrollbar_handle.put_back_future_display_offset(target);
                    self.schedule_terminal_render_retry(cx);
                    return;
                }
            }
            cx.notify();
        }
    }

    pub(super) fn pixel_to_point(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _cx: &Context<Self>,
    ) -> AlacPoint {
        let relative_x = position.x - bounds.origin.x;
        let relative_y = position.y - bounds.origin.y;

        let col = (relative_x / self.cell_width).floor().max(0.0) as usize;
        let line = (relative_y / self.line_height).floor().max(0.0) as i32;

        let col = col.min(self.terminal_frame_snapshot.columns.saturating_sub(1));
        let line = line.clamp(
            0,
            self.terminal_frame_snapshot.screen_lines.saturating_sub(1) as i32,
        );

        AlacPoint::new(Line(line), Column(col))
    }

    /// 根据鼠标在单元格内的位置计算 Side
    pub(super) fn pixel_to_side(&self, position: Point<Pixels>, bounds: Bounds<Pixels>) -> Side {
        let relative_x = position.x - bounds.origin.x;
        let col_f = (relative_x / self.cell_width).max(0.0);
        let cell_offset = col_f.fract();
        if cell_offset < 0.5 {
            Side::Left
        } else {
            Side::Right
        }
    }

    /// 当终端启用 SGR 鼠标 + 任意鼠标报告模式时，把按钮按下/释放事件以 SGR 形式
    /// 回报给 PTY。返回 true 表示已经处理，调用方应跳过 selection/dismiss/paste 等本地行为。
    ///
    /// 特殊穿透:Shift+Left 永远走终端自身的文本选区,不向 TUI 转发 —— 这是 xterm/iTerm/
    /// kitty/wezterm 等的通用约定,让用户在 vim/tmux 等捕获鼠标的应用里仍能复制文本。
    /// 同理 mouse_up 时,如果当前正在终端选区(由 shift+drag 启动),也跳过 release 回报,
    /// 避免在 release 阶段 shift 已松开就把 release 事件错发给 TUI、丢掉 selection 收尾。
    pub(super) fn try_report_sgr_mouse_button(
        &mut self,
        button: MouseButton,
        position: Point<Pixels>,
        modifiers: Modifiers,
        pressed: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.accepts_live_terminal_input(cx) {
            return false;
        }
        if button == MouseButton::Left
            && (modifiers.shift || (!pressed && self.mouse_state.selecting))
        {
            return false;
        }
        let mode = self.terminal_frame_snapshot.mode;
        if !sgr_mouse_mode_enabled(mode) {
            return false;
        }
        self.write_sgr_mouse_button_report(button, position, modifiers, pressed, cx)
    }

    pub(super) fn write_sgr_mouse_button_report(
        &mut self,
        button: MouseButton,
        position: Point<Pixels>,
        modifiers: Modifiers,
        pressed: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.accepts_live_terminal_input(cx) {
            return false;
        }
        let Some(base) = mouse_button_code(button) else {
            return false;
        };
        let point = self.pixel_to_point(position, self.terminal_bounds, cx);
        let encoded = base | encode_mouse_modifiers(modifiers);
        let report =
            sgr_mouse_button_report(encoded, point.column.0, point.line.0 as usize, pressed);
        self.write_to_pty(report.into_bytes(), cx);
        true
    }
}
