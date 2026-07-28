use super::*;

impl TerminalView {
    pub(super) fn copy(&mut self, _: &Copy, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.block_selection_text(cx) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        } else if let Some(text) = self.terminal.read(cx).selection_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        self.focus_terminal(window, cx);
    }

    pub(super) fn block_selection_text(&self, cx: &App) -> Option<String> {
        let selection = self.block_selection?;
        if selection.is_empty() {
            return None;
        }

        let terminal = self.terminal.read(cx);
        let term = terminal.term().lock();
        let columns = term.columns();
        let screen_lines = term.screen_lines();
        let content = term.renderable_content();
        let display_offset = content.display_offset;
        let mut rows = vec![vec![' '; columns]; screen_lines];

        for cell in content.display_iter {
            let screen_line = cell.point.line.0 + display_offset as i32;
            let Ok(row) = usize::try_from(screen_line) else {
                continue;
            };
            if row >= rows.len() || cell.point.column.0 >= columns {
                continue;
            }
            rows[row][cell.point.column.0] = cell.c;
        }

        let rows = rows
            .into_iter()
            .map(|chars| chars.into_iter().collect::<String>())
            .collect::<Vec<_>>();
        block_selection_text_from_rows(&rows, selection.anchor, selection.active)
    }

    pub(super) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        if let Some(clipboard) = cx.read_from_clipboard() {
            let (live_connection_kind, mode) = {
                let terminal = self.terminal.read(cx);
                (terminal.live_connection_kind(), terminal.mode())
            };
            let Some(connection_kind) = live_connection_kind else {
                return;
            };
            let should_upload_image = should_upload_clipboard_image_to_remote_cli(
                self.paste_image_upload,
                connection_kind,
                mode,
            );

            if should_upload_image {
                if let Some(image) = clipboard_image_from_item(&clipboard) {
                    self.paste_clipboard_image_to_remote_cli(image, window, cx);
                    return;
                }
            }
            if let Some(text) = clipboard.text() {
                self.paste_text(&text, window, cx);
            }
        }
    }

    pub(super) fn increase_font(
        &mut self,
        _: &IncreaseFont,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.increase_font_size(cx);
        self.sync_sidebar_theme(window, cx);
    }

    pub(super) fn decrease_font(
        &mut self,
        _: &DecreaseFont,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.decrease_font_size(cx);
        self.sync_sidebar_theme(window, cx);
    }

    pub(super) fn reset_font(
        &mut self,
        _: &ResetFont,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset_font_size(cx);
        self.sync_sidebar_theme(window, cx);
        window.push_notification(
            Notification::info(t!("TerminalView.font_reset_triggered").to_string()).autohide(true),
            cx,
        );
    }

    /// 粘贴文本到终端
    ///
    /// 统一使用 bracketed paste 模式处理所有粘贴操作，确保：
    /// 1. 多行文本不会被立即执行（每一行都需要用户确认）
    /// 2. 保持文本的完整性，让用户可以检查后再执行
    /// 3. 避免意外执行危险命令
    pub(super) fn paste_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        let text = normalize_paste_line_endings(text);
        let text = text.as_ref();
        let mode = self.terminal.read(cx).mode();

        // ALT_SCREEN（如 Vim、less）属于全屏交互程序，粘贴内容不会像 shell 那样直接执行。
        // 这里跳过高危/多行确认，避免编辑器场景误弹确认框。
        if mode.contains(TermMode::ALT_SCREEN) {
            self.paste_text_unchecked(text, window, cx);
            return;
        }

        if self.confirm_high_risk_command && Self::contains_high_risk_command(text) {
            self.show_paste_confirm_dialog(
                text.to_string(),
                t!("TerminalView.high_risk_paste_title").to_string(),
                t!("TerminalView.high_risk_paste_message").to_string(),
                window,
                cx,
            );
            return;
        }

        let is_bracketed_paste = mode.contains(TermMode::BRACKETED_PASTE);

        if !is_bracketed_paste {
            if let Some(hazard) = detect_unbracketed_paste_hazard(text) {
                self.show_unbracketed_paste_block_dialog(text, hazard, window, cx);
                return;
            }
        }

        let is_multiline = multiline_non_empty_line_count(text) > 1;
        if self.confirm_multiline_paste && is_multiline && !is_bracketed_paste {
            self.show_paste_confirm_dialog(
                text.to_string(),
                t!("TerminalView.multiline_paste_title").to_string(),
                t!("TerminalView.multiline_paste_message").to_string(),
                window,
                cx,
            );
            return;
        }

        self.paste_text_unchecked(text, window, cx);
    }

    pub(super) fn paste_text_unchecked(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        let text = normalize_paste_line_endings(text);
        let text = text.as_ref();
        // 仅在应用请求 bracketed paste 模式时才包装，避免把控制序列
        // 原样送进不支持的程序（例如 Vim 未开启时可能导致光标/位置异常）。
        let mode = self.terminal.read(cx).mode();
        self.apply_paste_to_history_prompt(text, cx);
        self.write_paste_to_pty(terminal_paste_bytes(text, mode), cx);
        self.focus_terminal(window, cx);
    }
}
