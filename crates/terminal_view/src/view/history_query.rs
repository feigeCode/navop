use super::*;

impl TerminalView {
    pub(super) fn history_prompt_enabled(&self, cx: &App) -> bool {
        let terminal = self.terminal.read(cx);
        let Some(connection_kind) = terminal.live_connection_kind() else {
            return false;
        };
        let mode = terminal.mode();
        history_prompt_available(
            self.autocomplete_enabled,
            connection_kind,
            mode,
            self.shell_prompt_input_active,
        )
    }

    pub(super) fn refresh_history_prompt_matches(&mut self, cx: &mut Context<Self>) {
        if !self.history_prompt_enabled(cx) {
            self.hide_history_prompt_dropdown();
            return;
        }

        if !self.history_prompt.is_active() {
            self.history_prompt.set_matches(Vec::new());
            return;
        }

        if let Some(query) = self.current_cd_completion_query(cx) {
            self.refresh_cd_completion_matches(query, cx);
            return;
        }

        let terminal = self.terminal.read(cx);
        let matches = match self.history_prompt.mode() {
            HistoryPromptMode::InlineSuggest => terminal
                .history_suggestions(self.history_prompt.query_input(), HISTORY_SUGGESTION_LIMIT),
            HistoryPromptMode::Search => terminal.history_search_results(
                self.history_prompt.query_input(),
                HISTORY_SUGGESTION_LIMIT,
            ),
        };
        let first_match = matches.first().cloned().unwrap_or_default();
        self.history_prompt.set_matches(matches);
        tracing::debug!(
            target: "terminal.history_prompt",
            reason = "refresh_matches",
            mode = ?self.history_prompt.mode(),
            query = %self.history_prompt.query_input(),
            matches_len = self.history_prompt.matches().len(),
            first_match = %first_match,
            "history prompt refreshed"
        );
    }

    pub(super) fn current_cd_completion_query(&self, cx: &App) -> Option<CdCompletionQuery> {
        if self.history_prompt.mode() != HistoryPromptMode::InlineSuggest {
            return None;
        }

        let terminal = self.terminal.read(cx);
        if !live_ssh_feature_supported(terminal.live_connection_kind()) {
            return None;
        }

        parse_cd_completion_query(
            self.history_prompt.query_input(),
            terminal.current_working_dir(),
        )
    }

    pub(super) fn refresh_cd_completion_matches(
        &mut self,
        query: CdCompletionQuery,
        cx: &mut Context<Self>,
    ) {
        if !self.is_live_ssh_terminal(cx) {
            return;
        }
        if let Some(directory_names) = self.cd_completion_cache.get(&query.parent_dir) {
            let matches = build_cd_completion_suggestions(&query, directory_names);
            self.history_prompt.set_matches(matches);
            tracing::debug!(
                target: "terminal.history_prompt",
                reason = "refresh_cd_matches_cached",
                query = %self.history_prompt.query_input(),
                parent_dir = %query.parent_dir,
                matches_len = self.history_prompt.matches().len(),
                "cd completion refreshed from cache"
            );
            return;
        }

        let Some(session_manager) = self.terminal.read(cx).ssh_session_manager().cloned() else {
            self.history_prompt.set_matches(Vec::new());
            return;
        };

        if self.cd_completion_loading_parent.as_deref() == Some(query.parent_dir.as_str()) {
            return;
        }

        self.history_prompt.set_matches(Vec::new());
        self.cd_completion_loading_parent = Some(query.parent_dir.clone());
        let existing_client = self.cd_completion_client.clone();
        let parent_dir = query.parent_dir.clone();
        let request_parent_dir = parent_dir.clone();
        let task = Tokio::spawn(cx, async move {
            let client = match existing_client {
                Some(client) => client,
                None => {
                    let shared_client = session_manager.client().await?;
                    Arc::new(Mutex::new(
                        RusshSftpClient::connect_with_client(shared_client).await?,
                    ))
                }
            };
            let entries = {
                let mut client = client.lock().await;
                client.list_dir(&request_parent_dir).await?
            };
            Ok::<_, anyhow::Error>((client, entries))
        });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.cd_completion_loading_parent = None;
                match result {
                    Ok(Ok((client, entries))) => {
                        this.cd_completion_client = Some(client);
                        let directory_names = entries
                            .into_iter()
                            .filter(|entry| entry.is_dir && entry.name != "." && entry.name != "..")
                            .map(|entry| entry.name)
                            .collect::<Vec<_>>();
                        this.cd_completion_cache
                            .insert(parent_dir.clone(), directory_names);

                        if let Some(current_query) = this.current_cd_completion_query(cx) {
                            if current_query.parent_dir == parent_dir {
                                if let Some(directory_names) =
                                    this.cd_completion_cache.get(&parent_dir)
                                {
                                    let matches = build_cd_completion_suggestions(
                                        &current_query,
                                        directory_names,
                                    );
                                    this.history_prompt.set_matches(matches);
                                    cx.notify();
                                }
                            }
                        }
                    }
                    Ok(Err(error)) => {
                        tracing::warn!(
                            target: "terminal.history_prompt",
                            parent_dir = %parent_dir,
                            error = %error,
                            "cd completion list_dir failed"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "terminal.history_prompt",
                            parent_dir = %parent_dir,
                            error = %error,
                            "cd completion task failed"
                        );
                    }
                }
            });
        })
        .detach();
    }
}
