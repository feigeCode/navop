//! 运行时对内命令。
//!
//! 对应 Codex 的 `Submission` / `Op`。第一版 Runtime 也直接提供等价的 async
//! 方法([`Runtime::start_turn`](crate::runtime::Runtime::start_turn) 等),
//! [`RuntimeCommand`] 主要服务于未来从 channel / IPC 接收外部指令的场景。

use crate::ids::SessionId;
use crate::runtime::input_queue::UserInput;
use crate::runtime::task::TaskKind;

/// 提交给 Runtime 的命令。
pub enum RuntimeCommand {
    /// 启动一轮新交互。
    StartTurn {
        session_id: SessionId,
        input: UserInput,
        task_kind: TaskKind,
    },
    /// 中断会话当前正在执行的一轮。
    Interrupt { session_id: SessionId },
    /// 关闭并移除会话。
    CloseSession { session_id: SessionId },
}
