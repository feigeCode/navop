//! ACP 历史会话：拉列表、打开一条历史会话、渲染列表行。
//!
//! 方案 §7.1 要的是「内置 agent 会话 ∪ ACP 会话，统一列表」。所以这里只产出
//! 数据和**一份共用的行构造器**，摆在哪里（内建侧栏 / 工作台外壳）由调用方定。
//!
//! 三条纪律：
//! - 能力判定全走 `crate::acp::sessions` 的纯逻辑，渲染层不自己拍脑袋。
//! - 连接只有 `list`/`load`/`resume` 这类真请求才 take，其余时间留在原地。
//! - 异步回写按发起时捕获的世代 + 操作令牌判归属（不变量 5）。

use agent_client_protocol::schema::v1::SessionId as AcpSessionId;
use gpui::{Hsla, SharedString};

use super::*;
use crate::acp::{AcpSessionOpen, AcpSessionSummary, acp_session_open_kind};

/// 重载补全时从连接能力重新推一次打开方式。
///
/// 能力不会在一次 load 之间变化；抽出来是为了让重载路径与首次打开用同一判定。
fn open_kind_at_finish(
    capabilities: &agent_client_protocol::schema::v1::AgentCapabilities,
) -> Option<AcpSessionOpen> {
    acp_session_open_kind(capabilities)
}
/// 回到外部会话的走法（由 [`AgentChatView::plan_acp_reopen`] 判定）。
///
/// 两臂都带上**要重开的那条协议会话 id**，调用方据此设置 `acp_reopen_pending`：
/// 连接握手用哪个地址 load、握手回来后 `promote_pending_acp_session` 拿哪个地址比对，
/// 必须是同一个，否则历史回放对不上、屏幕空白。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum AcpReopenPlan {
    /// 连接已经挂在这个 agent 上：直接把那条协议会话 `load` 回来。
    LoadHere(String),
    /// 需要重新握手：连接就绪后再 `load`。
    Reconnect(SharedString, String),
}

/// 一次 `session/list` 的可见状态。`None`（模型不存在）表示「这个后端不该显示 ACP 列表」，
/// 而不是「列表是空的」——后者是 `Some` + 空 `sessions`。
#[derive(Clone, Debug)]
pub(crate) struct AcpSessionListModel {
    pub(crate) sessions: Vec<AcpSessionSummary>,
    /// 请求在飞。渲染层据此区分「还没拉」和「拉完了是空的」。
    pub(crate) loading: bool,
    /// 上次失败的可见原因；清空表示上次是成功的。
    pub(crate) error: Option<String>,
}

impl AgentChatView {
    /// 当前后端是否该显示 ACP 历史会话区。
    ///
    /// 只看 `acp_sessions_supported`（agent 就绪时从连接能力缓存下来的），不看连接此刻
    /// 在不在——刷新过程中连接会被 take 走，那不该让列表闪掉。
    pub(crate) fn acp_session_list_visible(&self) -> bool {
        self.backend == Backend::Acp && self.acp_sessions_supported
    }

    /// 当前后端是否是 ACP（会话由外接 agent 管理）。
    ///
    /// 与 [`Self::acp_session_list_visible`] 分开：那个还要求 agent 声明了
    /// `session/list` 能力。这个只回答「后端是谁」，用来在能力缺失时给一句说明，
    /// 别让整个会话区凭空消失。
    pub(crate) fn backend_is_acp(&self) -> bool {
        self.backend == Backend::Acp
    }

    /// 给渲染层用的快照；不显示时是 `None`。
    pub(crate) fn acp_session_list_model(&self) -> Option<AcpSessionListModel> {
        self.acp_session_list_visible().then(|| AcpSessionListModel {
            sessions: self.acp_sessions.clone(),
            loading: self.acp_sessions_loading,
            error: self.acp_sessions_error.clone(),
        })
    }

    /// 当前连接指向的 ACP 会话 id，用来把列表里那一条标成「正在用」。
    ///
    /// 要的是能拿去 `session/load` 的**协议**会话 id，和 `session/list` 给的 id
    /// 同一套；不是连接内部的 `acp:<uuid>` 事件流 id，那个永远对不上列表。
    pub(crate) fn acp_session_id_snapshot(&self) -> Option<String> {
        self.acp.as_ref().map(|acp| acp.protocol_session_id())
    }

    /// 把当前挂在外部 agent 上的会话写进会话列表。
    ///
    /// ACP 会话的历史在 agent 那边，本地 `history` 是空的；持久化层那条「空历史
    /// 就跳过」的规则会让这类会话永远进不了侧栏。所以这里单独落一行：地址 + 一个
    /// 可读标题（本地转录里的首条用户消息，没有就用 agent 名）。
    ///
    /// 调用时机都是「真有过动静」：接上 agent、手动打开一条历史会话、一轮结束。
    pub(crate) fn persist_acp_session(&mut self, uid: &str, cx: &mut Context<Self>) {
        let Some(agent_id) = self.current_acp_id.clone() else {
            return;
        };
        let Some(session_id) = self.acp.as_ref().map(|acp| acp.protocol_session_id()) else {
            return;
        };
        if uid.trim().is_empty() || session_id.is_empty() {
            return;
        }
        let reference = agent_runtime::AcpSessionRef {
            agent_id: agent_id.to_string(),
            session_id,
        };
        // 运行时会话也记住这个地址：之后任何一次本地落盘都不会把它抹掉，
        // 切到这条会话时也能凭它知道「这段对话归外部 agent 管」。
        let session_id = SessionId::from_string(uid.to_string());
        if let Some(session) = self.runtime.session(&session_id) {
            session.set_acp_ref(Some(reference.clone()));
            // agent 报的上下文占用一并落快照：快照里的 `context_tokens` 就是
            // 「这条会话现在用了多少」的持久化位置，历史趋势以后要画图表时再建
            // 专门的表，先把快照值记准。
            let usage = self
                .acp
                .as_ref()
                .and_then(|acp| acp.state().usage().cloned());
            if let Some(usage) = usage {
                session.set_context_tokens(Some(usage.used));
            }
        }
        let title = self
            .agent_pushed_session_title()
            .or_else(|| self.external_session_title(uid))
            .unwrap_or_else(|| self.acp_agent_name(&agent_id).to_string());
        let workspace_root = self
            .session_roots
            .get(uid)
            .cloned()
            .unwrap_or_else(|| self.workspace_root.to_string_lossy().into_owned());
        persistence::save_acp_session(cx, uid, &title, Some(&workspace_root), reference);
        // live 摘要也要换上新标题：merge 时 live 的名字会覆盖落盘行，不更新它
        // 的话侧栏要等到重启才能看到 agent 取的新名字。
        let now = now_secs();
        self.upsert_live_summary(uid.to_string(), title, now);
        self.reload_sessions(cx);
    }

    /// 回到一条「外部 agent 承载」的会话：把 agent 那边接回来。
    ///
    /// 本地只有地址（`AcpSessionRef`），历史在 agent 手上，所以走法由
    /// [`Self::plan_acp_reopen`] 判定，这里只负责把选定的路走完。
    pub(crate) fn reopen_acp_session(
        &mut self,
        uid: &str,
        reference: agent_runtime::AcpSessionRef,
        cx: &mut Context<Self>,
    ) {
        match self.plan_acp_reopen(uid, &reference, cx) {
            Some(AcpReopenPlan::LoadHere(session_id)) => {
                // 连接已经在这个 agent 上：`load` 会把历史带回来，不用重新握手。
                self.acp_reopen_pending = None;
                self.open_protocol_session(&session_id, self.workspace_root.clone(), cx);
            }
            Some(AcpReopenPlan::Reconnect(agent_id, session_id)) => {
                // 绕不开重新握手：`resume` 只接上下文、不回放历史，连上后再 load 一次。
                self.acp_reopen_pending = Some(session_id);
                self.select_acp_backend(agent_id, cx);
            }
            None => {}
        }
    }

    /// 判定「回到外部会话」的走法，本身不碰连接。
    ///
    /// agent 已经被移除等接不上的情况，写明原因并返回 `None`，不假装历史还在。
    /// 要重开的协议会话以**记忆**为准（`AiChatSettings.acp_sessions`，每次连接都会刷新），
    /// 只把快照里的地址当兜底：两者可能因写入时机不同而分叉，此时记忆更新，用旧地址会
    /// 让 agent 回到一条不含近期工作的旧会话。
    pub(super) fn plan_acp_reopen(
        &mut self,
        uid: &str,
        reference: &agent_runtime::AcpSessionRef,
        cx: &mut Context<Self>,
    ) -> Option<AcpReopenPlan> {
        let agent_id = SharedString::from(reference.agent_id.clone());
        if self.ready_acp_config(&agent_id).is_none() {
            self.push_system_to_session(
                uid,
                t!("AgentUi.acp_agent_unavailable", name = reference.agent_id).to_string(),
            );
            return None;
        }
        let session_id = AppSettings::current(cx)
            .ai_chat
            .remembered_acp_session(uid, agent_id.as_ref())
            .map(str::to_string)
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| reference.session_id.clone());
        self.remember_acp_protocol_session(uid, &agent_id, &session_id, cx);
        if self.backend == Backend::Acp
            && self.current_acp_id.as_ref() == Some(&agent_id)
            && self.acp.is_some()
        {
            return Some(AcpReopenPlan::LoadHere(session_id));
        }
        Some(AcpReopenPlan::Reconnect(agent_id, session_id))
    }

    /// 这段会话在本地转录里留下的第一条用户消息（标题来源）。
    fn external_session_title(&self, uid: &str) -> Option<String> {
        let transcript = if uid == self.current_session {
            Some(&self.transcript)
        } else {
            self.session_transcripts.get(uid)
        }?;
        transcript.first_user_text().map(str::to_string)
    }

    /// agent 自己给这条会话取的名字（`session/update` 的 `SessionInfoUpdate`）。
    ///
    /// agent 命名比「首条用户消息截断」准得多（它是看过整段对话后总结的），
    /// 所以只要当前连接指向的还是这条会话、agent 也真的推过标题，就用它的。
    /// 连接已指向别的会话（后台轮次收尾时常见的时机）时拿不到，退回本地推导。
    fn agent_pushed_session_title(&self) -> Option<String> {
        let acp = self.acp.as_ref()?;
        if acp.protocol_session_id().is_empty() {
            return None;
        }
        acp.state()
            .title()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string)
    }

    /// 抹掉 ACP 列表状态：切后端、换 agent、能力不支持时调用。
    ///
    /// 顺带推进世代，让还在飞的响应写不进来——否则切走之后旧 agent 的列表会突然出现。
    pub(crate) fn clear_acp_sessions(&mut self) {
        self.acp_sessions.clear();
        self.acp_sessions_error = None;
        self.acp_sessions_loading = false;
        self.acp_sessions_supported = false;
        self.acp_sessions_generation = self.acp_sessions_generation.wrapping_add(1);
    }

    /// 换个世代的编号。
    fn next_acp_sessions_generation(&mut self) -> u64 {
        self.acp_sessions_generation = self.acp_sessions_generation.wrapping_add(1);
        self.acp_sessions_generation
    }

    /// 拉一次历史会话列表。
    ///
    /// 让路条件：连接不在、连不上、正在跑一轮、或已有 ACP 操作占用连接。让路而不是排队——
    /// 列表是环境信息，不是用户提交，晚一拍没有副作用，`activate_acp` 落地时也会再拉一次。
    pub(crate) fn reload_acp_sessions(&mut self, cx: &mut Context<Self>) {
        if !self.acp_session_list_visible() {
            return;
        }
        if self.acp_connecting
            || self.is_running
            || self.acp_session_has_foreground_turn(&self.current_session)
            || self.acp_session_transition.is_some()
        {
            return;
        }
        let Some(agent_id) = self.current_acp_id.clone() else {
            return;
        };
        let session_uid = self.current_session.clone();
        let Some(acp) = self.acp.take() else {
            return;
        };

        // 连接被 take 走了：用 `Listing` 相位把这个窗口公开出去，让提交排队而不是报「未连接」。
        let operation = self.begin_acp_session_transition_with_phase(
            agent_id.clone(),
            session_uid.clone(),
            AcpSessionTransitionPhase::Listing,
        );
        let generation = self.next_acp_sessions_generation();
        self.acp_sessions_loading = true;
        self.acp_sessions_error = None;
        let workspace_root = self.workspace_root.clone();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = acp.list_all_sessions(Some(workspace_root)).await;
            let _ = this.update(cx, |this, cx| {
                this.finish_acp_session_listing(
                    operation,
                    generation,
                    agent_id,
                    session_uid,
                    acp,
                    result,
                    cx,
                );
            });
        })
        .detach();
    }

    fn finish_acp_session_listing(
        &mut self,
        operation: AcpOperationToken,
        generation: u64,
        agent_id: SharedString,
        session_uid: String,
        acp: AcpConnection,
        result: anyhow::Result<Vec<agent_client_protocol::schema::v1::SessionInfo>>,
        cx: &mut Context<Self>,
    ) {
        // 连接只在「槽位还空着、还是同一个 agent」时放回。否则说明已经有更新的操作接管了它，
        // 硬塞回去会把那条操作挤掉。
        let connection_restored =
            self.acp.is_none() && self.current_acp_id.as_ref() == Some(&agent_id);
        if connection_restored {
            self.acp = Some(acp);
        }
        let current_generation = generation == self.acp_sessions_generation;
        if !current_generation {
            // 迟到的旧响应：连接已经放回，结果丢掉。
            cx.notify();
            return;
        }
        self.acp_sessions_loading = false;
        if !self.is_current_acp_session_transition(operation, &agent_id, &session_uid) {
            cx.notify();
            return;
        }
        self.clear_acp_session_transition(operation);
        match result {
            Ok(sessions) => {
                self.acp_sessions = acp_session_summaries(&sessions);
                self.acp_sessions_error = None;
            }
            Err(error) => {
                self.acp_sessions_error = Some(
                    t!("AgentUi.acp_session_list_failed", error = error.to_string()).to_string(),
                );
            }
        }
        self.sync_composer(cx);
        cx.notify();
    }

    /// 打开一条历史会话。
    ///
    /// 走 `load` 还是 `resume` 由 agent 能力决定（`session/load` 能带回历史，优先）。
    /// 之后要重挂事件泵：新会话的 `session/update` 通知认的是新 id。
    pub(crate) fn open_acp_session(&mut self, acp_session_id: &str, cx: &mut Context<Self>) {
        if !self.acp_session_list_visible() {
            return;
        }
        let cwd = self
            .acp_sessions
            .iter()
            .find(|session| session.id == acp_session_id)
            .map(|session| session.cwd.clone())
            .unwrap_or_else(|| self.workspace_root.clone());
        self.open_protocol_session(acp_session_id, cwd, cx);
    }

    /// 打开一条协议会话（列表点击与重开外部会话共用）。
    ///
    /// 不再要求 `session/list` 可见：那是列表 UI 的前提，不是 `load` 的前提——
    /// 重开一条落盘的会话时手上根本没有列表，但 agent 支持 `load` 时仍然要把
    /// 历史带回来。真正的能力判定在 `acp_session_open_kind`（拿不到打开方式就
    /// 什么也不做，不变量 11）。
    pub(crate) fn open_protocol_session(
        &mut self,
        acp_session_id: &str,
        cwd: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        if self.is_running || self.acp_session_has_foreground_turn(&self.current_session) {
            return;
        }
        if self.acp_connecting || self.acp_session_transition.is_some() {
            return;
        }
        let Some(capabilities) = self
            .acp
            .as_ref()
            .map(|acp| acp.state().agent_capabilities().clone())
        else {
            return;
        };
        // 能力缺失就不给入口（不变量 11）：拿不到打开方式时点击应当是哑的，而不是一定失败。
        let Some(open_kind) = acp_session_open_kind(&capabilities) else {
            return;
        };
        let Some(agent_id) = self.current_acp_id.clone() else {
            return;
        };
        let session_uid = self.current_session.clone();
        let Some(mut acp) = self.acp.take() else {
            return;
        };

        let operation = self.begin_acp_session_transition(agent_id.clone(), session_uid.clone());
        let target = AcpSessionId::new(acp_session_id);
        self.acp_turn_owners.clear();
        // `session/load` 会把整段历史重放成一批 `session/update`，它们不属于任何一轮：
        // 连接层要靠回放窗口才认得出，视图这边要靠同一个轮次 id 才敢放行。窗口必须在
        // 请求发出**之前**开——回放是随请求一起推过来的。
        let replay_turn = acp.begin_history_replay();
        self.acp_history_replay = Some(AcpHistoryReplay {
            event_session_id: acp.session_id(),
            session_uid: session_uid.clone(),
            turn_id: replay_turn,
        });
        self.acp_sessions_error = None;
        // 旧转录不能留在屏上：load 会回放历史，resume 从当前状态继续，两者都不该和上一段混排。
        self.transcript.clear();
        self.transcript.set_acp_status(
            t!(
                "AgentUi.acp_session_opening",
                name = self.acp_agent_name(&agent_id)
            )
            .to_string(),
        );
        self.set_running(false, cx);
        self.input
            .update(cx, |input, cx| input.set_running(true, cx));
        self.sync_pending_preview(cx);
        self.sync_composer(cx);
        self.request_scroll_to_bottom();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = match open_kind {
                AcpSessionOpen::Load => acp.load_session(target, cwd).await.map(|_| ()),
                AcpSessionOpen::Resume => acp.resume_session(target, cwd).await.map(|_| ()),
            };
            let _ = this.update(cx, |this, cx| {
                this.finish_acp_session_open(
                    operation, agent_id, session_uid, acp, result, cx,
                );
            });
        })
        .detach();
    }

    fn finish_acp_session_open(
        &mut self,
        operation: AcpOperationToken,
        agent_id: SharedString,
        session_uid: String,
        acp: AcpConnection,
        result: anyhow::Result<()>,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current_acp_session_transition(operation, &agent_id, &session_uid) {
            // 会话已经切走/关掉了：不把连接塞回去，它属于更新那条操作。
            return;
        }
        self.input
            .update(cx, |input, cx| input.set_running(false, cx));
        match result {
            Ok(()) => {
                // 回放期间丢过事件：再 load 一次补全。只补一次；第二次仍丢就
                // 认了——agent 那边每次重放都溢出时，无限重载只会更糟。
                let replay_lagged = self.acp_replay_lagged;
                self.acp_replay_lagged = false;
                if replay_lagged && self.acp_replay_retries == 0 {
                    self.acp_replay_retries = 1;
                    let protocol_session_id = acp.protocol_session_id();
                    tracing::warn!(
                        %protocol_session_id,
                        "ACP history replay lagged; reloading the session once"
                    );
                    // 不把连接塞回去也不清 transition：直接再走一次 load，
                    // 走的还是同一个 operation。
                    let replay_turn = acp.begin_history_replay();
                    self.acp_history_replay = Some(AcpHistoryReplay {
                        event_session_id: acp.session_id(),
                        session_uid: session_uid.clone(),
                        turn_id: replay_turn,
                    });
                    self.transcript.clear();
                    self.transcript.set_acp_status(
                        t!(
                            "AgentUi.acp_session_opening",
                            name = self.acp_agent_name(&agent_id)
                        )
                        .to_string(),
                    );
                    self.input
                        .update(cx, |input, cx| input.set_running(true, cx));
                    let cwd = self.workspace_root.clone();
                    cx.spawn(async move |this, cx| {
                        let target = AcpSessionId::new(protocol_session_id);
                        let mut acp = acp;
                        let result = match open_kind_at_finish(
                            acp.state().agent_capabilities(),
                        ) {
                            Some(AcpSessionOpen::Load) => {
                                acp.load_session(target, cwd).await.map(|_| ())
                            }
                            Some(AcpSessionOpen::Resume) => {
                                acp.resume_session(target, cwd).await.map(|_| ())
                            }
                            None => Ok(()),
                        };
                        let _ = this.update(cx, |this, cx| {
                            this.finish_acp_session_open(
                                operation, agent_id, session_uid, acp, result, cx,
                            );
                        });
                    })
                    .detach();
                    cx.notify();
                    return;
                }
                if replay_lagged {
                    tracing::warn!(
                        "ACP history replay lagged twice; giving up on the refill"
                    );
                }
                self.acp_replay_retries = 0;
                self.clear_acp_session_transition(operation);
                let receiver = acp.subscribe();
                let session_id = acp.session_id();
                let protocol_session_id = acp.protocol_session_id();
                self.acp = Some(acp);
                self.acp_turn_owners.clear();
                self._event_task = Self::spawn_event_pump(receiver, Some(session_id), cx);
                self.transcript.clear_acp_status();
                self.acp_sessions_error = None;
                // 手动打开历史会话之后，这个内置会话就指向它了：下次重连要回到这条。
                self.remember_acp_protocol_session(
                    &session_uid,
                    &agent_id,
                    &protocol_session_id,
                    cx,
                );
                // 从 agent 自己的列表里挑中的会话，同样要留在侧栏会话列表里。
                self.persist_acp_session(&session_uid, cx);
                // `activate_acp` 里那次列表刷新会被正在进行的 load 挡下（连接被占着），
                // 这里补一次：历史摆上屏幕之后，会话列表也该跟上。
                self.reload_acp_sessions(cx);
            }
            Err(error) => {
                let message =
                    t!("AgentUi.acp_session_open_failed", error = error.to_string()).to_string();
                self.acp_sessions_error = Some(message.clone());
                self.mark_acp_session_transition_failed(operation, &agent_id, &session_uid);
                self.acp = Some(acp);
                self.transcript.set_acp_status(message);
            }
        }
        self.sync_pending_preview(cx);
        self.sync_composer(cx);
        self.request_scroll_to_bottom();
        cx.notify();
    }
}

/// 一行 ACP 会话的视觉 + 交互。调用方给 id 和点击回调，避免这里绑死在某个 Entity 上。
///
/// ponytail: 只显示标题（空标题退回 id）和「外部会话」来源标记，不显示 `updated_at`——
/// 协议给的是 ISO 8601 字符串，而内置会话那行显示的是相对时间；为了不引时间库也不假装
/// 「刚刚」，这里先不显示时间。要显示时把两边的格式统一了再做。
pub(crate) fn acp_session_row(
    theme: &AgentChatTheme,
    summary: &AcpSessionSummary,
    selected: bool,
    element_id: SharedString,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::AnyElement {
    let hover = theme.hover_background();
    v_flex()
        .id(element_id)
        .w_full()
        .px_2()
        .py_1p5()
        .gap_0p5()
        .rounded(theme.surface_radius)
        .cursor_pointer()
        .when(selected, |this| this.bg(theme.panel_hover))
        .hover(move |style| style.bg(hover))
        .child(
            div()
                .w_full()
                .min_w_0()
                .truncate()
                .text_sm()
                .text_color(theme.foreground)
                .child(summary.label().to_string()),
        )
        .child(
            h_flex()
                .items_center()
                .gap_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(
                    Icon::new(IconName::ExternalLink)
                        .xsmall()
                        .text_color(theme.muted_foreground),
                )
                .child(t!("AgentUi.acp_session_external").to_string()),
        )
        .on_click(on_click)
        .into_any_element()
}

/// ACP 区的空/加载/失败提示。没有可显示的东西时返回 `None`（不渲染空壳）。
///
/// `has_rows` 决定要不要把「一条都没有」也算作需要提示——列表非空时不该再插一行空态。
pub(crate) fn acp_session_placeholder(
    theme: &AgentChatTheme,
    danger: Hsla,
    loading: bool,
    error: Option<&str>,
    has_rows: bool,
) -> Option<gpui::AnyElement> {
    let (text, color) = match (error, loading, has_rows) {
        (Some(error), _, _) => (error.to_string(), danger),
        (None, true, _) => (t!("AgentUi.acp_session_listing").to_string(), theme.muted_foreground),
        (None, false, false) => (
            t!("AgentUi.acp_sessions_empty").to_string(),
            theme.muted_foreground,
        ),
        (None, false, true) => return None,
    };
    Some(
        div()
            .w_full()
            .px_2()
            .py_2()
            .text_xs()
            .text_color(color)
            .child(text)
            .into_any_element(),
    )
}

/// ACP 区的小标题 + 刷新按钮。
///
/// 手工分组而不是给内置会话行加前缀：内置会话和 ACP 会话是两套持久化，视觉上分开更诚实，
/// 但要挨着放，用户一眼能看到「哪些在 navop 里、哪些在外面」。
pub(crate) fn acp_session_section_header(
    theme: &AgentChatTheme,
    element_id: SharedString,
    on_refresh: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_1()
        .px_2()
        .pt_2()
        .pb_0p5()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("AgentUi.acp_sessions_section").to_string()),
        )
        .child(
            IconButton::new(element_id, IconName::Refresh)
                .role(IconButtonRole::Compact)
                .tooltip(t!("AgentUi.acp_session_refresh").to_string())
                .on_click(on_refresh),
        )
        .into_any_element()
}
