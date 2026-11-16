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

use std::any::Any;
use std::sync::Arc;

use gpui::App;
use serde_json::Value;

/// 主线程任务的返回值：装箱成 `Any`，交给调用方 downcast。
///
/// 绝大多数调用只想要一段 JSON（[`MainThread::run`]），但有些任务要带回
/// 非 JSON 的句柄（比如终端注册表），所以这里不把返回类型钉死在 `Value`。
type AnyValue = Box<dyn Any + Send>;

/// 主线程任务：在主线程上执行，返回装箱结果或一句错误。
type Job = Box<dyn FnOnce(&mut App) -> Result<AnyValue, String> + Send>;

struct Queue {
    sender: tokio::sync::mpsc::UnboundedSender<(
        Job,
        tokio::sync::oneshot::Sender<Result<AnyValue, String>>,
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
            tokio::sync::oneshot::Sender<Result<AnyValue, String>>,
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

    /// 在主线程上跑一段任务，等结果（JSON 版本）。
    pub(crate) async fn run(
        &self,
        job: impl FnOnce(&mut App) -> Result<Value, String> + Send + 'static,
    ) -> Result<Value, String> {
        self.run_with(job).await
    }

    /// 在主线程上跑一段任务，把任意 `Send` 返回值带回来。
    ///
    /// 用来取只在主线程可读的 GPUI 全局（终端注册表这类句柄）：任务本身
    /// 只在主线程短暂执行，重活仍然留在 tokio 线程上。
    pub(crate) async fn run_with<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut App) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let job: Job =
            Box::new(move |cx| job(cx).map(|value| Box::new(value) as Box<dyn Any + Send>));
        let (done, wait) = tokio::sync::oneshot::channel();
        self.queue
            .sender
            .send((job, done))
            .map_err(|_| "主线程队列已关闭".to_string())?;
        let boxed = wait.await.map_err(|_| "主线程已关闭".to_string())??;
        boxed
            .downcast::<T>()
            .map(|value| *value)
            .map_err(|_| "主线程任务返回了意外的类型".to_string())
    }
}
