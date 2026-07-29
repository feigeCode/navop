use super::acp_options::agent_selection_is_active;
use super::*;
use crate::AcpAgentConfig;
use rust_i18n::t;

pub(super) struct AcpConnectOperation {
    pub(super) token: AcpOperationToken,
    config: AcpAgentConfig,
    pub(super) session_uid: String,
}

struct AcpAuthOperation {
    token: AcpOperationToken,
    agent_id: SharedString,
    agent_name: SharedString,
    session_uid: String,
}

impl AgentChatView {
    pub(super) fn select_local_backend(&mut self, cx: &mut Context<Self>) {
        let session_uid = self.current_session.clone();
        self.select_local_backend_for_session(&session_uid, cx);
    }

    pub(super) fn select_local_backend_for_session(
        &mut self,
        session_uid: &str,
        cx: &mut Context<Self>,
    ) {
        if self.local_backend_is_idle() {
            return;
        }
        self.invalidate_acp_operation();
        self.reset_acp_permission_session(cx);
        self.acp_turn_owner = None;
        self.acp = None;
        self.acp_pending = None;
        self.acp_auth_methods.clear();
        self.current_acp_id = None;
        self.backend = Backend::Local;
        self.acp_connecting = false;
        self.acp_connecting_id = None;
        self.acp_connect_origin_session = None;
        if session_uid == self.current_session {
            self.transcript.clear();
        } else if let Some(transcript) = self.session_transcripts.get_mut(session_uid) {
            transcript.clear();
        }
        self.set_running(false, cx);
        self.input
            .update(cx, |input, cx| input.set_running(false, cx));
        self._event_task = Self::spawn_event_pump(self.runtime.subscribe(), None, cx);
        self.sync_pending_preview(cx);
        self.sync_composer(cx);
        cx.notify();
    }

    pub(super) fn select_acp_backend(&mut self, id: SharedString, cx: &mut Context<Self>) {
        let Some((operation, permission_provider)) = self.prepare_acp_connect(id, cx) else {
            return;
        };
        self.spawn_acp_connect(operation, permission_provider, cx);
    }

    pub(super) fn spawn_acp_connect(
        &mut self,
        operation: AcpConnectOperation,
        permission_provider: AcpPermissionProvider,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let outcome = AcpConnection::connect_with_permission_provider(
                &operation.config,
                permission_provider,
                cx,
            )
            .await;
            let _ = this.update(cx, |this, cx| {
                this.finish_acp_connect(&operation, outcome, cx);
            });
        })
        .detach();
    }

    pub(super) fn prepare_acp_connect(
        &mut self,
        id: SharedString,
        cx: &mut Context<Self>,
    ) -> Option<(AcpConnectOperation, AcpPermissionProvider)> {
        if self.acp_connecting
            || agent_selection_is_active(
                self.backend,
                self.current_acp_id.as_ref(),
                self.acp.is_some() || self.acp_pending.is_some(),
                &id,
            )
        {
            return None;
        }
        let config = self.ready_acp_config(&id)?;
        self.sync_acp_tool_mode_from_provider(cx);
        let session_uid = self.current_session.clone();
        let operation = AcpConnectOperation {
            token: self.next_acp_operation(),
            config: config.with_skill_context(&self.skills.selected_context()),
            session_uid,
        };
        let permission_provider =
            self.begin_acp_connect(&operation.config, &operation.session_uid, cx);
        Some((operation, permission_provider))
    }

    pub(super) fn sync_acp_tool_mode_from_provider(&mut self, cx: &mut Context<Self>) {
        let Some(mode) = current_acp_tool_mode(cx) else {
            return;
        };
        self.selected_tool = tool_execution_mode_label(mode).into();
        self.sync_composer(cx);
    }

    fn local_backend_is_idle(&self) -> bool {
        self.backend == Backend::Local
            && !self.acp_connecting
            && self.acp_pending.is_none()
            && self.current_acp_id.is_none()
    }

    fn ready_acp_config(&self, id: &SharedString) -> Option<AcpAgentConfig> {
        self.acp_agents
            .iter()
            .find(|entry| &entry.id == id)
            .and_then(|entry| entry.config.clone())
    }

    fn begin_acp_connect(
        &mut self,
        config: &AcpAgentConfig,
        origin_session_uid: &str,
        cx: &mut Context<Self>,
    ) -> AcpPermissionProvider {
        let permission_provider = self.start_acp_permission_session(cx);
        self.backend = Backend::Acp;
        self.current_acp_id = Some(config.id.clone());
        self.acp_turn_owner = None;
        self.acp = None;
        self.acp_pending = None;
        self.acp_auth_methods.clear();
        self.acp_connecting = true;
        self.acp_connecting_id = Some(config.id.clone());
        self.acp_connect_origin_session = Some(origin_session_uid.to_string());
        self._event_task = Self::spawn_event_pump(self.runtime.subscribe(), None, cx);
        self.set_running(false, cx);
        self.transcript.clear();
        self.transcript
            .set_acp_status(t!("AgentUi.starting_agent", name = config.name).to_string());
        self.input
            .update(cx, |input, cx| input.set_running(true, cx));
        self.sync_composer(cx);
        cx.notify();
        permission_provider
    }

    fn finish_acp_connect(
        &mut self,
        operation: &AcpConnectOperation,
        outcome: anyhow::Result<AcpConnectOutcome>,
        cx: &mut Context<Self>,
    ) {
        let config = &operation.config;
        if !self.is_current_acp_connection_operation(
            operation.token,
            &config.id,
            &operation.session_uid,
        ) {
            return;
        }
        match outcome {
            Ok(AcpConnectOutcome::Ready(connection)) => self.finish_ready_connect(
                config.id.clone(),
                operation.session_uid.clone(),
                *connection,
                cx,
            ),
            Ok(AcpConnectOutcome::AuthenticationRequired(pending)) => {
                self.finish_pending_connect(operation, *pending, cx)
            }
            Err(error) => self.finish_connect_error(config, &operation.session_uid, error, cx),
        }
    }

    fn finish_ready_connect(
        &mut self,
        agent_id: SharedString,
        origin_session_uid: String,
        connection: AcpConnection,
        cx: &mut Context<Self>,
    ) {
        self.acp_connecting = false;
        self.acp_connecting_id = None;
        self.input
            .update(cx, |input, cx| input.set_running(false, cx));
        self.activate_acp(agent_id, origin_session_uid, connection, cx);
    }

    fn finish_pending_connect(
        &mut self,
        operation: &AcpConnectOperation,
        pending: AcpPendingConnection,
        cx: &mut Context<Self>,
    ) {
        let config = &operation.config;
        self.acp_auth_methods = pending.methods();
        self.acp_pending = Some(pending);
        self.current_acp_id = Some(config.id.clone());
        self.acp_connecting = false;
        self.acp_connecting_id = None;
        self.input
            .update(cx, |input, cx| input.set_running(false, cx));
        if let Some(transcript) = self.transcript_for_open_session_mut(&operation.session_uid) {
            transcript.set_acp_status(t!("AgentUi.login_required", name = config.name).to_string());
        }
        self.sync_pending_preview(cx);
        self.sync_composer(cx);
        cx.notify();
    }

    fn finish_connect_error(
        &mut self,
        config: &AcpAgentConfig,
        origin_session_uid: &str,
        source: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        self.reset_acp_permission_session(cx);
        self.acp_connecting = false;
        self.acp_connecting_id = None;
        self.acp_connect_origin_session = None;
        self.current_acp_id = Some(config.id.clone());
        self.input
            .update(cx, |input, cx| input.set_running(false, cx));
        let error = AcpError::new(
            AcpErrorKind::InitializeFailed,
            config.id.to_string(),
            config.name.to_string(),
            t!("AgentUi.connect_acp_failed").to_string(),
        )
        .with_detail(source.to_string())
        .with_recovery(AcpRecoveryAction::Retry);
        if let Some(transcript) = self.transcript_for_open_session_mut(origin_session_uid) {
            transcript.set_acp_error(&error);
        }
        self.sync_pending_preview(cx);
        self.sync_composer(cx);
        cx.notify();
    }

    pub(super) fn authenticate_acp(&mut self, method_id: String, cx: &mut Context<Self>) {
        let Some(agent_id) = self.current_acp_id.clone() else {
            return;
        };
        let Some(session_uid) = self.acp_connect_origin_session.clone() else {
            return;
        };
        let Some(pending) = self.acp_pending.take() else {
            return;
        };
        let agent_name = self.acp_agent_name(&agent_id);
        let operation = AcpAuthOperation {
            token: self.next_acp_operation(),
            agent_id,
            agent_name,
            session_uid,
        };
        self.acp_auth_methods.clear();
        self.acp_connecting = true;
        self.acp_connecting_id = Some(operation.agent_id.clone());
        if let Some(transcript) = self.transcript_for_open_session_mut(&operation.session_uid) {
            transcript
                .set_acp_status(t!("AgentUi.logging_in", name = operation.agent_name).to_string());
        }
        self.input
            .update(cx, |input, cx| input.set_running(true, cx));
        self.sync_pending_preview(cx);
        self.sync_composer(cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = pending.authenticate(method_id).await;
            let _ = this.update(cx, |this, cx| {
                this.finish_acp_auth(&operation, result, cx);
            });
        })
        .detach();
    }

    fn finish_acp_auth(
        &mut self,
        operation: &AcpAuthOperation,
        result: Result<AcpConnection, AcpError>,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current_acp_connection_operation(
            operation.token,
            &operation.agent_id,
            &operation.session_uid,
        ) {
            return;
        }
        self.acp_connecting = false;
        self.acp_connecting_id = None;
        self.input
            .update(cx, |input, cx| input.set_running(false, cx));
        match result {
            Ok(connection) => self.activate_acp(
                operation.agent_id.clone(),
                operation.session_uid.clone(),
                connection,
                cx,
            ),
            Err(error) => self.finish_auth_error(
                operation.agent_id.clone(),
                operation.agent_name.clone(),
                &operation.session_uid,
                error,
                cx,
            ),
        }
    }

    fn finish_auth_error(
        &mut self,
        agent_id: SharedString,
        agent_name: SharedString,
        origin_session_uid: &str,
        error: AcpError,
        cx: &mut Context<Self>,
    ) {
        self.reset_acp_permission_session(cx);
        self.acp_connect_origin_session = None;
        self.current_acp_id = Some(agent_id);
        if let Some(transcript) = self.transcript_for_open_session_mut(origin_session_uid) {
            transcript.set_acp_error(&error);
        }
        self.sync_pending_preview(cx);
        self.sync_composer(cx);
        cx.notify();
        tracing::warn!(agent = %agent_name, kind = ?error.kind, "ACP authentication failed");
    }

    fn activate_acp(
        &mut self,
        agent_id: SharedString,
        origin_session_uid: String,
        connection: AcpConnection,
        cx: &mut Context<Self>,
    ) {
        let receiver = connection.subscribe();
        let session_id = connection.session_id();
        self.acp = Some(connection);
        self.acp_turn_owner = None;
        self.acp_session_transition = None;
        self.backend = Backend::Acp;
        self.current_acp_id = Some(agent_id);
        self.acp_connect_origin_session = None;
        if let Some(transcript) = self.transcript_for_open_session_mut(&origin_session_uid) {
            transcript.clear_acp_status();
        }
        self._event_task = Self::spawn_event_pump(receiver, Some(session_id), cx);
        self.sync_pending_preview(cx);
        self.sync_composer(cx);
        self.advance_acp_pending_after_origin(&origin_session_uid, cx);
        cx.notify();
    }

    pub(super) fn cancel_acp_auth(&mut self, cx: &mut Context<Self>) {
        let session_uid = self
            .acp_connect_origin_session
            .clone()
            .unwrap_or_else(|| self.current_session.clone());
        self.pending_submissions.clear_session(&session_uid);
        self.select_local_backend_for_session(&session_uid, cx);
        self.sync_pending_preview(cx);
    }

    pub(super) fn acp_agent_name(&self, id: &SharedString) -> SharedString {
        self.acp_agents
            .iter()
            .find(|entry| &entry.id == id)
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| SharedString::from("ACP Agent"))
    }

    pub(super) fn render_acp_auth_actions(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        self.acp_pending.as_ref()?;
        let view = cx.entity();
        let mut actions = h_flex().flex_wrap().gap_2();
        for method in &self.acp_auth_methods {
            actions = actions.child(auth_button(view.clone(), method));
        }
        actions = actions.child(cancel_auth_button(view));
        Some(
            div()
                .w_full()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(actions)
                .into_any_element(),
        )
    }
}

fn auth_button(view: Entity<AgentChatView>, method: &str) -> Button {
    let method_id = method.to_string();
    Button::new(SharedString::from(format!("acp-auth-{method}")))
        .small()
        .primary()
        .child(t!("AgentUi.login_method", method = method).to_string())
        .on_click(move |_, _window, cx| {
            view.update(cx, |this, cx| this.authenticate_acp(method_id.clone(), cx));
        })
}

fn cancel_auth_button(view: Entity<AgentChatView>) -> Button {
    Button::new("acp-auth-cancel")
        .small()
        .outline()
        .child(t!("AgentUi.cancel").to_string())
        .on_click(move |_, _window, cx| {
            view.update(cx, |this, cx| this.cancel_acp_auth(cx));
        })
}
