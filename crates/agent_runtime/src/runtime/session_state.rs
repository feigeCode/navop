//! 会话级可变状态。

use crate::history::RuntimeHistory;
use crate::planner::Plan;

/// 会话持久状态:历史、当前计划、系统提示与最近错误。
pub struct SessionState {
    pub history: RuntimeHistory,
    pub current_plan: Option<Plan>,
    pub system_instruction: Option<String>,
    pub last_error: Option<String>,
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            history: RuntimeHistory::new(),
            current_plan: None,
            system_instruction: None,
            last_error: None,
        }
    }

    pub fn with_history(history: RuntimeHistory) -> Self {
        Self {
            history,
            current_plan: None,
            system_instruction: None,
            last_error: None,
        }
    }
}

impl Default for SessionState {
    fn default() -> Self {
        Self::new()
    }
}
