use super::*;

/// 翻页步长：保留一行上下文，与 xterm / Windows Terminal 的 Shift+PageUp 一致。
pub(super) fn terminal_page_scroll_lines(screen_lines: usize) -> i32 {
    screen_lines.saturating_sub(1).max(1) as i32
}

impl TerminalView {
    pub(super) fn scroll_line_up(
        &mut self,
        _: &ScrollLineUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scroll_history_by(1, cx);
    }

    pub(super) fn scroll_line_down(
        &mut self,
        _: &ScrollLineDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scroll_history_by(-1, cx);
    }

    pub(super) fn scroll_page_up(
        &mut self,
        _: &ScrollPageUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = terminal_page_scroll_lines(self.terminal_frame_snapshot.screen_lines);
        self.scroll_history_by(page, cx);
    }

    pub(super) fn scroll_page_down(
        &mut self,
        _: &ScrollPageDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = terminal_page_scroll_lines(self.terminal_frame_snapshot.screen_lines);
        self.scroll_history_by(-page, cx);
    }

    /// 按行数滚动回滚历史，正数向历史方向（上）。语义与鼠标滚轮一致：
    /// VI 模式移动 vi 光标，其余模式移动显示偏移。
    fn scroll_history_by(&mut self, lines: i32, cx: &mut Context<Self>) {
        let accepts_live_input = self.accepts_live_terminal_input(cx);
        let mode = self.terminal_frame_snapshot.mode;
        if accepts_live_input && mode.contains(TermMode::ALT_SCREEN) {
            // alt-screen 没有回滚历史，按键应继续交给 vim/less 等 TUI 程序。
            cx.propagate();
            return;
        }

        self.dismiss_history_prompt();
        if accepts_live_input && mode.contains(TermMode::VI) {
            if self.pending_vi_scroll_lines != 0 {
                self.pending_vi_scroll_lines = self.pending_vi_scroll_lines.saturating_add(lines);
                self.schedule_terminal_render_retry(cx);
                return;
            }
            if !self.try_scroll_vi_mode(lines, cx) {
                self.pending_vi_scroll_lines = lines;
                self.schedule_terminal_render_retry(cx);
                return;
            }
        } else if !self.scrollbar_handle.try_scroll_display_delta(lines) {
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
        cx.notify();
    }
}
