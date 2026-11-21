use super::*;
use one_core::settings::AppSettings;

impl TerminalView {
    pub fn sync_sidebar_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.current_theme.clone();
        self.command_bar.update(cx, |command_bar, cx| {
            command_bar.set_colors(theme.colors(), cx);
        });
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.update_current_theme(&theme, window, cx);
        });
    }

    pub(crate) fn add_quick_command(
        &mut self,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command_bar.update(cx, |command_bar, cx| {
            command_bar.add_quick_command(command, window, cx);
        });
    }

    /// 当前会话的终端设置覆盖（侧边栏改动只写这里，不落盘）。
    pub fn session_overrides(&self) -> &TerminalSessionOverrides {
        &self.session_overrides
    }

    /// 本次会话实际生效的终端设置：全局默认值叠加会话覆盖。
    pub fn effective_terminal_settings(&self, cx: &App) -> TerminalSettings {
        let base = current_settings(cx);
        self.session_overrides.merged_into(&base)
    }

    /// 更新会话级覆盖并立刻应用到当前终端。
    ///
    /// 只改内存和渲染状态，不写 `AppSettings`、不落盘；想改全局默认值去「设置 → 终端」。
    pub(crate) fn update_session_overrides(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        updater: impl FnOnce(&mut TerminalSessionOverrides),
    ) {
        let mut next = self.session_overrides.clone();
        updater(&mut next);
        if next == self.session_overrides {
            return;
        }
        self.session_overrides = next;
        let settings = self.effective_terminal_settings(cx);
        self.apply_settings_snapshot(&settings, window, cx);
        cx.notify();
    }

    /// 放弃当前会话的全部覆盖，回到全局默认值。
    pub fn reset_session_overrides(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session_overrides.is_empty() {
            return;
        }
        self.session_overrides.clear();
        let settings = current_settings(cx);
        self.apply_settings_snapshot(&settings, window, cx);
        cx.notify();
    }

    /// 把侧栏各工具面板的停靠位置合并进设置。
    ///
    /// 每台终端可用的面板不同（本地终端有文件浏览器，SSH 终端有文件管理器和
    /// 服务器监控），所以只覆盖自己实际拥有的面板，别把另一类会话的位置抹掉。
    pub(crate) fn persist_tool_panel_placements(&mut self, cx: &mut Context<Self>) {
        let placements = self.sidebar.read(cx).tool_panel_placements();
        let current = AppSettings::current(cx).terminal_tool_panel_placements;
        if placements
            .iter()
            .all(|(panel_id, placement)| current.get(panel_id) == Some(placement))
        {
            return;
        }
        AppSettings::update_and_save(cx, |settings| {
            for (panel_id, placement) in placements {
                settings
                    .terminal_tool_panel_placements
                    .insert(panel_id, placement);
            }
        });
    }

    /// 选中即复制：只影响当前会话。
    pub fn set_auto_copy(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.auto_copy_on_select == enabled {
            return;
        }
        self.auto_copy_on_select = enabled;
        self.session_overrides.auto_copy = Some(enabled);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_auto_copy(enabled, cx);
        });
        cx.notify();
    }

    pub fn apply_autocomplete_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.command_bar.update(cx, |command_bar, cx| {
            command_bar.set_autocomplete_enabled(enabled, cx);
        });
        if self.autocomplete_enabled == enabled {
            return;
        }
        self.autocomplete_enabled = enabled;
        if !enabled {
            self.suggestion_debounce.take();
            self.dismiss_history_prompt();
            self.hide_history_prompt_dropdown();
            self.dismiss_history_prompt_matches();
        }
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_autocomplete_enabled(enabled, cx);
        });
        cx.notify();
    }

    /// 自动补全：只影响当前会话。
    pub fn set_autocomplete_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.autocomplete_enabled == enabled {
            return;
        }
        self.session_overrides.enable_autocomplete = Some(enabled);
        self.apply_autocomplete_enabled(enabled, cx);
    }

    pub fn apply_suggestion_popup_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.command_bar.update(cx, |command_bar, cx| {
            command_bar.set_suggestion_popup_enabled(enabled, cx);
        });
        if self.suggestion_popup_enabled == enabled {
            return;
        }
        self.suggestion_popup_enabled = enabled;
        cx.notify();
    }

    /// 候选词弹层开关：只影响当前会话。
    pub fn set_suggestion_popup_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.suggestion_popup_enabled == enabled {
            return;
        }
        self.session_overrides.show_suggestion_popup = Some(enabled);
        self.apply_suggestion_popup_enabled(enabled, cx);
    }

    /// 设置命令提示 / cd 补全 / 历史搜索下拉弹层的背景不透明度（issue #73）。
    pub fn set_suggestion_popup_opacity(&mut self, opacity: f32, cx: &mut Context<Self>) {
        let opacity = AppSettings::normalize_terminal_suggestion_popup_opacity(opacity);
        if self.suggestion_popup_opacity == opacity {
            return;
        }
        self.suggestion_popup_opacity = opacity;
        self.session_overrides.suggestion_popup_opacity = Some(opacity);
        cx.notify();
    }

    pub fn set_middle_click_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.middle_click_paste == enabled {
            return;
        }
        self.middle_click_paste = enabled;
        self.session_overrides.middle_click_paste = Some(enabled);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_middle_click_paste(enabled, cx);
        });
        cx.notify();
    }

    pub fn set_right_click_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.right_click_paste == enabled {
            return;
        }
        self.right_click_paste = enabled;
        self.session_overrides.right_click_paste = Some(enabled);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_right_click_paste(enabled, cx);
        });
        cx.notify();
    }

    pub fn set_paste_image_upload(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.paste_image_upload == enabled {
            return;
        }
        self.paste_image_upload = enabled;
        self.session_overrides.paste_image_upload = Some(enabled);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_paste_image_upload(enabled, cx);
        });
        cx.notify();
    }

    pub fn set_vim_scroll_to_arrow_keys(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.vim_scroll_to_arrow_keys == enabled {
            return;
        }
        self.vim_scroll_to_arrow_keys = enabled;
        self.session_overrides.vim_scroll_to_arrow_keys = Some(enabled);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_vim_scroll_to_arrow_keys(enabled, cx);
        });
        cx.notify();
    }

    /// 增大字体
    pub fn increase_font_size(&mut self, cx: &mut Context<Self>) {
        let current = f32::from(self.font_size);
        self.set_font_size(current + 1.0, cx);
    }

    /// 减小字体
    pub fn decrease_font_size(&mut self, cx: &mut Context<Self>) {
        let current = f32::from(self.font_size);
        self.set_font_size(current - 1.0, cx);
    }

    /// 重置字体大小为默认值
    pub fn reset_font_size(&mut self, cx: &mut Context<Self>) {
        self.set_font_size(TERMINAL_RESET_FONT_SIZE, cx);
    }

    /// 获取当前字体大小
    pub fn font_size(&self) -> f32 {
        f32::from(self.font_size)
    }

    /// 设置主字体
    pub fn set_font_family(&mut self, family: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.font_family = normalize_terminal_primary_font(family.into().as_ref()).into();
        self.font_metrics = None;
        self.last_size = None;
        cx.notify();
    }

    /// 获取当前主字体
    pub fn font_family(&self) -> &SharedString {
        &self.font_family
    }

    /// 设置行高比例
    pub fn set_line_height_scale(&mut self, scale: f32, cx: &mut Context<Self>) {
        self.line_height_scale = scale.clamp(1.0, 2.5);
        self.line_height = self.font_size * self.line_height_scale;
        self.last_size = None;
        cx.notify();
    }

    /// 获取当前行高比例
    pub fn line_height_scale(&self) -> f32 {
        self.line_height_scale
    }

    pub fn reconnect(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        let Some(duplicate_source) = self.duplicate_source.as_ref() else {
            return;
        };
        let reconnect_source = resolve_ssh_reconnect_source(duplicate_source, |connection_id| {
            let storage = cx
                .try_global::<GlobalStorageState>()
                .ok_or_else(|| anyhow::anyhow!("Global storage is unavailable"))?;
            let repository = storage
                .storage
                .get::<ConnectionRepository>()
                .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;
            repository.get(connection_id)
        });
        let reconnect_source = match reconnect_source {
            Ok(source) => source,
            Err(error) => {
                tracing::error!(%error, "Failed to load latest SSH connection for reconnect");
                self.inject_terminal_notice(
                    &format!(
                        "\r\n\x1b[31m{}\x1b[0m\r\n",
                        t!(
                            "TerminalView.reconnect_load_latest_failed",
                            locale = "en",
                            error = error
                        )
                    ),
                    cx,
                );
                return;
            }
        };
        if let Some(source) = reconnect_source {
            let runtime_connection =
                match connection_form::credential::resolve_connection_for_runtime(
                    source.connection.clone(),
                    cx,
                ) {
                    Ok(connection) => connection,
                    Err(error) => {
                        tracing::error!(%error, "Failed to resolve latest SSH credentials for reconnect");
                        self.inject_terminal_notice(
                            &format!(
                                "\r\n\x1b[31m{}\x1b[0m\r\n",
                                t!(
                                    "TerminalView.reconnect_load_latest_failed",
                                    locale = "en",
                                    error = error
                                )
                            ),
                            cx,
                        );
                        return;
                    }
                };
            let apply_result = self.terminal.update(cx, |terminal, _cx| {
                terminal.apply_ssh_connection_update(SshConnectionUpdate {
                    connection: runtime_connection,
                    working_dir: source.working_dir.clone(),
                    sync_path_with_terminal: source.sync_path_with_terminal,
                })
            });
            if let Err(error) = apply_result {
                tracing::error!(%error, "Failed to apply latest SSH connection for reconnect");
                self.inject_terminal_notice(
                    &format!(
                        "\r\n\x1b[31m{}\x1b[0m\r\n",
                        t!(
                            "TerminalView.reconnect_apply_latest_failed",
                            locale = "en",
                            error = error
                        )
                    ),
                    cx,
                );
                return;
            }
            self.duplicate_source = Some(TerminalDuplicateSource::Ssh {
                connection: source.connection,
                working_dir: source.working_dir,
                sync_path_with_terminal: source.sync_path_with_terminal,
            });
        }

        let working_dir = self
            .terminal
            .read(cx)
            .current_working_dir()
            .map(str::to_string);
        let reconnect_started = self
            .terminal
            .update(cx, |terminal, cx| terminal.reconnect(cx));
        self.focus_terminal_after_connect = reconnect_started;
        if !reconnect_started {
            return;
        }

        cx.spawn(async move |this, cx| {
            loop {
                let state = match this.update(cx, |this, cx| {
                    this.terminal.read(cx).connection_state().clone()
                }) {
                    Ok(state) => state,
                    Err(_) => break,
                };

                match state {
                    ConnectionState::Connected => {
                        let _ = this.update(cx, |this, cx| {
                            this.sidebar.update(cx, |sidebar, cx| {
                                sidebar.reconnect_file_manager(working_dir.clone(), cx);
                                sidebar.reconnect_server_monitor(cx);
                            });
                        });
                        break;
                    }
                    ConnectionState::Disconnected { .. } => break,
                    ConnectionState::Connecting => {
                        cx.background_executor()
                            .timer(Duration::from_millis(100))
                            .await;
                    }
                }
            }
        })
        .detach();
    }
}
