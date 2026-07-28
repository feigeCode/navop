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
                self.sync_operation_history(cx);
                self.sync_ssh_mfa_inputs(window, cx);
                self.focus_terminal_after_connect_if_ready(window, cx);
                self.refresh_history_prompt_matches(cx);
                cx.emit(TabContentEvent::ContentChanged);
                cx.notify();
            }
            TerminalModelEvent::CommandHistoryChanged => {
                self.sidebar.update(cx, |sidebar, cx| {
                    sidebar.refresh_history_commands(cx);
                });
                self.refresh_history_prompt_matches(cx);
            }
            TerminalModelEvent::SshMfaChanged => {
                self.sync_ssh_mfa_inputs(window, cx);
                self.focus_terminal_after_connect_if_ready(window, cx);
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
        }
    }

    pub(super) fn sync_ssh_mfa_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.terminal.read(cx).ssh_mfa_request() else {
            self.ssh_mfa_inputs.clear();
            return;
        };

        let inputs_match_request = self.ssh_mfa_inputs.len() == request.prompts.len()
            && self
                .ssh_mfa_inputs
                .iter()
                .zip(request.prompts.iter())
                .all(|(input, prompt)| input.prompt == prompt.prompt && input.echo == prompt.echo);

        if !inputs_match_request {
            self.ssh_mfa_inputs = request
                .prompts
                .iter()
                .map(|prompt| SshMfaInput {
                    prompt: prompt.prompt.clone(),
                    echo: prompt.echo,
                    input: cx.new(|cx| {
                        let mut state =
                            InputState::new(window, cx).placeholder(prompt.prompt.clone());
                        if !prompt.echo {
                            state = state.masked(true);
                        }
                        state
                    }),
                })
                .collect();
        }
        if let Some(input) = self.ssh_mfa_inputs.first() {
            input.input.update(cx, |state, cx| state.focus(window, cx));
        }
    }

    pub(super) fn submit_ssh_mfa(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let responses = self
            .ssh_mfa_inputs
            .iter()
            .map(|input| input.input.read(cx).text().to_string())
            .collect();
        if self.terminal.read(cx).submit_ssh_mfa(responses) {
            self.ssh_mfa_inputs.clear();
            self.focus_terminal_after_connect = true;
            self.focus_terminal_after_connect_if_ready(window, cx);
        }
        cx.notify();
    }

    pub(super) fn focus_terminal_after_connect_if_ready(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_terminal_after_connect {
            return;
        }

        let (connection_state, has_mfa_request) = {
            let terminal = self.terminal.read(cx);
            (
                terminal.connection_state().clone(),
                terminal.ssh_mfa_request().is_some(),
            )
        };

        match connection_state {
            ConnectionState::Connected if !has_mfa_request => {
                self.focus_terminal_after_connect = false;
                self.focus_terminal(window, cx);
            }
            ConnectionState::Disconnected { .. } => {
                self.focus_terminal_after_connect = false;
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
