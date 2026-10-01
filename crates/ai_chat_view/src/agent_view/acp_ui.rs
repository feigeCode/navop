use agent_client_protocol::schema::v1::SessionId as AcpSessionId;

use super::acp_options::agent_selection_is_active;
use super::*;
use crate::AcpAgentConfig;
use rust_i18n::t;

pub(super) struct AcpConnectOperation {
    pub(super) token: AcpOperationToken,
    config: AcpAgentConfig,
    pub(super) session_uid: String,
    /// 这个内置会话 + 这个 agent 上次用的 ACP 协议会话，连接时优先复用。
    pub(super) resume: Option<String>,
    workspace_root: std::path::PathBuf,
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
        // 先把「屏幕上这段是哪个 agent 的内容」记下来：下面会把 `current_acp_id` 清掉，
        // 而用户需要知道刚才那段对话留在 agent 那一侧。
        let leaving_agent = if self.backend == Backend::Acp && !self.transcript.is_empty() {
            self.current_acp_id
                .clone()
                .map(|id| self.acp_agent_name(&id))
        } else {
            None
        };
        self.invalidate_acp_operation();
        self.reset_acp_client_session(cx);
        self.cancel_acp_auto_reconnect();
        self.acp_turn_owner = None;
        self.clear_acp_sessions();
        // 用户主动切回本地：那条待重开的会话不再算数。
        self.acp_reopen_pending = None;
        self.acp = None;
        self.acp_pending = None;
        self.acp_auth_methods.clear();
        self.current_acp_id = None;
        self.pending_acp_model = None;
        self.backend = Backend::Local;
        self.acp_connecting = false;
        self.acp_connecting_id = None;
        self.acp_connect_origin_session = None;
        if session_uid == self.current_session {
            self.restore_local_transcript(session_uid);
            // 与 ACP agent 的对话归 agent 所有：切回本地后它不在屏幕上，也不在本地模型的上下文里。
            // 不说清楚，用户会以为刚才那段对话丢了。
            if let Some(name) = leaving_agent {
                self.transcript
                    .push_system(t!("AgentUi.context_boundary_to_local", name = name).to_string());
            }
        } else {
            // 非当前会话：丢掉这块缓存即可，切到它时按 Runtime 快照重建。
            // 别在这里留一块空壳——空壳会遮蔽重建（见 `restore_local_transcript`）。
            self.remove_cached_session_transcript(session_uid);
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
        // 用户主动选择后端：重新给自动重连一次完整预算。
        self.invalidate_acp_reconnect_schedule();
        self.acp_reconnect.attempts = 0;
        let Some((operation, providers)) = self.prepare_acp_connect(id, cx) else {
            return;
        };
        self.spawn_acp_connect(operation, providers, cx);
    }

    pub(super) fn spawn_acp_connect(
        &mut self,
        operation: AcpConnectOperation,
        providers: AcpClientProviders,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let outcome = AcpConnection::connect_with_providers_and_resume(
                &operation.config,
                operation.workspace_root.clone(),
                providers,
                operation.resume.clone().map(AcpSessionId::new),
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
    ) -> Option<(AcpConnectOperation, AcpClientProviders)> {
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
        // 这个内置会话 + 这个 agent 上次的 ACP 协议会话：连上就优先复用，
        // 别再让 agent 每回开一个新的空会话。
        let resume = AppSettings::current(cx)
            .ai_chat
            .remembered_acp_session(&session_uid, config.id.as_ref())
            .map(str::to_string);
        let operation = AcpConnectOperation {
            token: self.next_acp_operation(),
            config: config.with_skill_context(&self.skills.selected_context()),
            session_uid,
            resume,
            workspace_root: self.workspace_root.clone(),
        };
        let providers = self.begin_acp_connect(&operation.config, &operation.session_uid, cx);
        Some((operation, providers))
    }

    pub(super) fn sync_acp_tool_mode_from_provider(&mut self, cx: &mut Context<Self>) {
        let Some(mode) = current_acp_tool_mode(cx) else {
            return;
        };
        self.tool_execution_mode = mode;
        self.sync_composer(cx);
    }

    fn local_backend_is_idle(&self) -> bool {
        self.backend == Backend::Local
            && !self.acp_connecting
            && self.acp_pending.is_none()
            && self.current_acp_id.is_none()
    }

    pub(super) fn ready_acp_config(&self, id: &SharedString) -> Option<AcpAgentConfig> {
        self.acp_agents
            .iter()
            .find(|entry| &entry.id == id && entry.enabled)
            .and_then(|entry| entry.config.clone())
    }

    pub(super) fn begin_acp_connect(
        &mut self,
        config: &AcpAgentConfig,
        origin_session_uid: &str,
        cx: &mut Context<Self>,
    ) -> AcpClientProviders {
        let providers = self.start_acp_client_session(cx);
        // 本地那段对话归 navop 所有，切走前必须先落进缓存：下面这句 `transcript.clear()`
        // 是为了腾出屏幕显示 ACP 会话，不该顺手把本地转录销毁。
        // 返回值还说明本地到底有没有内容——有内容才需要提示上下文边界。
        let local_had_content = self.stash_current_transcript();
        self.backend = Backend::Acp;
        self.current_acp_id = Some(config.id.clone());
        // 换 agent 时丢弃上一个 agent 的连接前选择，并对新 agent 立刻展示探测到的模型。
        self.pending_acp_model = None;
        // 已有新连接在飞：取消任何待触发的自动重连，避免并发拉起两个进程。
        self.invalidate_acp_reconnect_schedule();
        let connected_agent_id = config.id.clone();
        self.apply_probe_model_options(&connected_agent_id, cx);
        self.acp_turn_owner = None;
        self.clear_acp_sessions();
        self.acp = None;
        self.acp_pending = None;
        self.acp_auth_methods.clear();
        self.acp_connecting = true;
        self.acp_connecting_id = Some(config.id.clone());
        self.acp_connect_origin_session = Some(origin_session_uid.to_string());
        self._event_task = Self::spawn_event_pump(self.runtime.subscribe(), None, cx);
        self.set_running(false, cx);
        self.transcript.clear();
        // 上下文边界要说在明处：agent 看不见本地模型这段对话，它用的是自己的会话上下文。
        // 只在本地确实有内容时提示——空会话切过去不存在「东西被吃掉」的观感，提示只会变噪音。
        if local_had_content {
            self.transcript.push_system(
                t!(
                    "AgentUi.context_boundary_to_acp",
                    name = config.name.clone()
                )
                .to_string(),
            );
        }
        self.transcript
            .set_acp_status(t!("AgentUi.starting_agent", name = config.name).to_string());
        self.input
            .update(cx, |input, cx| input.set_running(true, cx));
        self.sync_composer(cx);
        cx.notify();
        providers
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
        self.reset_acp_client_session(cx);
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
        self.reset_acp_client_session(cx);
        self.acp_connect_origin_session = None;
        self.current_acp_id = Some(agent_id);
        AppSettings::update_and_save(cx, |settings| {
            settings.ai_chat.last_acp_agent_id = self.current_acp_id.as_ref().map(ToString::to_string);
        });
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
        let protocol_session_id = connection.protocol_session_id();
        let continuity = connection.session_continuity();
        self.acp_sessions_supported =
            acp_session_list_supported(connection.state().agent_capabilities());
        let acp_state = connection.state();
        self.model_options = acp_model_options(&acp_state, Some(&agent_id));
        self.selected_model = acp_model_option(&acp_state, Some(&agent_id));
        self.input.update(cx, |input, cx| {
            input.set_menu_options(
                self.model_options.clone(),
                self.tool_options.clone(),
                cx,
            );
        });
        self.acp = Some(connection);
        self.restore_persisted_acp_model(cx);
        self.acp_turn_owner = None;
        self.acp_session_transition = None;
        self.backend = Backend::Acp;
        self.current_acp_id = Some(agent_id.clone());
        self.acp_connect_origin_session = None;
        // 连接健康：清空上一轮的重连预算，并开始空闲健康检查。
        self.acp_reconnect.attempts = 0;
        self.acp_reconnect.scheduled = false;
        #[cfg(not(test))]
        self.spawn_acp_health(agent_id.clone(), cx);
        if let Some(transcript) = self.transcript_for_open_session_mut(&origin_session_uid) {
            transcript.clear_acp_status();
        }
        // 上下文到底接上了没有，要说在明处：复用失败降级新建只写日志的话，
        // 用户会以为对话还在，继续追问才发现 agent 什么都不记得。
        //
        // 只往**屏幕上正显示的那个会话**写：连接期间用户可能已经切走，那时这条提示会落进
        // 另一个会话的本地转录缓存，等切回本地时就变成「本地对话里混着 ACP 文案」。
        if origin_session_uid == self.current_session
            && let Some(message) = session_continuity_notice(continuity, agent_id.as_ref())
        {
            self.push_system_to_session(&origin_session_uid, message);
        }
        self._event_task = Self::spawn_event_pump(receiver, Some(session_id), cx);
        self.sync_pending_preview(cx);
        self.sync_composer(cx);
        self.advance_acp_pending_after_origin(&origin_session_uid, cx);
        // 记住这条会话地址：下次重连 / 重启才有东西可以复用，而不是一路 `session/new`。
        self.remember_acp_protocol_session(
            &origin_session_uid,
            &agent_id,
            &protocol_session_id,
            cx,
        );
        // 会话列表里得有这一行：ACP 会话的本地历史是空的，不在这里落盘就永远不会出现在侧栏。
        // 它读的是 `self.acp`，所以必须排在下面两个会 `take()` 连接的操作之前。
        self.persist_acp_session(&origin_session_uid, cx);
        // 从零重开一条外部会话时，`resume` 只接上下文、不回放历史；agent 支持
        // `session/load` 的话再主动 load 一次，把那段对话摆回屏幕上。
        //
        // 顺序是这里的要害：它必须排在 `reload_acp_sessions` **之前**。后者为了发
        // `session/list` 会把连接 `take()` 走，等轮到 `open_protocol_session` 时它
        // 只能 `take()` 到 `None` 而静默返回——用户点开一条 ACP 历史会话，屏幕一片
        // 空白，日志里连一次失败都看不到（这就是「点击 ACP 会话看不到历史」的根因）。
        self.promote_pending_acp_session(&protocol_session_id, cx);
        // 连接就绪后立刻拉一次历史会话，让统一列表能马上和内置会话并排。
        // 上一轮已经在跑、或上面刚占住连接去 load 时，这里会让路，改由
        // `finish_acp_session_open` 在 load 结束后补拉。
        self.reload_acp_sessions(cx);
        cx.notify();
    }

    /// 兑现「从零重开一条外部会话」的待办：连接回来之后，把那段历史真正 load 回屏幕。
    ///
    /// 只在**连接就在手上**时才消费待办。连接此刻不在（被 `session/list` 之类的操作
    /// `take()` 走了）就先留着——`open_protocol_session` 拿不到连接会静默返回，那时
    /// 把待办一并吃掉，历史就再也没有第二次机会补回来。
    pub(super) fn promote_pending_acp_session(
        &mut self,
        protocol_session_id: &str,
        cx: &mut Context<Self>,
    ) {
        if self.acp_reopen_pending.as_deref() != Some(protocol_session_id) {
            return;
        }
        if self.acp.is_none() {
            return;
        }
        let pending = self
            .acp_reopen_pending
            .take()
            .expect("checked against protocol_session_id above");
        self.open_protocol_session(&pending, self.workspace_root.clone(), cx);
    }

    /// 记住这个内置会话此刻指向的 ACP 协议会话，供下次重连 / 重启复用。
    pub(super) fn remember_acp_protocol_session(
        &self,
        session_uid: &str,
        agent_id: &SharedString,
        acp_session_id: &str,
        cx: &mut Context<Self>,
    ) {
        if acp_session_id.is_empty() || session_uid.is_empty() {
            return;
        }
        AppSettings::update_and_save(cx, |settings| {
            settings
                .ai_chat
                .remember_acp_session(session_uid, agent_id.as_ref(), acp_session_id);
        });
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

/// 会话延续的结果要不要告诉用户，以及说什么。
///
/// 只在「用户的预期与实际不符」时开口：
/// - 复用失败降级新建 → 用户以为对话还在，其实已经断了，必须说；
/// - `session/resume` 接上了但 agent 不回放历史 → 屏幕上空空如也，必须解释；
/// - 回放历史成功（肉眼可见）/ 本来就没有可复用的记忆 → 眼见即事实，不打扰。
pub(super) fn session_continuity_notice(
    continuity: Option<AcpSessionContinuity>,
    agent_name: &str,
) -> Option<String> {
    match continuity? {
        AcpSessionContinuity::RestartedAfterReuseFailure => {
            Some(t!("AgentUi.acp_session_restarted", name = agent_name).to_string())
        }
        AcpSessionContinuity::ReusedWithoutHistory => Some(
            t!(
                "AgentUi.acp_session_resumed_without_history",
                name = agent_name
            )
            .to_string(),
        ),
        AcpSessionContinuity::ReusedWithHistory | AcpSessionContinuity::StartedFresh => None,
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
