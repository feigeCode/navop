use super::*;

impl TerminalView {
    pub(super) fn write_to_pty(&mut self, data: Vec<u8>, cx: &mut Context<Self>) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        self.write_input_to_terminal(&data, cx);
        self.broadcast_user_input(&data, cx);
    }

    pub(super) fn write_broadcast_input(&mut self, data: Vec<u8>, cx: &mut Context<Self>) {
        if !self.is_live_ssh_terminal(cx) {
            return;
        }
        self.write_input_to_terminal(&data, cx);
    }

    pub(super) fn write_input_to_terminal(&mut self, data: &[u8], cx: &mut Context<Self>) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        // 用户输入时自动滚动到底部
        let display_offset = self.terminal_frame_snapshot.display_offset;
        if should_scroll_to_bottom_on_user_input(
            display_offset,
            &self.scrollbar_handle.future_display_offset,
        ) {
            if !self.scrollbar_handle.try_set_display_offset(0) {
                self.scrollbar_handle.put_back_future_display_offset(0);
                self.schedule_terminal_render_retry(cx);
            }
        }
        self.terminal.read(cx).write(data);
    }

    pub(super) fn commit_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        if !text.is_empty() {
            self.apply_inline_input_to_history_prompt(text, cx);
            self.write_to_pty(text.as_bytes().to_vec(), cx);
        }
    }

    pub(super) fn set_marked_text(
        &mut self,
        _text: String,
        range: Option<std::ops::Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        self.ime_state = Some(ImeState {
            marked_range: range,
        });
        cx.notify();
    }

    pub(super) fn clear_marked_text(&mut self, cx: &mut Context<Self>) {
        if self.ime_state.is_some() {
            self.ime_state = None;
            cx.notify();
        }
    }

    pub(super) fn marked_text_range(&self) -> Option<std::ops::Range<usize>> {
        self.ime_state
            .as_ref()
            .and_then(|state| state.marked_range.clone())
    }

    pub(super) fn handle_key_event(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if keystroke_matches_shortcuts(
            &event.keystroke,
            &shortcuts_for(cx, action_id::TERMINAL_COPY, &[TERMINAL_COPY_SHORTCUT]),
        ) {
            self.copy(&Copy, _window, cx);
            return;
        }

        // Xshell 风格：会话未激活（未连接/已断开）时 Ctrl+D 关闭当前窗口；
        // 会话存活时 Ctrl+D 照常透传为 EOF（\x04）退出 shell。
        // 锁定会话不关闭窗口，仅拦截输入。
        let is_locked = self.terminal.read(cx).is_locked();
        if !self.accepts_live_terminal_input(cx) && !is_locked {
            let modifiers = event.keystroke.modifiers;
            if modifiers.control
                && !modifiers.alt
                && !modifiers.shift
                && !modifiers.platform
                && event.keystroke.key == "d"
            {
                one_core::window_close::request_close_window(_window.window_handle(), cx);
            }
            return;
        }

        if !self.accepts_live_terminal_input(cx) {
            return;
        }

        // 输入时暂停闪烁
        if self.cursor_blink_enabled {
            self.blink_manager.update(cx, BlinkCursor::pause);
        }

        if keystroke_matches_shortcuts(
            &event.keystroke,
            &shortcuts_for(cx, action_id::TERMINAL_PASTE, &terminal_paste_defaults()),
        ) {
            self.paste(&Paste, _window, cx);
            return;
        }

        let mode = self.terminal_frame_snapshot.mode;

        if mode.contains(TermMode::VI) {
            self.hide_history_prompt_dropdown();
            self.handle_vi_key_event(event, cx);
            return;
        }

        let modifiers = event.keystroke.modifiers;
        let key = event.keystroke.key.as_str();
        tracing::debug!(
            target: "terminal.history_prompt",
            reason = "key_event",
            key,
            modifiers = ?modifiers,
            shell_mode = ?mode,
            "terminal key event"
        );

        if modifiers.control && !modifiers.alt && !modifiers.platform && key == "r" {
            if self.start_history_search(cx) {
                return;
            }
        }

        if self.history_prompt.mode() == HistoryPromptMode::Search {
            if !modifiers.control && !modifiers.alt && !modifiers.platform {
                match key {
                    "up" if self.try_navigate_history_prompt(false, cx) => return,
                    "down" if self.try_navigate_history_prompt(true, cx) => return,
                    "right" | "enter" if self.try_accept_history_prompt(cx) => return,
                    "backspace" => {
                        self.history_prompt.backspace();
                        self.refresh_history_prompt_matches(cx);
                        cx.notify();
                        return;
                    }
                    "escape" => {
                        self.exit_history_search(cx);
                        return;
                    }
                    "space" => {
                        self.history_prompt.append_text(" ");
                        self.refresh_history_prompt_matches(cx);
                        cx.notify();
                        return;
                    }
                    _ if key.len() == 1 => {
                        self.history_prompt.append_text(key);
                        self.history_prompt.show_dropdown();
                        self.refresh_history_prompt_matches(cx);
                        cx.notify();
                        return;
                    }
                    _ => {
                        self.hide_history_prompt_dropdown();
                    }
                }
            } else {
                self.hide_history_prompt_dropdown();
            }
        }

        if modifiers.control && !modifiers.alt && !modifiers.platform {
            match key {
                "u" | "c" => self.clear_history_prompt(),
                // Ctrl+Right: 逐词接受建议
                "right" if self.try_accept_next_word_history_prompt(cx) => return,
                _ if should_dismiss_history_prompt_for_keystroke(&event.keystroke) => {
                    self.dismiss_history_prompt();
                }
                _ => self.hide_history_prompt_dropdown(),
            }
        }

        if !modifiers.control && !modifiers.alt && !modifiers.platform {
            match key {
                "up" if self.try_navigate_history_prompt(false, cx) => return,
                "down" if self.try_navigate_history_prompt(true, cx) => return,
                "right" if self.try_accept_history_prompt(cx) => return,
                "backspace" => {
                    if self.history_prompt_enabled(cx) {
                        self.history_prompt.backspace();
                        if self.history_prompt.query_input().is_empty() {
                            self.suggestion_debounce.take();
                            cx.notify();
                        } else {
                            self.schedule_debounced_refresh(cx);
                        }
                    }
                }
                "enter" => {
                    let _ = self.try_accept_explicit_history_prompt(cx);
                    self.clear_history_prompt();
                }
                "left" | "home" | "end" | "delete" => {
                    self.dismiss_history_prompt();
                }
                "pageup" | "pagedown" => {
                    self.dismiss_history_prompt();
                }
                "escape" => {
                    self.dismiss_history_prompt();
                }
                _ => {
                    if should_defer_inline_history_prompt_input_to_text_system(&event.keystroke) {
                        // 普通文本输入统一走 EntityInputHandler::replace_text_in_range -> commit_text，
                        // 避免 keydown 与文本系统各自追加一次，导致 history_prompt 双写。
                    } else if should_dismiss_history_prompt_for_keystroke(&event.keystroke) {
                        self.dismiss_history_prompt();
                    }
                }
            }
        } else if modifiers.alt && !modifiers.control && !modifiers.platform {
            // Alt+F: 逐词接受建议（emacs 风格）
            if key == "f" && self.try_accept_next_word_history_prompt(cx) {
                return;
            }
            self.dismiss_history_prompt();
        } else {
            self.dismiss_history_prompt();
        }

        if is_terminal_action_shortcut(&event.keystroke, cx) {
            return;
        }

        if let Some(command) = self
            .command_bar
            .read(cx)
            .command_for_shortcut(&event.keystroke)
        {
            self.write_to_pty(command.into_bytes(), cx);
            return;
        }

        let backspace_code = self.terminal.read(cx).telnet_backspace_code();
        if let Some(esc_str) =
            crate::keys::to_esc_str_with_backspace(&event.keystroke, &mode, false, backspace_code)
        {
            let bytes = match esc_str {
                Cow::Borrowed(s) => s.as_bytes().to_vec(),
                Cow::Owned(s) => s.into_bytes(),
            };
            self.write_to_pty(bytes, cx);
        }
    }
}
