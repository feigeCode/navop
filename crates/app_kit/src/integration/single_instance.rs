//! 单实例门禁：同一产品只允许一个 GUI 进程，二次启动把请求转交给已运行的实例。
//!
//! 两个平台后端：
//!
//! - Windows（`windows`）：命名管道。沿用全量产品的实现（`FILE_FLAG_FIRST_PIPE_INSTANCE`
//!   做互斥、先补建下一实例再移交），并额外提供按 `app_id` 派生管道名的入口；
//! - macOS/Linux（`unix`）：`UnixListener` socket，二次启动 `connect` 后发 `SHOW` 与
//!   启动请求，旧进程收到后把主窗口带到前台；
//! - 其它平台：明确报错，不静默退化成「多实例」。
//!
//! 三个产品的 `app_id` 不同，因此 socket 路径 / 管道名天然隔离，互不干扰。

use std::path::PathBuf;

#[cfg(any(unix, windows))]
mod codec;
#[cfg(unix)]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub use windows::{claim_or_forward, pipe_name};

/// 转发请求里最多携带的路径数（命令行一次性打开几千个文件没有意义）。
pub const MAX_PATH_COUNT: usize = 256;
/// 单条路径的字节上限。
pub const MAX_PATH_BYTES: usize = 32 * 1024;
/// 整条请求的字节上限，防止对端用长度前缀把内存吃光。
pub const MAX_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

/// 二次启动要交给主实例的启动请求。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartupRequest {
    paths: Vec<PathBuf>,
}

impl StartupRequest {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// 取出待打开路径（主实例侧消费转发请求时用）。
    pub fn into_paths(self) -> Vec<PathBuf> {
        self.paths
    }
}

/// 门禁结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SingleInstanceOutcome {
    /// 本进程是主实例，继续启动。
    Primary,
    /// 已有实例在跑，请求已经交付，本进程应当干净退出。
    Forwarded,
}

/// 门禁失败。
#[derive(Debug)]
pub enum SingleInstanceError {
    /// 连接已有实例或写入请求失败。
    Forward(std::io::Error),
    /// 创建监听端失败。
    Listen(std::io::Error),
}

impl SingleInstanceError {
    /// 面向用户的中文提示：启动失败不是用户操作错误，要说清「怎么办」。
    pub fn user_message(&self) -> String {
        match self {
            Self::Forward(error) => format!(
                "无法把启动请求转交给正在运行的 Navop 实例：{error}\n\
                 请先退出正在运行的 Navop，或重启电脑后再试。"
            ),
            Self::Listen(error) => format!(
                "无法创建 Navop 单实例监听：{error}\n\
                 请检查系统临时目录是否可写，或重启电脑后再试。"
            ),
        }
    }
}

impl std::fmt::Display for SingleInstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.user_message())
    }
}

impl std::error::Error for SingleInstanceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Forward(error) | Self::Listen(error) => Some(error),
        }
    }
}

/// 跨平台门禁：`on_request` 在**后台监听线程**上被调用，返回值表示请求是否被接受
/// （Windows 侧用它回 ACK，Unix 侧用来记日志）。
///
/// 返回 [`SingleInstanceOutcome::Primary`] 时本进程要继续启动，且监听线程与进程同寿。
pub fn acquire(
    app_id: &str,
    request: &StartupRequest,
    on_request: impl Fn(StartupRequest) -> bool + Send + Sync + 'static,
) -> Result<SingleInstanceOutcome, SingleInstanceError> {
    #[cfg(target_os = "windows")]
    {
        windows::claim_or_forward_named(&windows::pipe_name(app_id), request, on_request)
    }
    #[cfg(unix)]
    {
        unix::claim_or_forward(app_id, request, on_request)
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = (app_id, request, on_request);
        Err(SingleInstanceError::Listen(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "当前平台未实现单实例门禁",
        )))
    }
}

/// 独立产品的门禁入口：`Ok(None)` 表示已有实例在运行，调用方应当**干净地**
/// `return`（不 panic、不 `process::exit`，让日志与资源正常收尾）。
pub fn acquire_for_product(
    app_id: &'static str,
) -> Result<Option<StartupGate>, SingleInstanceError> {
    let (tx, rx) = smol::channel::unbounded();
    let outcome = acquire(app_id, &StartupRequest::default(), move |request| {
        tx.try_send(request).is_ok()
    })?;

    match outcome {
        SingleInstanceOutcome::Primary => Ok(Some(StartupGate { app_id, rx })),
        SingleInstanceOutcome::Forwarded => Ok(None),
    }
}

/// 主实例侧的转发请求接收端。装到 GPUI 上之后，每次二次启动都会把主窗口带到前台。
pub struct StartupGate {
    app_id: &'static str,
    rx: smol::channel::Receiver<StartupRequest>,
}

impl StartupGate {
    /// 把门禁挂到当前应用上：监听线程收到的每个请求都会前置主窗口。
    pub fn install(self, cx: &mut gpui::App) {
        let app_id = self.app_id;
        let rx = self.rx;
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let mut cx = cx.clone();
            while let Ok(request) = rx.recv().await {
                tracing::info!(
                    app_id,
                    paths = request.paths().len(),
                    "已有实例转发了启动请求，前置主窗口"
                );
                bring_main_window_to_front(&mut cx);
            }
            tracing::debug!(app_id, "单实例监听通道已关闭");
        })
        .detach();
    }
}

/// 二次启动时把第一个窗口带到前台。找不到窗口只记日志：启动早期的转发可能早于
/// 窗口创建，此时激活动作本来也没有意义。
fn bring_main_window_to_front(cx: &mut gpui::AsyncApp) {
    use gpui::AppContext as _;

    let window = cx.update(|cx| cx.windows().first().copied());
    let Some(window) = window else {
        tracing::warn!("转发请求到达时还没有窗口，跳过前置");
        return;
    };
    if let Err(error) = cx.update_window(window, |_, window, _| window.activate_window()) {
        tracing::warn!(%error, "前置主窗口失败");
    }
}

#[cfg(test)]
mod tests;
