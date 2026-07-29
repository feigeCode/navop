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
use crate::tools::{ToolCall, ToolObservation};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[path = "session_turns.rs"]
mod turns;
pub(crate) use turns::PendingToolResolution;
use turns::TurnState;

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
}

/// 一次会话。
pub struct Session {
    id: SessionId,
    state: Mutex<SessionState>,
    resources: Mutex<ResourceContext>,
    skills: Mutex<SkillContext>,
    input_queue: Mutex<InputQueue>,
    turns: Mutex<TurnState>,
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
}

#[cfg(test)]
#[path = "session_cancellation_tests.rs"]
mod cancellation_tests;
