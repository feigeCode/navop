use super::credential_capture::{
    CaptureOutcome, CredentialCapture, connection_notice_text, sanitize_notice_text,
    should_emit_connecting_notice,
};
use super::*;

impl TerminalView {
    pub(super) fn handle_terminal_event(
        &mut self,
        _terminal: &Entity<Terminal>,
        event: &TerminalModelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalModelEvent::InputStart => {
                self.shell_prompt_input_active = true;
                self.local_command_running = false;
            }
            TerminalModelEvent::PromptStart => {
                self.shell_prompt_input_active = false;
                self.local_command_running = false;
            }
            TerminalModelEvent::CommandStart => {
                self.shell_prompt_input_active = false;
                self.local_command_running = true;
            }
            TerminalModelEvent::ChildExit(_) => {
                self.shell_prompt_input_active = false;
                self.local_command_running = false;
            }
            _ => {}
        }
        self.refresh_public_mcp_session(cx);

        if should_reset_history_prompt_for_terminal_event(event) {
            self.dismiss_history_prompt();
        }

        match event {
            TerminalModelEvent::Wakeup => {
                self.sync_recording_ticker(cx);
                self.sync_credential_capture(cx);
                self.sync_zmodem_background_task(None, cx);
                self.focus_terminal_after_connect_if_ready(window, cx);
                self.refresh_history_prompt_matches(cx);
                cx.emit(TabContentEvent::ContentChanged);
                cx.notify();
            }
            TerminalModelEvent::HostKeyVerificationRequired => {
                self.show_host_key_verification_dialog(window, cx);
            }
            TerminalModelEvent::CommandHistoryChanged => {
                self.sidebar.update(cx, |sidebar, cx| {
                    sidebar.refresh_history_commands(cx);
                });
                self.refresh_history_prompt_matches(cx);
            }
            TerminalModelEvent::SshCredentialChanged => {
                self.sync_credential_capture(cx);
                self.focus_terminal_after_connect_if_ready(window, cx);
                cx.notify();
            }
            TerminalModelEvent::TelnetCredentialChanged => {
                self.sync_credential_capture(cx);
                self.focus_terminal_after_connect_if_ready(window, cx);
                cx.notify();
            }
            TerminalModelEvent::SshMfaChanged => {
                self.sync_credential_capture(cx);
                self.focus_terminal_after_connect_if_ready(window, cx);
                cx.notify();
            }
            TerminalModelEvent::ZmodemRequestChanged => {
                self.sync_zmodem_picker(cx);
            }
            TerminalModelEvent::ZmodemProgressChanged(progress) => {
                self.sync_zmodem_background_task(Some(progress.clone()), cx);
                cx.notify();
            }
            TerminalModelEvent::ZmodemTransferFinished {
                transfer_id,
                outcome,
                progress,
            } => {
                self.finish_zmodem_background_task(*transfer_id, outcome, progress.clone(), cx);
                cx.notify();
            }
            TerminalModelEvent::PromptStart
            | TerminalModelEvent::InputStart
            | TerminalModelEvent::CommandStart => {
                cx.notify();
            }
            TerminalModelEvent::TitleChanged(_) => {
                cx.emit(TabContentEvent::StateChanged);
            }
            TerminalModelEvent::Bell => {
                // 可选：播放声音或闪烁标签
            }
            TerminalModelEvent::ChildExit(_) => {
                cx.notify();
            }
            TerminalModelEvent::ClipboardStore(data) => {
                cx.write_to_clipboard(ClipboardItem::new_string(data.clone()));
            }
            TerminalModelEvent::WorkingDirChanged(path) => {
                let path = path.clone();
                self.sidebar.update(cx, |sidebar, cx| {
                    sidebar.sync_workspace_explorer_path(path.clone(), cx);
                    sidebar.set_file_manager_initial_dir(path.clone(), cx);
                    sidebar.sync_file_manager_path(path, cx);
                });
            }
            TerminalModelEvent::LockStateChanged => {
                cx.emit(TabContentEvent::StateChanged);
                cx.notify();
            }
        }

        self.sync_connection_status_badge(cx);
    }

    /// Emit `TabContentEvent::StateChanged` when the terminal's connection
    /// status transitions so the owning tab bar can refresh its status badge.
    fn sync_connection_status_badge(&mut self, cx: &mut Context<Self>) {
        let current = self.terminal_connection_status(cx);
        if current == self.last_connection_status {
            return;
        }
        let previous = self.last_connection_status;
        self.last_connection_status = current;
        self.emit_connection_status_notice(previous, current, cx);
        cx.emit(TabContentEvent::StateChanged);
        cx.notify();
    }

    /// 连接状态变化时把提示直接写入终端网格（MobaXterm 风格），替代原
    /// 浮动横条。只在 PTY 不再产出内容的 Connecting/Disconnected 状态注入。
    fn emit_connection_status_notice(
        &mut self,
        previous: Option<one_core::tab_container::TabConnectionStatus>,
        current: Option<one_core::tab_container::TabConnectionStatus>,
        cx: &mut Context<Self>,
    ) {
        use one_core::tab_container::TabConnectionStatus;
        let has_host_key_request = {
            let terminal = self.terminal.read(cx);
            terminal.host_key_verification_request().is_some()
        };
        if has_host_key_request {
            // 主机指纹确认有自己的对话框；此处不注入断线提示避免误导。
            return;
        }
        match current {
            Some(TabConnectionStatus::Connecting) if should_emit_connecting_notice(previous) => {
                self.inject_terminal_notice(
                    &connection_notice_text(&ConnectionState::Connecting),
                    cx,
                );
            }
            Some(TabConnectionStatus::Disconnected) => {
                let state = self.terminal.read(cx).connection_state().clone();
                let mut text = String::new();
                if self
                    .terminal_frame_snapshot
                    .mode
                    .contains(TermMode::ALT_SCREEN)
                {
                    // 会话已死，全屏应用不会再重绘；先回主屏让提示可见。
                    text.push_str("\x1b[?1049l");
                }
                text.push_str(&connection_notice_text(&state));
                self.inject_terminal_notice(&text, cx);
            }
            _ => {}
        }
    }

    fn inject_terminal_notice(&self, text: &str, cx: &mut Context<Self>) {
        self.terminal.update(cx, |terminal, cx| {
            terminal.inject_system_message(text, cx);
        });
    }

    pub(super) fn sync_credential_capture(&mut self, cx: &mut Context<Self>) {
        let active = credential_capture::active_capture_request(&self.terminal.read(cx));
        let already_active = self
            .credential_capture
            .as_ref()
            .is_some_and(|capture| Some(capture.request().clone()) == active);
        if already_active {
            return;
        }
        match active {
            Some(request) => {
                let capture = CredentialCapture::for_request(request);
                self.inject_capture_prompt(&capture, cx);
                self.credential_capture = Some(capture);
            }
            None => {
                self.credential_capture = None;
            }
        }
        cx.notify();
    }

    fn inject_capture_prompt(&self, capture: &CredentialCapture, cx: &mut Context<Self>) {
        let mut text = String::from("\r\n");
        if let Some((name, instructions)) = capture.mfa_prelude() {
            if !name.is_empty() {
                text.push_str(&format!(
                    "\x1b[1m{}\x1b[0m\r\n",
                    sanitize_notice_text(&name)
                ));
            }
            if !instructions.is_empty() {
                text.push_str(&format!("{}\r\n", sanitize_notice_text(&instructions)));
            }
        }
        text.push_str(&capture.prompt_line());
        self.inject_terminal_notice(&text, cx);
    }

    pub(super) fn handle_credential_capture_key_event(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        // Xshell 风格：凭据等待期间 Ctrl+D 关闭当前窗口。
        if modifiers.control && !modifiers.alt && !modifiers.platform && event.keystroke.key == "d"
        {
            one_core::window_close::request_close_window(window.window_handle(), cx);
            return;
        }
        let plain = !modifiers.control && !modifiers.alt && !modifiers.platform;
        match event.keystroke.key.as_str() {
            "enter" if plain => self.handle_credential_capture_submit(window, cx),
            "backspace" if plain => {
                if self
                    .credential_capture
                    .as_mut()
                    .is_some_and(|capture| capture.backspace())
                {
                    self.inject_terminal_notice("\x08 \x08", cx);
                }
            }
            "escape" => self.handle_credential_capture_cancel(cx),
            // 终端直觉：Ctrl+C 中止验证码/OTP 输入（MFA 可取消）。
            "c" if modifiers.control && !modifiers.alt && !modifiers.platform => {
                self.handle_credential_capture_cancel(cx)
            }
            key if plain && key.len() == 1 => self.capture_append_text(key, cx),
            _ => {}
        }
    }

    pub(super) fn capture_append_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let echo = self
            .credential_capture
            .as_mut()
            .is_some_and(|capture| capture.append(text));
        if echo {
            self.inject_terminal_notice(text, cx);
        }
    }

    fn handle_credential_capture_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(capture) = self.credential_capture.as_mut() else {
            return;
        };
        match capture.submit_current() {
            CaptureOutcome::Advanced => {
                let prompt = capture.prompt_line();
                self.inject_terminal_notice(&format!("\r\n{prompt}"), cx);
            }
            CaptureOutcome::Credentials {
                fields,
                username,
                password,
            } => {
                self.inject_terminal_notice("\r\n", cx);
                let submitted = self.terminal.update(cx, |terminal, cx| {
                    if fields.is_telnet {
                        terminal.submit_telnet_credentials(
                            fields.generation,
                            TerminalTelnetCredentials { username, password },
                            cx,
                        )
                    } else {
                        terminal.submit_ssh_credentials(
                            fields.generation,
                            TerminalSshCredentials { username, password },
                            cx,
                        )
                    }
                });
                if submitted {
                    self.focus_terminal_after_connect = true;
                    self.focus_terminal_after_connect_if_ready(window, cx);
                }
            }
            CaptureOutcome::Mfa(responses) => {
                self.inject_terminal_notice("\r\n", cx);
                self.terminal.read(cx).submit_ssh_mfa(responses);
                // capture 状态由随后的 SshMfaChanged 事件统一清理。
            }
            CaptureOutcome::Rejected => {}
        }
        cx.notify();
    }

    fn handle_credential_capture_cancel(&mut self, cx: &mut Context<Self>) {
        let Some(capture) = self.credential_capture.as_ref() else {
            return;
        };
        if capture.cancellable() {
            self.inject_terminal_notice("\r\n", cx);
            self.terminal.read(cx).cancel_ssh_mfa();
        }
    }

    pub(super) fn focus_terminal_after_connect_if_ready(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_terminal_after_connect {
            return;
        }

        let (connection_state, has_credential_request, has_mfa_request) = {
            let terminal = self.terminal.read(cx);
            (
                terminal.connection_state().clone(),
                terminal.ssh_credential_request().is_some()
                    || terminal.telnet_credential_request().is_some(),
                terminal.ssh_mfa_request().is_some(),
            )
        };

        match connection_state {
            ConnectionState::Connected if !has_credential_request && !has_mfa_request => {
                self.focus_terminal_after_connect = false;
                if self.reconnect_success_pending {
                    self.reconnect_success_pending = false;
                    window.push_notification(
                        Notification::success(t!("SshSession.reconnected_new_shell").to_string())
                            .autohide(true),
                        cx,
                    );
                }
                self.focus_terminal(window, cx);
            }
            ConnectionState::Disconnected { .. } => {
                self.focus_terminal_after_connect = false;
                self.reconnect_success_pending = false;
            }
            _ => {}
        }
    }

    pub(super) fn create_addon_manager() -> AddonManager {
        let mut manager = AddonManager::new();
        register_default_addons(&mut manager);
        manager
    }
}
