use super::*;

const MAX_PENDING_TERMINAL_SEARCH_RUNS: usize = 64;
const MAX_PENDING_TERMINAL_SEARCH_REQUESTS: usize = 256;

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
