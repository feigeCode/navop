use super::*;
use crate::quick_command_parameters::prompt_for_quick_command_parameters;

/// 断开态 Ctrl+D 的关闭目标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DisconnectedCtrlDCloseTarget {
    /// 关闭所在标签页：交给宿主 TabContainer 走正常关闭生命周期。
    Tab,
    /// 独立窗口内的终端没有标签容器，退化为关闭窗口。
    Window,
}

/// 断开态 Ctrl+D 判定：仅 ctrl+d，不带 alt/shift/platform 修饰键。
pub(super) fn is_disconnected_ctrl_d(keystroke: &Keystroke) -> bool {
    let modifiers = keystroke.modifiers;
    modifiers.control
        && !modifiers.alt
        && !modifiers.shift
        && !modifiers.platform
        && keystroke.key == "d"
}

/// 按终端渲染模式决定断开态 Ctrl+D 关闭什么。
pub(super) fn disconnected_ctrl_d_close_target(
    render_mode: TerminalRenderMode,
) -> DisconnectedCtrlDCloseTarget {
    match render_mode {
        TerminalRenderMode::WorkspacePane => DisconnectedCtrlDCloseTarget::Tab,
        TerminalRenderMode::Embedded => DisconnectedCtrlDCloseTarget::Window,
    }
}

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
            if self.credential_capture.is_some() {
                self.capture_append_text(text, cx);
            }
            return;
        }
        if !text.is_empty() {
            self.apply_inline_input_to_history_prompt(text, cx);
            self.write_to_pty(text.as_bytes().to_vec(), cx);
        }
    }

    /// 断开状态下按 Enter 立即重连；返回是否已消费该按键。
    fn try_reconnect_on_enter(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let reconnectable = {
            let terminal = self.terminal.read(cx);
            matches!(
                terminal.connection_state(),
                ConnectionState::Disconnected { .. }
            ) && terminal.can_reconnect()
        };
        if !reconnectable {
            return false;
        }
        let modifiers = &keystroke.modifiers;
        if keystroke.key != "enter" || modifiers.control || modifiers.alt || modifiers.platform {
            return false;
        }
        self.reconnect(window, cx);
        true
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

        // 凭据/MFA 等待期间按键进入终端内联捕获，不透传 PTY。
        if self.credential_capture.is_some() {
            self.handle_credential_capture_key_event(event, _window, cx);
            return;
        }

        // 断开状态下 Enter 直接触发重连（提示已写入终端网格）。
        if self.try_reconnect_on_enter(&event.keystroke, _window, cx) {
            return;
        }

        // Xshell / MobaXterm 风格：会话已断开（SSH/串口断开、shell 已退出）时，Ctrl+D
        // 关闭当前标签页；会话存活时 Ctrl+D 照常透传为 EOF（\x04）结束 shell，第二次
        // Ctrl+D 才命中这里。锁定会话不关闭标签页，仅拦截输入。
        //
        // 注意：`accepts_live_terminal_input` 只判断"终端类型是否支持实时输入"，SSH 会话
        // 断开后它仍然为 true，因此必须叠加 `terminal_is_disconnected` 才覆盖断开态。
        let is_locked = self.terminal.read(cx).is_locked();
        let session_inactive =
            self.terminal_is_disconnected(cx) || !self.accepts_live_terminal_input(cx);
        if !is_locked && session_inactive && is_disconnected_ctrl_d(&event.keystroke) {
            match disconnected_ctrl_d_close_target(self.render_mode) {
                // 工作区标签内：由 TerminalWorkspace 决定关当前 pane 还是整个标签页。
                DisconnectedCtrlDCloseTarget::Tab => {
                    cx.emit(TerminalPaneEvent::ClosePaneRequested);
                }
                // 独立窗口里的终端没有标签容器，保持关闭窗口的兜底行为。
                DisconnectedCtrlDCloseTarget::Window => {
                    one_core::window_close::request_close_window(_window.window_handle(), cx);
                }
            }
            return;
        }

        // 不支持实时输入的终端（只读/录像回放）其余按键一律不透传。
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
            let terminal = cx.entity();
            if prompt_for_quick_command_parameters(
                &command,
                cx.theme().muted_foreground,
                _window,
                cx,
                move |command, window, cx| {
                    terminal.update(cx, |terminal, cx| {
                        terminal.write_to_pty(command.into_bytes(), cx);
                        terminal.focus_terminal(window, cx);
                    });
                },
            ) {
                return;
            }
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
