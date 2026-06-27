//! 当前正在执行的一轮。

use crate::ids::TurnId;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// 描述会话中正在运行的一轮任务,用于中断与状态追踪。
pub struct ActiveTurn {
    pub turn_id: TurnId,
    /// 取消令牌:调用 [`CancellationToken::cancel`] 即请求中断本轮。
    pub cancellation: CancellationToken,
    /// 后台任务句柄。内联执行(如测试)时为 `None`。
    pub handle: Option<JoinHandle<()>>,
}

impl ActiveTurn {
    pub fn new(
        turn_id: TurnId,
        cancellation: CancellationToken,
        handle: Option<JoinHandle<()>>,
    ) -> Self {
        Self {
            turn_id,
            cancellation,
            handle,
        }
    }

    /// 请求中断本轮。
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}
