use super::*;

impl TerminalView {
    fn sync_cd_completion_session(&mut self, session_manager: &Arc<SshSessionManager>) {
        let session_matches = self
            .cd_completion_session_manager
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|cached| Arc::ptr_eq(&cached, session_manager));
        if session_matches {
            return;
        }

        self.cd_completion_session_manager = Some(Arc::downgrade(session_manager));
        self.cd_completion_client = None;
        self.cd_completion_cache.clear();
        self.cd_completion_loading_parent = None;
    }

    fn clear_cd_completion_session(&mut self) {
        self.cd_completion_session_manager = None;
        self.cd_completion_client = None;
        self.cd_completion_cache.clear();
        self.cd_completion_loading_parent = None;
    }

    pub(super) fn history_prompt_enabled(&self, cx: &App) -> bool {
        let terminal = self.terminal.read(cx);
        let Some(connection_kind) = terminal.live_connection_kind() else {
            return false;
        };
        history_prompt_available(
            self.autocomplete_enabled,
            connection_kind,
            self.terminal_frame_snapshot.mode,
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

        let (is_connected, session_manager) = {
            let terminal = self.terminal.read(cx);
            (
                matches!(terminal.connection_state(), ConnectionState::Connected),
                terminal.ssh_session_manager().cloned(),
            )
        };
        let Some(session_manager) = session_manager.filter(|_| is_connected) else {
            self.clear_cd_completion_session();
            self.history_prompt.set_matches(Vec::new());
            return;
        };
        self.sync_cd_completion_session(&session_manager);

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

        if self.cd_completion_loading_parent.as_deref() == Some(query.parent_dir.as_str()) {
            return;
        }

        self.history_prompt.set_matches(Vec::new());
        self.cd_completion_loading_parent = Some(query.parent_dir.clone());
        let existing_client = self.cd_completion_client.clone();
        let request_session_manager = session_manager.clone();
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
                let request_is_current = {
                    let terminal = this.terminal.read(cx);
                    matches!(terminal.connection_state(), ConnectionState::Connected)
                        && terminal
                            .ssh_session_manager()
                            .is_some_and(|current| Arc::ptr_eq(current, &request_session_manager))
                };
                if !request_is_current {
                    return;
                }
                if this.cd_completion_loading_parent.as_deref() == Some(parent_dir.as_str()) {
                    this.cd_completion_loading_parent = None;
                }

                match result {
                    Ok(Ok((client, entries))) => {
                        this.cd_completion_client = Some(client);
                        if !this
                            .cd_completion_cache
                            .insert(
                                parent_dir.clone(),
                                entries
                                    .into_iter()
                                    .filter(|entry| {
                                        entry.is_dir && entry.name != "." && entry.name != ".."
                                    })
                                    .map(|entry| entry.name),
                            )
                        {
                            tracing::warn!(
                                target: "terminal.history_prompt",
                                parent_dir_bytes = parent_dir.len(),
                                "cd completion result was not cached because the parent path is too large"
                            );
                        }

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
