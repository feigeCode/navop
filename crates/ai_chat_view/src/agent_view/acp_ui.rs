use super::*;
use crate::AcpAgentConfig;

impl AgentChatView {
    pub(super) fn select_local_backend(&mut self, cx: &mut Context<Self>) {
        if self.local_backend_is_idle() {
            return;
        }
        self.acp = None;
        self.acp_pending = None;
        self.acp_auth_methods.clear();
        self.current_acp_id = None;
        self.backend = Backend::Local;
        self.acp_connecting = false;
        self.acp_connecting_id = None;
        self.transcript.clear();
        self.set_running(false, cx);
        self.input
            .update(cx, |input, cx| input.set_running(false, cx));
        self._event_task =
            Self::spawn_event_pump(self.runtime.subscribe(), self.session_id.clone(), cx);
        self.sync_composer(cx);
        cx.notify();
    }

    pub(super) fn select_acp_backend(&mut self, id: SharedString, cx: &mut Context<Self>) {
        if self.acp_connecting || self.selected_ready_agent(&id) {
            return;
        }
        let Some(config) = self.ready_acp_config(&id) else {
            return;
        };
        let config = config.with_skill_context(&self.skills.selected_context());
        self.begin_acp_connect(&config, cx);
        cx.spawn(async move |this, cx| {
            let outcome = AcpConnection::connect(&config, cx).await;
            let _ = this.update(cx, |this, cx| {
                this.finish_acp_connect(&config, outcome, cx);
            });
        })
        .detach();
    }

    fn local_backend_is_idle(&self) -> bool {
        self.backend == Backend::Local
            && !self.acp_connecting
            && self.acp_pending.is_none()
            && self.current_acp_id.is_none()
    }

    fn selected_ready_agent(&self, id: &SharedString) -> bool {
        self.backend == Backend::Acp && self.current_acp_id.as_ref() == Some(id)
    }

    fn ready_acp_config(&self, id: &SharedString) -> Option<AcpAgentConfig> {
        self.acp_agents
            .iter()
            .find(|entry| &entry.id == id)
            .and_then(|entry| entry.config.clone())
    }

    fn begin_acp_connect(&mut self, config: &AcpAgentConfig, cx: &mut Context<Self>) {
        self.acp_connecting = true;
        self.acp_connecting_id = Some(config.id.clone());
        self.set_running(false, cx);
        self.transcript.clear();
        self.transcript
            .set_acp_status(format!("正在启动 {}", config.name));
        self.input
            .update(cx, |input, cx| input.set_running(true, cx));
        self.sync_composer(cx);
        cx.notify();
    }

    fn finish_acp_connect(
        &mut self,
        config: &AcpAgentConfig,
        outcome: anyhow::Result<AcpConnectOutcome>,
        cx: &mut Context<Self>,
    ) {
        if self.acp_connecting_id.as_ref() != Some(&config.id) {
            return;
        }
        match outcome {
            Ok(AcpConnectOutcome::Ready(connection)) => {
                self.finish_ready_connect(config.id.clone(), connection, cx)
            }
            Ok(AcpConnectOutcome::AuthenticationRequired(pending)) => {
                self.finish_pending_connect(config, pending, cx)
            }
            Err(error) => self.finish_connect_error(config, error, cx),
        }
    }

    fn finish_ready_connect(
        &mut self,
        agent_id: SharedString,
        connection: AcpConnection,
        cx: &mut Context<Self>,
    ) {
        self.acp_connecting = false;
        self.acp_connecting_id = None;
        self.input
            .update(cx, |input, cx| input.set_running(false, cx));
        self.activate_acp(agent_id, connection, cx);
    }

    fn finish_pending_connect(
        &mut self,
        config: &AcpAgentConfig,
        pending: AcpPendingConnection,
        cx: &mut Context<Self>,
    ) {
        self.acp_auth_methods = pending.methods();
        self.acp_pending = Some(pending);
        self.current_acp_id = Some(config.id.clone());
        self.acp_connecting = false;
        self.acp_connecting_id = None;
        self.transcript
            .set_acp_status(format!("{} 需要登录", config.name));
        self.sync_composer(cx);
        cx.notify();
    }

    fn finish_connect_error(
        &mut self,
        config: &AcpAgentConfig,
        source: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        self.acp_connecting = false;
        self.acp_connecting_id = None;
        self.current_acp_id = Some(config.id.clone());
        self.input
            .update(cx, |input, cx| input.set_running(false, cx));
        let error = AcpError::new(
            AcpErrorKind::InitializeFailed,
            config.id.to_string(),
            config.name.to_string(),
            "连接 ACP Agent 失败",
        )
        .with_detail(source.to_string())
        .with_recovery(AcpRecoveryAction::Retry);
        self.transcript.set_acp_error(&error);
        self.sync_composer(cx);
        cx.notify();
    }

    pub(super) fn authenticate_acp(&mut self, method_id: String, cx: &mut Context<Self>) {
        let Some(pending) = self.acp_pending.take() else {
            return;
        };
        let Some(agent_id) = self.current_acp_id.clone() else {
            return;
        };
        let agent_name = self.acp_agent_name(&agent_id);
        self.acp_auth_methods.clear();
        self.acp_connecting = true;
        self.acp_connecting_id = Some(agent_id.clone());
        self.transcript
            .set_acp_status(format!("正在登录 {agent_name}"));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = pending.authenticate(method_id).await;
            let _ = this.update(cx, |this, cx| {
                this.finish_acp_auth(agent_id, agent_name, result, cx);
            });
        })
        .detach();
    }

    fn finish_acp_auth(
        &mut self,
        agent_id: SharedString,
        agent_name: SharedString,
        result: Result<AcpConnection, AcpError>,
        cx: &mut Context<Self>,
    ) {
        if self.acp_connecting_id.as_ref() != Some(&agent_id) {
            return;
        }
        self.acp_connecting = false;
        self.acp_connecting_id = None;
        self.input
            .update(cx, |input, cx| input.set_running(false, cx));
        match result {
            Ok(connection) => self.activate_acp(agent_id, connection, cx),
            Err(error) => self.finish_auth_error(agent_id, agent_name, error, cx),
        }
    }

    fn finish_auth_error(
        &mut self,
        agent_id: SharedString,
        agent_name: SharedString,
        error: AcpError,
        cx: &mut Context<Self>,
    ) {
        self.current_acp_id = Some(agent_id);
        self.transcript.set_acp_error(&error);
        self.sync_composer(cx);
        cx.notify();
        tracing::warn!(agent = %agent_name, kind = ?error.kind, "ACP authentication failed");
    }

    fn activate_acp(
        &mut self,
        agent_id: SharedString,
        connection: AcpConnection,
        cx: &mut Context<Self>,
    ) {
        let receiver = connection.subscribe();
        let session_id = connection.session_id();
        self.acp = Some(connection);
        self.backend = Backend::Acp;
        self.current_acp_id = Some(agent_id);
        self.transcript.clear_acp_status();
        self._event_task = Self::spawn_event_pump(receiver, session_id, cx);
        self.sync_composer(cx);
        cx.notify();
    }

    pub(super) fn cancel_acp_auth(&mut self, cx: &mut Context<Self>) {
        self.acp_pending = None;
        self.acp_auth_methods.clear();
        self.current_acp_id = None;
        self.acp_connecting = false;
        self.acp_connecting_id = None;
        self.transcript.clear_acp_status();
        self.input
            .update(cx, |input, cx| input.set_running(false, cx));
        self.sync_composer(cx);
        cx.notify();
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
        .child(format!("登录 ({method})"))
        .on_click(move |_, _window, cx| {
            view.update(cx, |this, cx| this.authenticate_acp(method_id.clone(), cx));
        })
}

fn cancel_auth_button(view: Entity<AgentChatView>) -> Button {
    Button::new("acp-auth-cancel")
        .small()
        .outline()
        .child("取消")
        .on_click(move |_, _window, cx| {
            view.update(cx, |this, cx| this.cancel_acp_auth(cx));
        })
}
