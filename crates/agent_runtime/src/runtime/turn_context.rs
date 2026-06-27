//! 每轮交互的上下文快照。

use crate::ids::{SessionId, TurnId};
use crate::resource::ResourceContext;

/// 默认单轮最大步数,防止 plan-execute 循环失控。
const DEFAULT_MAX_STEPS: usize = 16;

/// 一轮交互的上下文。在 turn 开始时根据会话当时的状态生成快照。
#[derive(Clone)]
pub struct TurnContext {
    pub turn_id: TurnId,
    pub session_id: SessionId,
    /// 本轮使用的资源上下文快照。
    pub resources: ResourceContext,
    /// 本轮 plan-execute 循环允许的最大步数。
    pub max_steps: usize,
}

impl TurnContext {
    pub fn new(session_id: SessionId, resources: ResourceContext) -> Self {
        Self {
            turn_id: TurnId::new(),
            session_id,
            resources,
            max_steps: DEFAULT_MAX_STEPS,
        }
    }

    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps.max(1);
        self
    }
}
