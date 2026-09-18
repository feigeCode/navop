//! ACP 历史会话：拉列表、打开一条历史会话、渲染列表行。
//!
//! 方案 §7.1 要的是「内置 agent 会话 ∪ ACP 会话，统一列表」。所以这里只产出
//! 数据和**一份共用的行构造器**，摆在哪里（内建侧栏 / 工作台外壳）由调用方定。
//!
//! 三条纪律：
//! - 能力判定全走 `crate::acp::sessions` 的纯逻辑，渲染层不自己拍脑袋。
//! - 连接只有 `list`/`load`/`resume` 这类真请求才 take，其余时间留在原地。
//! - 异步回写按发起时捕获的世代 + 操作令牌判归属（不变量 5）。

use agent_client_protocol::schema::SessionId as AcpSessionId;
use gpui::{Hsla, SharedString};

use super::*;
use crate::acp::{AcpSessionOpen, AcpSessionSummary};

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

    /// 给渲染层用的快照；不显示时是 `None`。
    pub(crate) fn acp_session_list_model(&self) -> Option<AcpSessionListModel> {
        self.acp_session_list_visible().then(|| AcpSessionListModel {
            sessions: self.acp_sessions.clone(),
            loading: self.acp_sessions_loading,
            error: self.acp_sessions_error.clone(),
        })
    }

    /// 当前连接指向的 ACP 会话 id，用来把列表里那一条标成「正在用」。
    pub(crate) fn acp_session_id_snapshot(&self) -> Option<String> {
        self.acp.as_ref().map(|acp| acp.session_id().to_string())
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
            || self.acp_turn_owner.is_some()
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
        result: anyhow::Result<Vec<agent_client_protocol::schema::SessionInfo>>,
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
        if !self.acp_session_list_visible() || self.is_running || self.acp_turn_owner.is_some() {
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
        let cwd = self
            .acp_sessions
            .iter()
            .find(|session| session.id == acp_session_id)
            .map(|session| session.cwd.clone())
            .unwrap_or_else(|| self.workspace_root.clone());
        self.acp_turn_owner = None;
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
                self.clear_acp_session_transition(operation);
                let receiver = acp.subscribe();
                let session_id = acp.session_id();
                self.acp = Some(acp);
                self.acp_turn_owner = None;
                self._event_task = Self::spawn_event_pump(receiver, Some(session_id), cx);
                self.transcript.clear_acp_status();
                self.acp_sessions_error = None;
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
