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

    /// 没有后台任务句柄的一轮（内联执行 / 测试里模拟「正在跑」）。
    ///
    /// `Session::steer` 只认 `active_turn_id()`，所以想让插话生效就得先有一轮在飞；
    /// 这个构造器把那步摊平，省得调用方为了拿一个 [`CancellationToken`] 去依赖
    /// `tokio_util`。
    pub fn detached(turn_id: TurnId) -> Self {
        Self::new(turn_id, CancellationToken::new(), None)
    }

    /// 请求中断本轮。
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}
