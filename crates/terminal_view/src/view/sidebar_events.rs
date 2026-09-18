use super::*;
use one_core::storage::models::SshAuthMethod;
use ssh::SshAuth;
use terminal::terminal::SshTerminalConfig;

const MAX_PENDING_TERMINAL_SEARCH_RUNS: usize = 64;
const MAX_PENDING_TERMINAL_SEARCH_REQUESTS: usize = 256;

/// “保存为连接”时用于补全临时连接的目标信息与运行时凭据。
#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimeSshTarget {
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
}

/// 从终端的运行时会话配置中取出目标信息与运行时输入的凭据。
///
/// 临时连接在提交凭据后会把运行时配置写回 `Terminal::ssh_config`，
/// 这里的密码只应存在于内存中，用于预填保存表单并写入凭据库。
fn runtime_ssh_target(ssh_config: Option<&SshTerminalConfig>) -> Option<RuntimeSshTarget> {
    let runtime = ssh_config?;
    let password = match &runtime.ssh_config.auth {
        SshAuth::Password(password) => Some(password.clone()).filter(|value| !value.is_empty()),
        _ => None,
    };
    Some(RuntimeSshTarget {
        host: runtime.ssh_config.host.clone(),
        port: runtime.ssh_config.port,
        username: runtime.ssh_config.username.clone(),
        password,
    })
}

/// 把运行时目标信息与凭据合并进临时连接，得到可直接交给保存表单的连接。
///
/// 临时连接默认每次连接都要求输入用户名 / 密码，这里已经有运行时值，
/// 因此清空对应的 `prompt_*` 标记，让保存表单默认勾选“保存用户名 / 密码”。
fn temporary_connection_with_runtime_target(
    connection: &StoredConnection,
    target: &RuntimeSshTarget,
) -> StoredConnection {
    let Ok(mut params) = connection.to_ssh_params() else {
        return connection.clone();
    };

    if !target.host.is_empty() {
        params.host = target.host.clone();
        params.port = target.port;
    }
    if !target.username.is_empty() {
        params.username = target.username.clone();
        params.prompt_username = None;
    }
    if let Some(password) = target.password.as_deref() {
        params.auth_method = SshAuthMethod::Password {
            password: password.to_string(),
        };
        params.prompt_password = None;
    }

    // 名称交给 `new_ssh` 生成，避免带上临时连接的 “(temporary)” 后缀。
    let mut merged = StoredConnection::new_ssh(String::new(), params, connection.workspace_id);
    merged.remark = connection.remark.clone();
    merged.sync_enabled = connection.sync_enabled;
    merged.team_id = connection.team_id.clone();
    merged.owner_id = connection.owner_id.clone();
    merged.preferred_open_mode = connection.preferred_open_mode;
    merged
}

struct TerminalSearchCompletion {
    generation: u64,
    pattern: String,
    previous_match: Option<std::ops::RangeInclusive<AlacPoint>>,
    result: Option<std::ops::RangeInclusive<AlacPoint>>,
    display_offset: Option<usize>,
}

fn terminal_search_display_offset(term: &Term<GpuiEventProxy>, point: AlacPoint) -> usize {
    let current = term.grid().display_offset() as i64;
    let history_size = term.history_size() as i64;
    let screen_lines = term.screen_lines() as i64;
    let line = point.line.0 as i64;

    let target = if line < -current {
        -line
    } else if line >= screen_lines - current {
        screen_lines.saturating_sub(1).saturating_sub(line)
    } else {
        current
    };
    target.clamp(0, history_size) as usize
}

impl TerminalView {
    pub(super) fn handle_quick_command_sync_event(
        &mut self,
        _notifier: &Entity<QuickCommandSyncNotifier>,
        _event: &QuickCommandSyncEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command_bar.update(cx, |command_bar, cx| {
            command_bar.load_quick_commands(cx);
            command_bar.refresh_suggestions(cx);
        });
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.refresh_quick_commands(cx);
        });
    }

    pub(super) fn handle_workspace_editor_event(
        &mut self,
        _editor: &Entity<WorkspaceEditor>,
        event: &WorkspaceEditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, WorkspaceEditorEvent::VisibilityChanged(false)) {
            self.focus_terminal(window, cx);
        }
        if matches!(event, WorkspaceEditorEvent::VisibilityChanged(_)) {
            cx.emit(TabContentEvent::StateChanged);
            cx.notify();
        }
    }

    pub(super) fn handle_terminal_settings_event(
        &mut self,
        _store: &Entity<crate::settings::TerminalSettingsStore>,
        event: &TerminalSettingsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalSettingsEvent::Changed { current, .. } => {
                self.apply_settings_snapshot(current, window, cx);
            }
        }
    }

    pub(super) fn handle_app_settings_changed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = current_settings(cx);
        self.apply_settings_snapshot(&settings, window, cx);
    }

    pub(super) fn handle_app_theme_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = current_settings(cx);
        let theme = TerminalTheme::resolve(&settings.theme, cx.theme());
        self.apply_theme(&theme, window, cx);
    }

    /// 构造“保存为连接”所需的连接信息。
    ///
    /// 返回 `None` 表示当前终端不是临时连接（已有数据库记录）。
    fn temporary_connection_for_save(&self, cx: &Context<Self>) -> Option<StoredConnection> {
        let connection = self.sidebar.read(cx).temporary_connection()?;
        let ssh_config = self.terminal.read(cx).ssh_config().cloned();
        Some(match runtime_ssh_target(ssh_config.as_ref()) {
            Some(target) => temporary_connection_with_runtime_target(&connection, &target),
            None => connection,
        })
    }

    /// SSH 凭据就绪后补建文件管理器 / 服务器监控面板。
    ///
    /// 需要运行时输入凭据的 SSH 连接在构造时拿不到 `SshSessionManager`，
    /// 这两个面板只能等凭据提交后再创建。
    pub(super) fn ensure_ssh_tool_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ssh_tool_panels_pending {
            return;
        }
        let Some(session_manager) = self.terminal.read(cx).ssh_session_manager().cloned() else {
            return;
        };
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.install_ssh_tool_panels(session_manager, window, cx);
        });
        if self.sidebar.read(cx).ssh_tool_panels_ready() {
            self.ssh_tool_panels_pending = false;
        }
    }

    /// 处理侧边栏事件
    pub(super) fn handle_sidebar_event(
        &mut self,
        _sidebar: &Entity<TerminalSidebar>,
        event: &TerminalSidebarEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalSidebarEvent::PanelChanged(_panel) => {
                cx.emit(TabContentEvent::StateChanged);
                cx.notify();
            }
            TerminalSidebarEvent::SearchPatternChanged(pattern) => {
                let _ = self.set_search_pattern(pattern);
                cx.notify();
            }
            TerminalSidebarEvent::SearchPrevious => {
                self.search_backward_internal(cx);
            }
            TerminalSidebarEvent::SearchNext => {
                self.search_forward_internal(cx);
            }
            TerminalSidebarEvent::FontSizeChanged(size) => {
                self.set_font_size(*size, cx);
            }
            TerminalSidebarEvent::FontFamilyChanged(family) => {
                let family = family.clone();
                let _ = update_settings(cx, move |settings| {
                    settings.font_family = family;
                });
            }
            TerminalSidebarEvent::ThemeChanged(theme) => {
                let theme_name = theme.name.to_string();
                let _ = update_settings(cx, move |settings| {
                    settings.theme = theme_name;
                });
            }
            TerminalSidebarEvent::ScrollbackLinesChanged(lines) => {
                let lines = *lines;
                let _ = update_settings(cx, move |settings| {
                    settings.scrollback_lines = lines;
                });
            }
            TerminalSidebarEvent::ExecuteCommand(command) => {
                // 末尾显式换行是“点击即执行”的标记；默认仍只粘贴，避免误操作。
                if quick_command_executes_on_click(command) {
                    self.command_bar.update(cx, |_, cx| {
                        cx.emit(TerminalCommandBarEvent::Submit(command.clone()));
                    });
                } else {
                    self.paste_text(command, window, cx);
                }
            }
            TerminalSidebarEvent::QuickCommandsChanged => {
                self.command_bar.update(cx, |command_bar, cx| {
                    command_bar.load_quick_commands(cx);
                    command_bar.refresh_suggestions(cx);
                });
            }
            TerminalSidebarEvent::PasteCodeToTerminal(code) => {
                // 粘贴代码块到终端（使用 bracketed paste 模式，不自动执行）
                self.paste_code_block(&code, window, cx);
            }
            TerminalSidebarEvent::AskAi => {
                // AI 请求已由 sidebar 内部处理，这里只需要通知刷新
                cx.notify();
            }
            TerminalSidebarEvent::CursorBlinkChanged(enabled) => {
                let enabled = *enabled;
                let _ = update_settings(cx, move |settings| {
                    settings.cursor_blink = enabled;
                });
            }
            TerminalSidebarEvent::SelectionHighlightChanged(enabled) => {
                let enabled = *enabled;
                let _ = update_settings(cx, move |settings| {
                    settings.selection_highlight = enabled;
                });
            }
            TerminalSidebarEvent::ShowLineTimestampsChanged(enabled) => {
                let enabled = *enabled;
                let _ = update_settings(cx, move |settings| {
                    settings.show_line_timestamps = enabled;
                });
            }
            TerminalSidebarEvent::ShowLineNumbersChanged(enabled) => {
                let enabled = *enabled;
                let _ = update_settings(cx, move |settings| {
                    settings.show_line_numbers = enabled;
                });
            }
            TerminalSidebarEvent::ConfirmMultilinePasteChanged(enabled) => {
                let enabled = *enabled;
                let _ = update_settings(cx, move |settings| {
                    settings.confirm_multiline_paste = enabled;
                });
            }
            TerminalSidebarEvent::ConfirmHighRiskCommandChanged(enabled) => {
                let enabled = *enabled;
                let _ = update_settings(cx, move |settings| {
                    settings.confirm_high_risk_command = enabled;
                });
            }
            TerminalSidebarEvent::AutoSessionLoggingChanged(enabled) => {
                let enabled = *enabled;
                let _ = update_settings(cx, move |settings| {
                    settings.auto_session_logging = enabled;
                });
            }
            TerminalSidebarEvent::AutoCopyChanged(enabled) => {
                self.set_auto_copy(*enabled, cx);
            }
            TerminalSidebarEvent::AutocompleteChanged(enabled) => {
                self.set_autocomplete_enabled(*enabled, cx);
            }
            TerminalSidebarEvent::SuggestionPopupChanged(enabled) => {
                self.apply_suggestion_popup_enabled(*enabled, cx);
            }
            TerminalSidebarEvent::MiddleClickPasteChanged(enabled) => {
                self.set_middle_click_paste(*enabled, cx);
            }
            TerminalSidebarEvent::RightClickPasteChanged(enabled) => {
                self.set_right_click_paste(*enabled, cx);
            }
            TerminalSidebarEvent::PasteImageUploadChanged(enabled) => {
                self.set_paste_image_upload(*enabled, cx);
            }
            TerminalSidebarEvent::VimScrollToArrowKeysChanged(enabled) => {
                self.set_vim_scroll_to_arrow_keys(*enabled, cx);
            }
            TerminalSidebarEvent::SyncPathChanged(enabled) => {
                let enabled = *enabled;
                let _ = update_settings(cx, move |settings| {
                    settings.sync_path_with_terminal = enabled;
                });
            }
            TerminalSidebarEvent::CustomHighlightsChanged(rules) => {
                let rules = rules.clone();
                let _ = update_settings(cx, move |settings| {
                    settings.custom_highlights = rules;
                });
            }
            TerminalSidebarEvent::OpenSftp(connection) => {
                cx.emit(TerminalPaneEvent::OpenSftp(connection.clone()));
            }
            TerminalSidebarEvent::SaveAsConnection => {
                if let Some(connection) = self.temporary_connection_for_save(cx) {
                    cx.emit(TerminalPaneEvent::SaveAsConnection(connection));
                }
            }
            TerminalSidebarEvent::CdToTerminal(path) => {
                // 向终端发送 cd 命令并回车
                let cmd = format!("cd {}\n", shell_escape(path));
                self.write_to_pty(cmd.into_bytes(), cx);
            }
            TerminalSidebarEvent::SyncWorkingDir => {
                // 手动同步同样要带上上报主机名：跨主机时面板需要拒收路径，
                // 而不是把别台机器的目录套到当前远程会话上。
                if let Some(reported) = self
                    .terminal
                    .read(cx)
                    .reported_working_dir()
                    .cloned()
                {
                    self.sidebar.update(cx, |sidebar, cx| {
                        sidebar.sync_file_manager_path(reported, cx);
                    });
                }
            }
        }
    }

    pub(super) fn invalidate_terminal_searches(&mut self) {
        self.pending_terminal_searches.clear();
        self.terminal_search_generation
            .fetch_add(1, Ordering::AcqRel);
    }

    fn enqueue_terminal_search(
        &mut self,
        direction: TerminalSearchDirection,
        cx: &mut Context<Self>,
    ) {
        let pending_request_count = self
            .pending_terminal_searches
            .iter()
            .map(|pending| pending.repetitions as usize)
            .sum::<usize>()
            + usize::from(self.terminal_search_task.is_some());
        if pending_request_count >= MAX_PENDING_TERMINAL_SEARCH_REQUESTS {
            tracing::warn!(
                max_pending_requests = MAX_PENDING_TERMINAL_SEARCH_REQUESTS,
                "dropping terminal search request because the pending queue is full"
            );
            return;
        }

        let merged = if let Some(pending) = self.pending_terminal_searches.back_mut() {
            if pending.direction == direction {
                pending.repetitions += 1;
                true
            } else {
                false
            }
        } else {
            false
        };
        if !merged && self.pending_terminal_searches.len() < MAX_PENDING_TERMINAL_SEARCH_RUNS {
            self.pending_terminal_searches
                .push_back(PendingTerminalSearch {
                    direction,
                    repetitions: 1,
                });
        } else if !merged {
            tracing::warn!(
                max_pending_runs = MAX_PENDING_TERMINAL_SEARCH_RUNS,
                "dropping terminal search request because the pending queue is full"
            );
            return;
        }
        self.start_next_terminal_search(cx);
    }

    fn start_next_terminal_search(&mut self, cx: &mut Context<Self>) {
        if self.terminal_search_task.is_some() {
            return;
        }

        let Some(pending) = self.pending_terminal_searches.front_mut() else {
            return;
        };
        let direction = pending.direction;
        pending.repetitions -= 1;
        if pending.repetitions == 0 {
            self.pending_terminal_searches.pop_front();
        }

        let Some(request) = self
            .addon_manager
            .get_as::<SearchAddon>("search")
            .and_then(SearchAddon::search_request)
        else {
            self.pending_terminal_searches.clear();
            return;
        };

        let term = self.terminal.read(cx).term().clone();
        let generation_counter = self.terminal_search_generation.clone();
        let generation = generation_counter.load(Ordering::Acquire);
        let task = cx.background_executor().spawn(async move {
            if generation_counter.load(Ordering::Acquire) != generation {
                return None;
            }

            let mut term = term.lock();
            if generation_counter.load(Ordering::Acquire) != generation {
                return None;
            }

            let mut regex = request.regex;
            let result = find_terminal_search_match(
                &mut term,
                &mut regex,
                request.current_match.as_ref(),
                direction,
            );
            let display_offset = result
                .as_ref()
                .map(|result| terminal_search_display_offset(&term, *result.start()));

            if generation_counter.load(Ordering::Acquire) != generation {
                return None;
            }

            Some(TerminalSearchCompletion {
                generation,
                pattern: request.pattern,
                previous_match: request.current_match,
                result,
                display_offset,
            })
        });

        self.terminal_search_task = Some(cx.spawn(async move |this, cx| {
            let completion = task.await;
            let _ = this.update(cx, |this, cx| {
                this.terminal_search_task = None;

                if let Some(completion) = completion {
                    if this.terminal_search_generation.load(Ordering::Acquire)
                        == completion.generation
                    {
                        let applied = this
                            .addon_manager
                            .get_as_mut::<SearchAddon>("search")
                            .is_some_and(|search| {
                                search.apply_search_result(
                                    &completion.pattern,
                                    &completion.previous_match,
                                    completion.result,
                                )
                            });

                        if applied {
                            if let Some(display_offset) = completion.display_offset {
                                if !this.scrollbar_handle.try_set_display_offset(display_offset) {
                                    this.scrollbar_handle
                                        .put_back_future_display_offset(display_offset);
                                    this.schedule_terminal_render_retry(cx);
                                }
                            }
                            cx.notify();
                        }
                    }
                }

                this.start_next_terminal_search(cx);
            });
        }));
    }

    /// 内部搜索：向前搜索
    pub(super) fn search_forward_internal(&mut self, cx: &mut Context<Self>) {
        self.enqueue_terminal_search(TerminalSearchDirection::Forward, cx);
    }

    /// 内部搜索：向后搜索
    pub(super) fn search_backward_internal(&mut self, cx: &mut Context<Self>) {
        self.enqueue_terminal_search(TerminalSearchDirection::Backward, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeSshTarget, temporary_connection_with_runtime_target};
    use one_core::storage::StoredConnection;
    use one_core::storage::models::{SshAuthMethod, SshParams};

    /// 与主页快速连接生成的临时连接保持一致：默认每次连接都输入用户名 / 密码。
    fn temporary_ssh_connection() -> StoredConnection {
        let params: SshParams = serde_json::from_value(serde_json::json!({
            "host": "example.com",
            "port": 22,
            "username": "",
            "auth_method": { "Password": { "password": "" } },
            "prompt_username": true,
            "prompt_password": true,
        }))
        .expect("临时连接的参数 JSON 应可解析");
        StoredConnection::new_ssh("SSH example.com (temporary)".to_string(), params, None)
    }

    fn runtime_target() -> RuntimeSshTarget {
        RuntimeSshTarget {
            host: "10.0.0.5".to_string(),
            port: 2222,
            username: "alice".to_string(),
            password: Some("s3cret".to_string()),
        }
    }

    #[test]
    fn runtime_credentials_replace_temporary_prompts() {
        let merged = temporary_connection_with_runtime_target(
            &temporary_ssh_connection(),
            &runtime_target(),
        );
        let params = merged.to_ssh_params().expect("合并后的参数应可解析");

        assert_eq!("10.0.0.5", params.host);
        assert_eq!(2222, params.port);
        assert_eq!("alice", params.username);
        match &params.auth_method {
            SshAuthMethod::Password { password } => assert_eq!("s3cret", password),
            other => panic!("运行时凭据应合并为密码认证，实际为 {other:?}"),
        }
        // 运行时已有值，保存表单应默认勾选保存用户名 / 密码。
        assert!(!params.prompts_for_username());
        assert!(!params.prompts_for_password());
        // 新连接没有数据库 ID，名称也不带临时标记。
        assert_eq!(None, merged.id);
        assert_eq!("alice@10.0.0.5:2222", merged.name);
    }

    #[test]
    fn missing_runtime_password_keeps_password_prompt() {
        let target = RuntimeSshTarget {
            password: None,
            ..runtime_target()
        };
        let merged = temporary_connection_with_runtime_target(&temporary_ssh_connection(), &target);
        let params = merged.to_ssh_params().expect("合并后的参数应可解析");

        assert_eq!("alice", params.username);
        assert!(!params.prompts_for_username());
        // 没有拿到运行时密码，仍保留每次输入密码的行为。
        assert!(params.prompts_for_password());
    }

    #[test]
    fn unparsable_params_are_left_untouched() {
        let mut connection = temporary_ssh_connection();
        connection.params = "{not json".to_string();

        let merged = temporary_connection_with_runtime_target(&connection, &runtime_target());

        assert_eq!("{not json", merged.params);
    }
}
