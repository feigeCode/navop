//! 引擎线程 ←→ 主线程的单向桥。
//!
//! nsh 的页面跑在自己的 engine 线程上，而 `navop.*` 的 handler 也在那里被
//! 调用。凡是要碰 GPUI 全局（`DevExtensionRegistry`、窗口、文件选择器）的
//! 操作，都必须回到主线程执行 —— handler 直接读 App 会 panic 或读到
//! 半截状态。
//!
//! 这里只做一件事：把一段 `FnOnce(&mut App) -> Result<Value, String>` 投到
//! 主线程队列，等它的返回值。任务本身要求 `Send`，所以闭包里只能带
//! 简单数据（字符串、路径、JSON），不能带 `Rc`/`Entity` —— 那些只能在
//! 主线程上取。

use std::sync::Arc;

use gpui::App;
use serde_json::Value;

/// 主线程任务：在主线程上执行，返回 JSON 或一句错误。
type Job = Box<dyn FnOnce(&mut App) -> Result<Value, String> + Send>;

struct Queue {
    sender: tokio::sync::mpsc::UnboundedSender<(
        Job,
        tokio::sync::oneshot::Sender<Result<Value, String>>,
    )>,
}

/// 主线程队列的句柄（克隆成本极低，随 `NshHost` 共享）。
#[derive(Clone)]
pub(crate) struct MainThread {
    queue: Arc<Queue>,
}

impl MainThread {
    /// 建队列并挂上 drain 任务（`NshHost::init` 里调一次）。
    pub(crate) fn new(cx: &mut App) -> Self {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<(
            Job,
            tokio::sync::oneshot::Sender<Result<Value, String>>,
        )>();
        // drain 任务住在主线程上：它只在收到任务时醒来，空闲时是零开销。
        cx.spawn(async move |cx| {
            while let Some((job, done)) = receiver.recv().await {
                // 主线程已退出时 `update` 会拿到 Err：放弃应答，让等待方看到
                // "主线程已关闭"，而不是永远等下去。
                // `update` 直接返回闭包的结果（不再套一层 Result）：
                // 任务自己带回来的错误原样交给等待方。
                let _ = done.send(cx.update(job));
            }
        })
        .detach();
        Self {
            queue: Arc::new(Queue { sender }),
        }
    }

    /// 在主线程上跑一段任务，等结果。
    pub(crate) async fn run(
        &self,
        job: impl FnOnce(&mut App) -> Result<Value, String> + Send + 'static,
    ) -> Result<Value, String> {
        let (done, wait) = tokio::sync::oneshot::channel();
        self.queue
            .sender
            .send((Box::new(job), done))
            .map_err(|_| "主线程队列已关闭".to_string())?;
        wait.await.map_err(|_| "主线程已关闭".to_string())?
    }
}
