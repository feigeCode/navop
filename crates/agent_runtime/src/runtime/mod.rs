//! Runtime 外壳:对应 Codex 的 `Codex`。
//!
//! 持有会话管理器、共享服务([`RuntimeServices`])与事件广播端,负责接收命令、
//! 创建会话、启动 / 中断每一轮任务,并把任务结果转换为终态事件。

mod active_turn;
mod command;
mod event;
mod input_queue;
mod session;
mod session_manager;
mod session_state;
mod task;
mod turn_context;

pub use active_turn::ActiveTurn;
pub use command::RuntimeCommand;
pub use event::{RuntimeEvent, RuntimeEventReceiver, RuntimeEventSender};
pub use input_queue::{InputImage, InputQueue, TurnInput, UserInput};
pub use session::Session;
pub use session::SessionSnapshot;
pub use session_manager::SessionManager;
pub use session_state::SessionState;
pub use task::{RuntimeTask, TaskContext, TaskKind, TaskOutcome};
pub use turn_context::TurnContext;

use crate::error::RuntimeError;
use crate::ids::{SessionId, TurnId};
use crate::model::ModelClient;
use crate::resource::ResourceContext;
use crate::tasks::AgentTask;
use crate::tools::ToolRouter;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Runtime 依赖的共享服务。
#[derive(Clone)]
pub struct RuntimeServices {
    pub model: Arc<dyn ModelClient>,
    pub tools: Arc<ToolRouter>,
}

impl RuntimeServices {
    pub fn new(model: Arc<dyn ModelClient>, tools: Arc<ToolRouter>) -> Self {
        Self { model, tools }
    }
}

/// 事件广播通道容量。
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Agent 运行时。
pub struct Runtime {
    sessions: SessionManager,
    services: Arc<RuntimeServices>,
    events: RuntimeEventSender,
}

impl Runtime {
    pub fn new(services: RuntimeServices) -> Self {
        let (events, _rx) = tokio::sync::broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            sessions: SessionManager::new(),
            services: Arc::new(services),
            events,
        }
    }

    pub fn services(&self) -> &Arc<RuntimeServices> {
        &self.services
    }

    /// 订阅运行时事件。
    pub fn subscribe(&self) -> RuntimeEventReceiver {
        self.events.subscribe()
    }

    /// 创建一个新会话。
    pub fn create_session(&self, resources: ResourceContext) -> Arc<Session> {
        let session = Session::new(SessionId::new(), resources, self.events.clone());
        self.sessions.insert(session.clone());
        session
    }

    /// 由持久化快照恢复一个会话并登记到管理器。若同 ID 会话已存在则覆盖。
    pub fn restore_session(&self, snapshot: SessionSnapshot) -> Arc<Session> {
        let session = Session::restore(snapshot, self.events.clone());
        self.sessions.insert(session.clone());
        session
    }

    pub fn session(&self, id: &SessionId) -> Option<Arc<Session>> {
        self.sessions.get(id)
    }

    pub fn close_session(&self, id: &SessionId) {
        if let Some(session) = self.sessions.remove(id) {
            session.cancel_active_turn();
        }
    }

    fn build_task(kind: TaskKind) -> Arc<dyn RuntimeTask> {
        match kind {
            TaskKind::Agent | TaskKind::Ask | TaskKind::Plan => Arc::new(AgentTask::new()),
        }
    }

    /// 内联执行一轮并返回结果。适合测试与同步调用方。
    pub async fn run_turn_blocking(
        &self,
        session_id: &SessionId,
        input: UserInput,
        kind: TaskKind,
    ) -> Result<TaskOutcome, RuntimeError> {
        let session = self
            .session(session_id)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        if session.is_busy() {
            return Err(RuntimeError::SessionBusy(session_id.clone()));
        }

        let turn = Arc::new(TurnContext::new(session_id.clone(), session.resources()));
        let cancellation = CancellationToken::new();
        session.set_active_turn(ActiveTurn::new(
            turn.turn_id.clone(),
            cancellation.clone(),
            None,
        ));
        session.emit(RuntimeEvent::TurnStarted {
            session_id: session_id.clone(),
            turn_id: turn.turn_id.clone(),
        });

        let task = Self::build_task(kind);
        let ctx = TaskContext {
            kind,
            session: session.clone(),
            services: self.services.clone(),
            turn: turn.clone(),
            input: vec![TurnInput::User(input)],
        };
        let outcome = task.run(ctx, cancellation).await;

        session.clear_active_turn();
        emit_outcome(&session, &turn.turn_id, &outcome);
        Ok(outcome)
    }

    /// 在后台启动一轮任务,返回该轮 ID。结果通过事件流观测。
    pub fn start_turn(
        &self,
        session_id: &SessionId,
        input: UserInput,
        kind: TaskKind,
    ) -> Result<TurnId, RuntimeError> {
        let session = self
            .session(session_id)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        if session.is_busy() {
            return Err(RuntimeError::SessionBusy(session_id.clone()));
        }

        let turn = Arc::new(TurnContext::new(session_id.clone(), session.resources()));
        let turn_id = turn.turn_id.clone();
        let cancellation = CancellationToken::new();
        // 先登记当前轮,避免任务快速结束时清理早于登记造成的竞态。
        session.set_active_turn(ActiveTurn::new(turn_id.clone(), cancellation.clone(), None));
        session.emit(RuntimeEvent::TurnStarted {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
        });

        let task = Self::build_task(kind);
        let services = self.services.clone();
        let session_run = session.clone();
        let turn_run = turn.clone();
        let cancel_run = cancellation.clone();
        tokio::spawn(async move {
            let ctx = TaskContext {
                kind,
                session: session_run.clone(),
                services,
                turn: turn_run.clone(),
                input: vec![TurnInput::User(input)],
            };
            let outcome = task.run(ctx, cancel_run).await;
            session_run.clear_active_turn();
            emit_outcome(&session_run, &turn_run.turn_id, &outcome);
        });

        Ok(turn_id)
    }

    /// 中断会话当前正在执行的一轮。
    pub fn interrupt(&self, session_id: &SessionId) -> Result<(), RuntimeError> {
        let session = self
            .session(session_id)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        session.cancel_active_turn();
        Ok(())
    }

    /// 处理一条 [`RuntimeCommand`]。
    pub async fn submit(&self, command: RuntimeCommand) -> Result<(), RuntimeError> {
        match command {
            RuntimeCommand::StartTurn {
                session_id,
                input,
                task_kind,
            } => {
                self.start_turn(&session_id, input, task_kind)?;
                Ok(())
            }
            RuntimeCommand::Interrupt { session_id } => self.interrupt(&session_id),
            RuntimeCommand::CloseSession { session_id } => {
                self.close_session(&session_id);
                Ok(())
            }
        }
    }
}

/// 把任务结果转换为终态事件并发出。
fn emit_outcome(session: &Session, turn_id: &TurnId, outcome: &TaskOutcome) {
    let session_id = session.id().clone();
    let event = match outcome {
        TaskOutcome::Completed { answer } => RuntimeEvent::TurnCompleted {
            session_id,
            turn_id: turn_id.clone(),
            answer: answer.clone(),
        },
        TaskOutcome::NeedUserInput { question } => RuntimeEvent::NeedUserInput {
            session_id,
            turn_id: turn_id.clone(),
            question: question.clone(),
        },
        TaskOutcome::Failed { reason } => RuntimeEvent::TurnFailed {
            session_id,
            turn_id: turn_id.clone(),
            reason: reason.clone(),
        },
        TaskOutcome::Cancelled => RuntimeEvent::TurnFailed {
            session_id,
            turn_id: turn_id.clone(),
            reason: "任务已取消".to_string(),
        },
    };
    session.emit(event);
}
