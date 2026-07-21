use super::*;

impl TerminalView {
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
            TerminalSidebarEvent::ExecuteCommand(command) => {
                // 仅粘贴命令，不自动回车执行，降低误操作风险
                self.paste_text(command, window, cx);
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
            TerminalSidebarEvent::AutoCopyChanged(enabled) => {
                self.set_auto_copy(*enabled, cx);
            }
            TerminalSidebarEvent::AutocompleteChanged(enabled) => {
                self.set_autocomplete_enabled(*enabled, cx);
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
            TerminalSidebarEvent::CdToTerminal(path) => {
                // 向终端发送 cd 命令并回车
                let cmd = format!("cd {}\n", shell_escape(path));
                self.write_to_pty(cmd.into_bytes(), cx);
            }
            TerminalSidebarEvent::SyncWorkingDir => {
                if let Some(path) = self
                    .terminal
                    .read(cx)
                    .current_working_dir()
                    .map(str::to_string)
                {
                    self.sidebar.update(cx, |sidebar, cx| {
                        sidebar.sync_file_manager_path(path, cx);
                    });
                }
            }
        }
    }

    /// 内部搜索：向前搜索
    pub(super) fn search_forward_internal(&mut self, cx: &mut Context<Self>) {
        if let Some(search) = self.addon_manager.get_as_mut::<SearchAddon>("search") {
            let term = self.terminal.read(cx).term().clone();
            let mut term = term.lock();
            search.find_next(&mut term);
        }
        cx.notify();
    }

    /// 内部搜索：向后搜索
    pub(super) fn search_backward_internal(&mut self, cx: &mut Context<Self>) {
        if let Some(search) = self.addon_manager.get_as_mut::<SearchAddon>("search") {
            let term = self.terminal.read(cx).term().clone();
            let mut term = term.lock();
            search.find_previous(&mut term);
        }
        cx.notify();
    }
}
