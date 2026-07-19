use super::*;

impl TerminalView {
    pub(super) fn dismiss_history_prompt(&mut self) {
        self.history_prompt.dismiss();
    }

    pub(super) fn hide_history_prompt_dropdown(&mut self) {
        self.history_prompt.hide_dropdown();
    }

    pub(super) fn apply_inline_input_to_history_prompt(
        &mut self,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        if !self.history_prompt_enabled(cx) {
            self.hide_history_prompt_dropdown();
            return;
        }
        self.history_prompt.append_text(text);
        self.history_prompt.show_dropdown();
        self.schedule_debounced_refresh(cx);
    }

    /// 防抖刷新建议匹配（30ms 延迟）
    pub(super) fn schedule_debounced_refresh(&mut self, cx: &mut Context<Self>) {
        self.suggestion_debounce.take();
        self.suggestion_debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(30))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.refresh_history_prompt_matches(cx);
                cx.notify();
            });
        }));
    }

    pub(super) fn apply_paste_to_history_prompt(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.history_prompt_enabled(cx) {
            self.hide_history_prompt_dropdown();
            return;
        }
        self.history_prompt.apply_paste(text);
        self.refresh_history_prompt_matches(cx);
    }

    pub(super) fn clear_history_prompt(&mut self) {
        self.dismiss_history_prompt();
    }

    pub(super) fn start_history_search(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.history_prompt_enabled(cx) || !self.history_prompt.is_active() {
            self.hide_history_prompt_dropdown();
            return false;
        }

        if self.history_prompt.mode() == HistoryPromptMode::Search {
            return self.try_navigate_history_prompt(true, cx);
        }

        self.history_prompt.enter_search();
        self.refresh_history_prompt_matches(cx);
        cx.notify();
        true
    }

    pub(super) fn exit_history_search(&mut self, cx: &mut Context<Self>) {
        if self.history_prompt.mode() != HistoryPromptMode::Search {
            return;
        }
        self.history_prompt.exit_search();
        self.refresh_history_prompt_matches(cx);
        cx.notify();
    }

    pub(super) fn dismiss_history_prompt_matches(&mut self) {
        self.history_prompt.dismiss_matches();
    }

    pub(super) fn replace_history_prompt_line(&mut self, command: &str, cx: &mut Context<Self>) {
        tracing::debug!(
            target: "terminal.history_prompt",
            reason = "replace_line",
            command = %command,
            "history prompt replacing terminal line"
        );
        let mut bytes = Vec::with_capacity(command.len() + 1);
        bytes.extend_from_slice(b"\x15");
        bytes.extend_from_slice(command.as_bytes());
        self.write_to_pty(bytes, cx);
    }

    pub(super) fn apply_history_prompt_accept(
        &mut self,
        accepted: HistoryPromptAccept,
        selected_match: Option<String>,
        cx: &mut Context<Self>,
    ) {
        match accepted {
            HistoryPromptAccept::AppendSuffix(suffix) => {
                tracing::debug!(
                    target: "terminal.history_prompt",
                    reason = "accept_suffix",
                    query = %self.history_prompt.query_input(),
                    selected_match = ?selected_match,
                    suffix = %suffix,
                    "history prompt accepted suffix"
                );
                self.write_to_pty(suffix.into_bytes(), cx)
            }
            HistoryPromptAccept::ReplaceLine(command) => {
                tracing::debug!(
                    target: "terminal.history_prompt",
                    reason = "accept_replace_line",
                    query = %self.history_prompt.query_input(),
                    selected_match = ?selected_match,
                    command = %command,
                    "history prompt accepted line replacement"
                );
                self.replace_history_prompt_line(&command, cx);
            }
        }
        self.dismiss_history_prompt_matches();
        cx.notify();
    }

    pub(super) fn try_accept_history_prompt(&mut self, cx: &mut Context<Self>) -> bool {
        let selected_match = self.history_prompt.selected_match().map(str::to_string);
        let Some(accepted) = self.history_prompt.accept_selected_suggestion() else {
            tracing::debug!(
                target: "terminal.history_prompt",
                reason = "accept_rejected",
                mode = ?self.history_prompt.mode(),
                query = %self.history_prompt.query_input(),
                selected_match = ?selected_match,
                "history prompt accept rejected"
            );
            return false;
        };
        self.apply_history_prompt_accept(accepted, selected_match, cx);
        true
    }

    pub(super) fn try_accept_explicit_history_prompt(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.history_prompt_enabled(cx) || !self.history_prompt.is_active() {
            return false;
        }

        let selected_match = self.history_prompt.selected_match().map(str::to_string);
        let Some(accepted) = self.history_prompt.accept_explicit_selection() else {
            tracing::debug!(
                target: "terminal.history_prompt",
                reason = "accept_explicit_rejected",
                mode = ?self.history_prompt.mode(),
                query = %self.history_prompt.query_input(),
                selected_match = ?selected_match,
                "history prompt explicit accept rejected"
            );
            return false;
        };
        self.apply_history_prompt_accept(accepted, selected_match, cx);
        true
    }

    /// 逐词接受建议（Ctrl+Right / Alt+F）
    pub(super) fn try_accept_next_word_history_prompt(&mut self, cx: &mut Context<Self>) -> bool {
        let selected_match = self.history_prompt.selected_match().map(str::to_string);
        let Some(accepted) = self.history_prompt.accept_next_word() else {
            tracing::debug!(
                target: "terminal.history_prompt",
                reason = "accept_next_word_rejected",
                query = %self.history_prompt.query_input(),
                selected_match = ?selected_match,
                "history prompt next-word accept rejected"
            );
            return false;
        };
        match accepted {
            HistoryPromptAccept::AppendSuffix(suffix) => {
                tracing::debug!(
                    target: "terminal.history_prompt",
                    reason = "accept_next_word_suffix",
                    query = %self.history_prompt.query_input(),
                    selected_match = ?selected_match,
                    suffix = %suffix,
                    "history prompt accepted next word"
                );
                self.write_to_pty(suffix.into_bytes(), cx)
            }
            HistoryPromptAccept::ReplaceLine(command) => {
                tracing::debug!(
                    target: "terminal.history_prompt",
                    reason = "accept_next_word_replace_line",
                    query = %self.history_prompt.query_input(),
                    selected_match = ?selected_match,
                    command = %command,
                    "history prompt next-word triggered line replacement"
                );
                self.replace_history_prompt_line(&command, cx);
            }
        }
        cx.notify();
        true
    }

    pub(super) fn try_navigate_history_prompt(
        &mut self,
        previous: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.history_prompt_enabled(cx) {
            return false;
        }

        if self.history_prompt.matches().is_empty() {
            self.refresh_history_prompt_matches(cx);
        }

        let command = if previous {
            self.history_prompt.navigate_previous()
        } else {
            self.history_prompt.navigate_next()
        };
        if command.is_none() {
            return false;
        }
        cx.notify();
        true
    }

    pub(super) fn select_history_prompt_match(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(_) = self.history_prompt.select_match(index) else {
            return;
        };
        cx.notify();
    }
}
