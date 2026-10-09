//! 会话:运行时的核心状态容器。
//!
//! 以 `Arc<Session>` 在 Runtime 与任务间共享,内部用 `Mutex` 保护可变状态。
//! 所有写状态的方法同时负责发出对应的 [`RuntimeEvent`](crate::runtime::RuntimeEvent),
//! 对齐 Codex 中"history + 事件一起处理"的做法。

use crate::history::{HistoryItem, RuntimeHistory};
use crate::ids::{SessionId, SubAgentId, TurnId};
use crate::planner::{Plan, StepStatus};
use crate::resource::ResourceContext;
use crate::runtime::active_turn::ActiveTurn;
use crate::runtime::event::{RuntimeEvent, RuntimeEventSender};
use crate::runtime::input_queue::{InputQueue, TurnInput};
use crate::runtime::session_state::SessionState;
use crate::skill::SkillContext;
use crate::tools::{ToolAction, ToolCall, ToolObservation};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[path = "session_turns.rs"]
mod turns;
pub(crate) use turns::PendingToolResolution;
use turns::TurnState;

/// 「这条会话由外部 agent 承载」的地址。
///
/// 外部 agent 自己存历史，navop 这边只记「这个会话指的是它哪一条协议会话」，
/// 用来把会话留在侧栏列表里，并在重新打开时接回同一段对话，而不是新开一条。
/// 会话的工作目录不在这里：它是外壳的属性，连接时由当前工作区决定。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpSessionRef {
    /// 外部 agent 的 id（配置里的稳定标识）。
    pub agent_id: String,
    /// 该 agent 侧的协议会话 id（能拿去 `session/load` / `session/resume` 的地址）。
    pub session_id: String,
}

/// 会话的可持久化快照:足以重建一个 [`Session`] 全部对话状态的最小集合。
///
/// 只包含可序列化的对话事实(标识、资源、历史、当前计划),**不含**运行时瞬态
/// (事件通道、输入队列、当前轮)。用于会话持久化:落盘前 [`Session::snapshot`],
/// 重启后 [`Session::restore`]。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub id: SessionId,
    #[serde(default)]
    pub resources: ResourceContext,
    #[serde(default)]
    pub history: Vec<HistoryItem>,
    #[serde(default)]
    pub plan: Option<Plan>,
    #[serde(default)]
    pub system_instruction: Option<String>,
    #[serde(default)]
    pub skills: SkillContext,
    /// 会话创建时所在的工作区根目录（规范化字符串）。
    ///
    /// 会话归属在工作区**首次落盘时定格**，之后不随外壳切换工作区而改变；
    /// 旧快照没有该字段，反序列化为 `None`（侧栏归入「未分组」）。
    #[serde(default)]
    pub workspace_root: Option<String>,
    /// 用户在输入框里**尚未发送**的文字（会话草稿）。
    ///
    /// 切换会话时由视图捕获/恢复；随快照落盘，重启后回到对应会话仍能看到。
    /// 空白会话不落盘（持久化层跳过空历史快照），所以空白会话的草稿只在
    /// 本次进程内有效——这是刻意的：navop 的空白会话是瞬态的，不进列表。
    #[serde(default)]
    pub draft: Option<String>,
    /// 会话当前的上下文占用估算(token 数,取最近一次模型请求的输入+输出)。
    ///
    /// provider 不报告计量时保持 `None`(没有测过就别装测过);切换模型后
    /// 数字会被下一次请求自然覆盖。窗口大小不在这里——它取决于当前模型,
    /// 由展示层按模型名解析,快照只存「用了多少」这个事实。
    #[serde(default)]
    pub context_tokens: Option<u64>,
    /// 会话是否由外部 agent 承载（见 [`AcpSessionRef`]）。
    ///
    /// `Some` 时本地 `history` 可能为空——历史在 agent 那边，本地只留这个地址；
    /// 旧快照没有该字段，反序列化为 `None`（纯本地会话）。
    #[serde(default)]
    pub acp: Option<AcpSessionRef>,
}

/// 一次会话。
pub struct Session {
    id: SessionId,
    state: Mutex<SessionState>,
    resources: Mutex<ResourceContext>,
    skills: Mutex<SkillContext>,
    input_queue: Mutex<InputQueue>,
    turns: Mutex<TurnState>,
    draft: Mutex<Option<String>>,
    context_tokens: Mutex<Option<u64>>,
    /// 会话的外部 agent 地址（纯载体：运行时不用它，只负责让它随快照往返，
    /// 否则一次本地落盘就会把「这条会话属于哪个 agent」抹掉）。
    acp: Mutex<Option<AcpSessionRef>>,
    events: RuntimeEventSender,
}

impl Session {
    pub fn new(id: SessionId, resources: ResourceContext, events: RuntimeEventSender) -> Arc<Self> {
        Arc::new(Self {
            id,
            state: Mutex::new(SessionState::new()),
            resources: Mutex::new(resources),
            skills: Mutex::new(SkillContext::new()),
            input_queue: Mutex::new(InputQueue::new()),
            turns: Mutex::new(TurnState::default()),
            draft: Mutex::new(None),
            context_tokens: Mutex::new(None),
            acp: Mutex::new(None),
            events,
        })
    }

    /// 由持久化快照重建会话。共享传入的事件通道(与同一 Runtime 的其他会话一致),
    /// 运行时瞬态(输入队列 / 当前轮)重置为初始值。
    pub fn restore(snapshot: SessionSnapshot, events: RuntimeEventSender) -> Arc<Self> {
        let state = SessionState {
            history: RuntimeHistory::from_items(snapshot.history),
            current_plan: snapshot.plan,
            system_instruction: snapshot.system_instruction,
            last_error: None,
        };
        Arc::new(Self {
            id: snapshot.id,
            state: Mutex::new(state),
            resources: Mutex::new(snapshot.resources),
            skills: Mutex::new(snapshot.skills),
            input_queue: Mutex::new(InputQueue::new()),
            turns: Mutex::new(TurnState::default()),
            draft: Mutex::new(snapshot.draft),
            context_tokens: Mutex::new(snapshot.context_tokens),
            acp: Mutex::new(snapshot.acp),
            events,
        })
    }

    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// 生成会话的可持久化快照(历史 + 当前计划 + 资源 + 标识)。
    pub fn snapshot(&self) -> SessionSnapshot {
        let (history, plan, system_instruction) = {
            let state = self.state.lock().expect("session 锁中毒");
            (
                state.history.items().to_vec(),
                state.current_plan.clone(),
                state.system_instruction.clone(),
            )
        };
        SessionSnapshot {
            id: self.id.clone(),
            resources: self.resources(),
            skills: self.skills(),
            history,
            plan,
            system_instruction,
            // 会话归属由持久化层在落盘时按「首存定格」规则补写，见
            // ai_chat_view::persistence::save_session_with_workspace。
            workspace_root: None,
            draft: self.draft(),
            context_tokens: self.context_tokens(),
            acp: self.acp_ref(),
        }
    }

    // ===== 资源 =====

    pub fn resources(&self) -> ResourceContext {
        self.resources.lock().expect("session 锁中毒").clone()
    }

    pub fn set_resources(&self, resources: ResourceContext) {
        *self.resources.lock().expect("session 锁中毒") = resources;
    }

    pub fn skills(&self) -> SkillContext {
        self.skills.lock().expect("session 锁中毒").clone()
    }

    pub fn set_skills(&self, skills: SkillContext) {
        *self.skills.lock().expect("session 锁中毒") = skills;
    }

    // ===== 状态快照 =====

    pub fn history_snapshot(&self) -> RuntimeHistory {
        self.state.lock().expect("session 锁中毒").history.clone()
    }

    pub fn compact_history(&self, summary: impl Into<String>, keep_last_items: usize) -> bool {
        self.state
            .lock()
            .expect("session 锁中毒")
            .history
            .compact_old_items(summary, keep_last_items)
    }

    pub fn current_plan(&self) -> Option<Plan> {
        self.state
            .lock()
            .expect("session 锁中毒")
            .current_plan
            .clone()
    }

    pub fn system_instruction(&self) -> Option<String> {
        self.state
            .lock()
            .expect("session 锁中毒")
            .system_instruction
            .clone()
    }

    pub fn set_system_instruction(&self, instruction: Option<String>) {
        let instruction = instruction.and_then(|text| {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
        self.state
            .lock()
            .expect("session 锁中毒")
            .system_instruction = instruction;
    }

    // ===== 输入草稿 =====

    /// 当前未发送的输入草稿；`None` 表示没有。
    ///
    /// 纯空白的草稿被视为没有草稿（与「空输入框」同义），避免切走再切回
    /// 后输入框凭空多出一段不可见的空白。
    pub fn draft(&self) -> Option<String> {
        self.draft
            .lock()
            .expect("session 锁中毒")
            .clone()
            .filter(|text| !text.trim().is_empty())
    }

    /// 覆盖输入草稿。传 `None` 或纯空白即清除。
    pub fn set_draft(&self, draft: Option<String>) {
        *self.draft.lock().expect("session 锁中毒") = draft.filter(|text| !text.trim().is_empty());
    }

    /// 记录一次模型采样后的上下文占用(输入 + 输出 token,见
    /// [`TokenUsage::context_tokens`])。
    ///
    /// 覆盖写:每次请求的 prompt 就是那之后的上下文基线,历史值无需保留。
    /// 多次调用幂等——`Usage` 事件与 `Completed` 各记一次是正常情况。
    pub fn record_token_usage(&self, usage: crate::model::TokenUsage) {
        let tokens = usage.context_tokens();
        *self.context_tokens.lock().expect("session 锁中毒") = Some(tokens);
    }

    /// 当前上下文占用估算;provider 从未报告过计量时为 `None`。
    pub fn context_tokens(&self) -> Option<u64> {
        *self.context_tokens.lock().expect("session 锁中毒")
    }

    /// 外部 agent 报的上下文占用直接写入（ACP 的 `session/update` UsageUpdate）。
    ///
    /// 与 [`Self::record_token_usage`] 分开：那个从模型采样计量取值，这个的来源
    /// 是 agent 推送的快照，语义都是「当前上下文占用了多少」，落同一个字段。
    pub fn set_context_tokens(&self, tokens: Option<u64>) {
        *self.context_tokens.lock().expect("session 锁中毒") = tokens;
    }

    /// 会话当前指向的外部 agent 会话地址（没有就是本地会话）。
    pub fn acp_ref(&self) -> Option<AcpSessionRef> {
        self.acp.lock().expect("session 锁中毒").clone()
    }

    /// 记下 / 清掉外部 agent 会话地址。
    pub fn set_acp_ref(&self, reference: Option<AcpSessionRef>) {
        *self.acp.lock().expect("session 锁中毒") = reference;
    }

    pub fn set_last_error(&self, error: Option<String>) {
        self.state.lock().expect("session 锁中毒").last_error = error;
    }

    // ===== 历史记录 + 事件 =====

    pub fn record_user_input(&self, text: impl Into<String>) {
        self.state
            .lock()
            .expect("session 锁中毒")
            .history
            .record_user(text);
    }

    /// 记录一条带图片的用户输入(多模态)。
    pub fn record_user_input_with_images(
        &self,
        text: impl Into<String>,
        images: Vec<crate::runtime::InputImage>,
    ) {
        self.state
            .lock()
            .expect("session 锁中毒")
            .history
            .record_user_with_images(text, images);
    }

    pub fn record_assistant_message(&self, turn_id: &TurnId, text: impl Into<String>) {
        self.record_assistant_message_with_reasoning(turn_id, text, "");
    }

    pub fn record_assistant_message_with_reasoning(
        &self,
        turn_id: &TurnId,
        text: impl Into<String>,
        reasoning: impl Into<String>,
    ) {
        let text = text.into();
        let reasoning = reasoning.into();
        let _ = self.with_writable_turn(turn_id, || {
            self.state
                .lock()
                .expect("session 锁中毒")
                .history
                .record_assistant_with_reasoning(text.clone(), reasoning);
            self.emit(RuntimeEvent::AssistantMessage {
                session_id: self.id.clone(),
                turn_id: turn_id.clone(),
                text,
            });
        });
    }

    /// 发出一段助手文本增量(流式)。增量不写入历史,最终由
    /// [`Session::record_assistant_message`] 落历史并发完整消息。
    pub fn emit_assistant_delta(&self, turn_id: &TurnId, delta: impl Into<String>) {
        let delta = delta.into();
        let _ = self.with_writable_turn(turn_id, || {
            self.emit(RuntimeEvent::AssistantMessageDelta {
                session_id: self.id.clone(),
                turn_id: turn_id.clone(),
                delta,
            });
        });
    }

    /// 发出一段思考增量。增量不写入历史,只用于 UI 折叠展示。
    pub fn emit_reasoning_delta(&self, turn_id: &TurnId, delta: impl Into<String>) {
        let delta = delta.into();
        let _ = self.with_writable_turn(turn_id, || {
            self.emit(RuntimeEvent::ReasoningDelta {
                session_id: self.id.clone(),
                turn_id: turn_id.clone(),
                delta,
            });
        });
    }

    pub fn record_tool_call(&self, turn_id: &TurnId, call: &ToolCall) {
        let _ = self.with_writable_turn(turn_id, || {
            self.state
                .lock()
                .expect("session 锁中毒")
                .history
                .record_tool_call(call.clone());
            self.emit(RuntimeEvent::ToolCallStarted {
                session_id: self.id.clone(),
                turn_id: turn_id.clone(),
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                kind: ToolAction::from_tool_name(call.tool_name.as_str()),
                arguments: call.arguments.clone(),
            });
        });
    }

    pub fn record_observation(&self, turn_id: &TurnId, observation: ToolObservation) {
        let call_id = observation.call_id.clone();
        let success = observation.success;
        let _ = self.with_writable_turn(turn_id, || {
            self.state
                .lock()
                .expect("session 锁中毒")
                .history
                .record_observation(observation.clone());
            self.emit(RuntimeEvent::ObservationAdded {
                session_id: self.id.clone(),
                turn_id: turn_id.clone(),
                observation,
            });
            self.emit(RuntimeEvent::ToolCallFinished {
                session_id: self.id.clone(),
                turn_id: turn_id.clone(),
                call_id,
                success,
            });
        });
    }

    pub fn start_subagent(
        &self,
        turn_id: &TurnId,
        subagent_id: SubAgentId,
        name: impl Into<String>,
        task: impl Into<String>,
    ) {
        let name = name.into();
        let task = task.into();
        let _ = self.with_writable_turn(turn_id, || {
            self.emit(RuntimeEvent::SubAgentStarted {
                session_id: self.id.clone(),
                turn_id: turn_id.clone(),
                subagent_id,
                name,
                task,
            });
        });
    }

    pub fn update_subagent(
        &self,
        turn_id: &TurnId,
        subagent_id: SubAgentId,
        summary: impl Into<String>,
    ) {
        let summary = summary.into();
        let _ = self.with_writable_turn(turn_id, || {
            self.emit(RuntimeEvent::SubAgentUpdated {
                session_id: self.id.clone(),
                turn_id: turn_id.clone(),
                subagent_id,
                summary,
            });
        });
    }

    pub fn finish_subagent(
        &self,
        turn_id: &TurnId,
        subagent_id: SubAgentId,
        success: bool,
        summary: impl Into<String>,
    ) {
        let summary = summary.into();
        let _ = self.with_writable_turn(turn_id, || {
            self.emit(RuntimeEvent::SubAgentFinished {
                session_id: self.id.clone(),
                turn_id: turn_id.clone(),
                subagent_id,
                success,
                summary,
            });
        });
    }

    // ===== 计划 =====

    pub fn update_plan(&self, turn_id: &TurnId, plan: Plan) {
        let _ = self.with_writable_turn(turn_id, || {
            self.state.lock().expect("session 锁中毒").current_plan = Some(plan.clone());
            self.emit(RuntimeEvent::PlanUpdated {
                session_id: self.id.clone(),
                turn_id: turn_id.clone(),
                plan,
            });
        });
    }

    /// 更新某计划步骤状态(若存在当前计划),并发出 [`RuntimeEvent::PlanUpdated`]。
    ///
    /// 仅在步骤确实被更新时发事件;先在锁内改完并克隆出新计划,再在锁外 emit,
    /// 与 [`Session::update_plan`] 保持一致(避免持锁发送事件)。
    pub fn mark_step(
        &self,
        turn_id: &TurnId,
        step_id: &crate::ids::PlanStepId,
        status: StepStatus,
    ) {
        let _ = self.with_writable_turn(turn_id, || {
            let plan = {
                let mut state = self.state.lock().expect("session 锁中毒");
                let plan = state.current_plan.as_mut()?;
                plan.mark_step(step_id, status).then(|| plan.clone())?
            };
            self.emit(RuntimeEvent::PlanUpdated {
                session_id: self.id.clone(),
                turn_id: turn_id.clone(),
                plan,
            });
            Some(())
        });
    }

    // ===== 输入队列 =====

    pub fn queue_input(&self, input: TurnInput) {
        self.input_queue.lock().expect("session 锁中毒").push(input);
    }

    pub fn take_pending_inputs(&self) -> Vec<TurnInput> {
        self.input_queue.lock().expect("session 锁中毒").drain()
    }

    pub fn has_pending_input(&self) -> bool {
        self.input_queue
            .lock()
            .expect("session 锁中毒")
            .has_pending()
    }

    // ===== 事件 =====

    /// 发出一个事件。无订阅者时静默忽略。
    pub fn emit(&self, event: RuntimeEvent) {
        let _ = self.events.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::PlanStepId;
    use crate::planner::{PlanSource, PlanStep};

    fn test_session() -> (Arc<Session>, tokio::sync::broadcast::Receiver<RuntimeEvent>) {
        let (tx, rx) = tokio::sync::broadcast::channel(16);
        let session = Session::new(
            SessionId::from_string("sess_test"),
            ResourceContext::new(),
            tx,
        );
        (session, rx)
    }

    #[test]
    fn mark_step_emits_plan_updated_with_new_status() {
        let (session, mut rx) = test_session();
        let turn_id = TurnId::from_string("turn_test");

        let step = PlanStep::new("查看连接数", "SHOW PROCESSLIST");
        let step_id = step.id.clone();
        let plan = Plan::new("排查慢查询", PlanSource::Llm).with_steps(vec![step]);

        session.update_plan(&turn_id, plan);
        // 消费 update_plan 发出的初始 PlanUpdated。
        assert!(matches!(
            rx.try_recv(),
            Ok(RuntimeEvent::PlanUpdated { .. })
        ));

        // 推进步骤状态后必须再次发出 PlanUpdated(本次修复的核心回归点)。
        session.mark_step(&turn_id, &step_id, StepStatus::Completed);
        match rx.try_recv() {
            Ok(RuntimeEvent::PlanUpdated { plan, .. }) => {
                assert_eq!(plan.steps[0].status, StepStatus::Completed);
            }
            other => panic!("期望 mark_step 发出 PlanUpdated,实际:{other:?}"),
        }
    }

    #[test]
    fn mark_step_without_plan_emits_nothing() {
        let (session, mut rx) = test_session();
        let turn_id = TurnId::from_string("turn_test");
        // 无当前计划:静默返回,不发事件。
        session.mark_step(&turn_id, &PlanStepId::new(), StepStatus::Completed);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn mark_step_unknown_step_emits_nothing() {
        let (session, mut rx) = test_session();
        let turn_id = TurnId::from_string("turn_test");
        let plan = Plan::new("目标", PlanSource::Llm).with_steps(vec![PlanStep::new("step", "")]);
        session.update_plan(&turn_id, plan);
        let _ = rx.try_recv(); // 丢弃初始事件。
        // 未知 step_id:plan.mark_step 返回 false,不应再发事件。
        session.mark_step(&turn_id, &PlanStepId::new(), StepStatus::Completed);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn record_tool_call_emits_arguments() {
        use crate::tools::{ToolCall, ToolName};

        let (session, mut rx) = test_session();
        let turn_id = TurnId::from_string("turn_test");
        let call = ToolCall::new(
            ToolName::new("exec_command"),
            serde_json::json!({"command": "rtk cargo check"}),
        );

        session.record_tool_call(&turn_id, &call);

        match rx.try_recv() {
            Ok(RuntimeEvent::ToolCallStarted { arguments, .. }) => {
                assert_eq!(arguments["command"], "rtk cargo check");
            }
            other => panic!("期望 ToolCallStarted 携带 arguments,实际:{other:?}"),
        }
    }

    #[test]
    fn snapshot_round_trips_through_json_and_restores() {
        use crate::tools::{ObservationData, ToolCall, ToolName, ToolObservation};

        let (session, _rx) = test_session();
        let turn_id = TurnId::from_string("turn_test");

        // 构造一段有代表性的历史:用户、助手、工具调用 + 观测。
        session.set_system_instruction(Some("始终用 DBA 视角回答。".into()));
        session.record_user_input("查询连接数");
        session.record_assistant_message(&turn_id, "好的,我来查询");
        let call = ToolCall::new(ToolName::new("echo"), serde_json::json!({"text": "hi"}));
        let call_id = call.call_id.clone();
        session.record_tool_call(&turn_id, &call);
        session.record_observation(
            &turn_id,
            ToolObservation::success(
                call_id,
                ToolName::new("echo"),
                "echo: hi",
                ObservationData::Text("hi".into()),
            ),
        );
        let plan = Plan::new("查询连接数", PlanSource::Llm)
            .with_steps(vec![PlanStep::new("执行查询", "echo")]);
        session.update_plan(&turn_id, plan);

        // 快照 -> JSON -> 快照,再恢复成新会话。
        let snapshot = session.snapshot();
        assert_eq!(snapshot.history.len(), 4);
        let json = serde_json::to_string(&snapshot).expect("快照应可序列化为 JSON");
        let parsed: SessionSnapshot = serde_json::from_str(&json).expect("JSON 应可反序列化回快照");

        let (tx, _rx2) = tokio::sync::broadcast::channel(16);
        let restored = Session::restore(parsed, tx);

        assert_eq!(restored.id(), session.id());
        assert_eq!(
            restored.system_instruction().as_deref(),
            Some("始终用 DBA 视角回答。")
        );
        assert_eq!(restored.history_snapshot().len(), 4);
        let restored_plan = restored.current_plan().expect("应恢复出当前计划");
        assert_eq!(restored_plan.goal, "查询连接数");
        assert_eq!(restored_plan.steps.len(), 1);
    }

    #[test]
    fn external_agent_reference_survives_the_local_save_path() {
        // 外部 agent 会话由它自己存历史，本地只有地址。若 `snapshot()` 不带这个地址，
        // 一次本地落盘（切走会话时就会发生）就会把会话变回「纯本地」，
        // 侧栏那行也就再也接不回原来的对话。
        let (session, _rx) = test_session();
        assert!(session.acp_ref().is_none());

        session.set_acp_ref(Some(AcpSessionRef {
            agent_id: "agent-1".into(),
            session_id: "acp-42".into(),
        }));
        let snapshot = session.snapshot();
        assert_eq!(
            Some("acp-42".to_string()),
            snapshot.acp.as_ref().map(|acp| acp.session_id.clone())
        );

        let json = serde_json::to_string(&snapshot).expect("快照应可序列化为 JSON");
        let parsed: SessionSnapshot = serde_json::from_str(&json).expect("JSON 应可反序列化回快照");
        let (tx, _rx2) = tokio::sync::broadcast::channel(16);
        let restored = Session::restore(parsed, tx);
        assert_eq!(
            restored.acp_ref().map(|acp| acp.agent_id),
            Some("agent-1".to_string())
        );

        // 清掉之后不再随快照回来（切回本地也可以显式解绑）。
        restored.set_acp_ref(None);
        assert!(restored.snapshot().acp.is_none());
    }

    #[test]
    fn draft_round_trips_through_snapshot_and_restores() {
        let (session, _rx) = test_session();
        // 新会话没有草稿。
        assert_eq!(session.draft(), None);

        // 草稿不 trim 内容（打字中途的前导空白可能是刻意的），只把纯空白归一为无草稿。
        session.set_draft(Some("  帮我查一下连接数  ".into()));
        assert_eq!(session.draft().as_deref(), Some("  帮我查一下连接数  "));
        session.set_draft(Some("   ".into()));
        assert_eq!(session.draft(), None);
        session.set_draft(None);
        assert_eq!(session.draft(), None);

        session.set_draft(Some("未发送的文字".into()));
        let snapshot = session.snapshot();
        assert_eq!(snapshot.draft.as_deref(), Some("未发送的文字"));
        let json = serde_json::to_string(&snapshot).expect("快照应可序列化为 JSON");
        let parsed: SessionSnapshot = serde_json::from_str(&json).expect("JSON 应可反序列化回快照");
        let (tx, _rx2) = tokio::sync::broadcast::channel(16);
        let restored = Session::restore(parsed, tx);
        assert_eq!(restored.draft().as_deref(), Some("未发送的文字"));
    }

    #[test]
    fn legacy_snapshot_without_draft_field_deserializes_to_none() {
        let json = r#"{"id":"sess_x","history":[]}"#;
        let parsed: SessionSnapshot =
            serde_json::from_str(json).expect("旧快照(无 draft 字段)应可反序列化");
        assert_eq!(parsed.draft, None);
    }

    #[test]
    fn context_tokens_round_trip_through_snapshot_and_restore() {
        let (session, _rx) = test_session();
        // 没测过就是没测过:不是 0,是 None。
        assert_eq!(session.context_tokens(), None);

        session.record_token_usage(crate::model::TokenUsage {
            prompt_tokens: 1200,
            completion_tokens: 300,
            total_tokens: 1500,
        });
        assert_eq!(session.context_tokens(), Some(1500));

        // 覆盖写:下一次请求的计量取代旧值,Usage 事件与 Completed 重复记账无副作用。
        session.record_token_usage(crate::model::TokenUsage {
            prompt_tokens: 2000,
            completion_tokens: 100,
            total_tokens: 0, // 个别 provider 只填部分字段
        });
        assert_eq!(session.context_tokens(), Some(2100));

        let snapshot = session.snapshot();
        assert_eq!(snapshot.context_tokens, Some(2100));
        let json = serde_json::to_string(&snapshot).expect("快照应可序列化为 JSON");
        let parsed: SessionSnapshot = serde_json::from_str(&json).expect("JSON 应可反序列化回快照");
        let (tx, _rx2) = tokio::sync::broadcast::channel(16);
        let restored = Session::restore(parsed, tx);
        assert_eq!(restored.context_tokens(), Some(2100));
    }

    #[test]
    fn legacy_snapshot_without_context_tokens_field_deserializes_to_none() {
        let json = r#"{"id":"sess_x","history":[]}"#;
        let parsed: SessionSnapshot = serde_json::from_str(json).expect("旧快照应可反序列化");
        assert_eq!(parsed.context_tokens, None);
    }
}

#[cfg(test)]
#[path = "session_cancellation_tests.rs"]
mod cancellation_tests;
