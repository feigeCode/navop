//! Windows 单实例后端：命名管道 + `FILE_FLAG_FIRST_PIPE_INSTANCE` 互斥语义。
//!
//! 取名字这件事本身就是互斥的：首个管道实例带 `FILE_FLAG_FIRST_PIPE_INSTANCE`，
//! 同一台机器上同名实例只能存在一个，因此"建得出首个实例"就是"本进程拥有这个
//! 应用"的证明。
//!
//! 服务循环永远**先补建下一个实例、再移交当前连接**：任何时刻都至少有一个实例
//! 持有名称，别的进程不会在这条缝隙里抢到主实例资格。所有 I/O 都带期限，半开
//! 连接不能拖住后续启动请求。

use std::io;
use std::path::Path;
use std::time::Duration;

use sha2::{Digest as _, Sha256};

use super::{SingleInstanceError, SingleInstanceOutcome, StartupRequest};

mod forward;
mod server;

use forward::{ERROR_ACCESS_DENIED, forward_attempt};
use server::{create_pipe_instance, serve_forever};

const CLAIM_DEADLINE: Duration = Duration::from_secs(2);
const DECISION_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(20);

/// 产品专属管道名：三个产品（`navop` / `navop-shell` / `navop-data`）的 `app_id`
/// 不同，因此派生出的名称互不相同，天然互不干扰。
pub fn pipe_name(app_id: &str) -> String {
    format!(r"\\.\pipe\{}", instance_name(app_id))
}

/// 兼容旧入口：按配置目录派生实例名（全量产品升级前后必须连到同一个实例）。
fn instance_name_for_config_dir(config_dir: &Path) -> String {
    instance_name(&config_dir.to_string_lossy().to_lowercase())
}

fn pipe_name_for_config_dir(config_dir: &Path) -> String {
    format!(r"\\.\pipe\{}", instance_name_for_config_dir(config_dir))
}

fn instance_name(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    let suffix = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("navop-single-instance-{suffix}")
}

/// 首个管道实例建好即 `Primary`；连上已有实例并投递成功即 `Forwarded`。
enum Claim {
    Primary(tokio::net::windows::named_pipe::NamedPipeServer),
    Forwarded,
}

fn is_name_in_use(error: &io::Error) -> bool {
    // 首个实例建不出来时，OS 用 ERROR_ACCESS_DENIED 表示"这个名字已经有实例了"
    // （`FILE_FLAG_FIRST_PIPE_INSTANCE` 的语义）。Rust std 把它映射成
    // `ErrorKind::PermissionDenied` 而不是 `AddrInUse`，所以只能按原始错误码判断。
    error.raw_os_error() == Some(ERROR_ACCESS_DENIED)
}

/// 同一个配置目录只允许一个 GUI 实例。
///
/// 返回 [`SingleInstanceOutcome::Primary`] 表示本进程已经取得主实例资格（管道
/// 实例已建立并处于监听状态），可以继续启动 GUI；返回
/// [`SingleInstanceOutcome::Forwarded`] 表示启动请求已被已有实例接管，调用方需要
/// 直接退出。任何 `Err` 都必须终止当前进程：它意味着"既当不了主实例、也联系不上
/// 已有实例"，此时继续启动就会产出第二个窗口。
///
/// `on_request` 的返回值表示启动请求是否成功进入主实例的处理队列；只有成功入队
/// 才会回成功 ACK，不需要等文件真正打开。
pub fn claim_or_forward(
    config_dir: &Path,
    request: &StartupRequest,
    on_request: impl Fn(StartupRequest) -> bool + Send + Sync + 'static,
) -> Result<SingleInstanceOutcome, SingleInstanceError> {
    claim_or_forward_named(&pipe_name_for_config_dir(config_dir), request, on_request)
}

/// 与 [`claim_or_forward`] 相同，但直接给定管道名；`acquire(app_id, ...)` 走这里。
pub(super) fn claim_or_forward_named(
    pipe_name: &str,
    request: &StartupRequest,
    on_request: impl Fn(StartupRequest) -> bool + Send + Sync + 'static,
) -> Result<SingleInstanceOutcome, SingleInstanceError> {
    let pipe_name = pipe_name.to_owned();
    let request = request.clone();
    let on_request: std::sync::Arc<dyn Fn(StartupRequest) -> bool + Send + Sync> =
        std::sync::Arc::new(on_request);
    let (outcome_tx, outcome_rx) = std::sync::mpsc::sync_channel(1);

    std::thread::Builder::new()
        .name("navop-single-instance".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = outcome_tx.send(Err(SingleInstanceError::Listen(error)));
                    return;
                }
            };

            runtime.block_on(async move {
                match claim_or_forward_async(&pipe_name, &request).await {
                    Ok(Claim::Forwarded) => {
                        let _ = outcome_tx.send(Ok(SingleInstanceOutcome::Forwarded));
                    }
                    Ok(Claim::Primary(server)) => {
                        // 首个管道实例已经建好，此后"本进程拥有这个配置目录"
                        // 才成立；先上报判定，再进入服务循环。
                        let _ = outcome_tx.send(Ok(SingleInstanceOutcome::Primary));
                        serve_forever(pipe_name, server, on_request).await;
                    }
                    Err(error) => {
                        let _ = outcome_tx.send(Err(error));
                    }
                }
            });
        })
        .map_err(SingleInstanceError::Listen)?;

    match outcome_rx.recv_timeout(DECISION_TIMEOUT) {
        Ok(outcome) => outcome,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err(SingleInstanceError::Listen(io::Error::new(
                io::ErrorKind::TimedOut,
                "the single-instance worker did not report a decision in time",
            )))
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(SingleInstanceError::Listen(
            io::Error::other("the single-instance worker stopped before reporting a decision"),
        )),
    }
}

async fn claim_or_forward_async(
    pipe_name: &str,
    request: &StartupRequest,
) -> Result<Claim, SingleInstanceError> {
    use tokio::time::Instant;

    let deadline = Instant::now() + CLAIM_DEADLINE;

    loop {
        match create_pipe_instance(pipe_name, true) {
            Ok(server) => return Ok(Claim::Primary(server)),
            Err(error) if is_name_in_use(&error) => {
                match forward_attempt(pipe_name, request).await {
                    // 已有实例真的接下了请求：本进程必须退出。
                    ForwardAttempt::Delivered => return Ok(Claim::Forwarded),
                    ForwardAttempt::Rejected(error) => {
                        return Err(SingleInstanceError::Forward(error));
                    }
                    ForwardAttempt::Unreachable(error) => {
                        // 名字被占着却连不上，可能是对方刚好退出。在总期限内
                        // 继续走"取得资格 / 转发"这条循环，但绝不绕过它。
                        if Instant::now() >= deadline {
                            return Err(SingleInstanceError::Forward(error));
                        }
                        tokio::time::sleep(CONNECT_RETRY_DELAY).await;
                    }
                }
            }
            Err(error) => return Err(SingleInstanceError::Listen(error)),
        }
    }
}
