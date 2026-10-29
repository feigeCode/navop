//! 向已有主实例投递启动请求，并等待它的 ACK。

use std::io;
use std::time::Duration;

use super::super::StartupRequest;
use super::super::codec;

pub(super) const ACKNOWLEDGEMENT_ACCEPTED: u8 = 1;
pub(super) const ACKNOWLEDGEMENT_REJECTED: u8 = 0;
pub(super) const REQUEST_IO_TIMEOUT: Duration = Duration::from_secs(2);
pub(super) const ERROR_FILE_NOT_FOUND: i32 = 2;
pub(super) const ERROR_ACCESS_DENIED: i32 = 5;
pub(super) const ERROR_PIPE_BUSY: i32 = 231;

/// 一次转发的三种结果：投递成功、暂时连不上（可重试）、确定失败。
pub(super) enum ForwardAttempt {
    /// 主实例确认请求已经进入启动队列。
    Delivered,
    /// 连不上主实例：它可能正在切换管道实例，或刚刚退出。
    Unreachable(io::Error),
    /// 连上了，但请求被拒或链路中断，属于确定失败。
    Rejected(io::Error),
}

fn is_connect_retryable(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(ERROR_FILE_NOT_FOUND | ERROR_PIPE_BUSY | ERROR_ACCESS_DENIED)
    )
}

async fn forward_attempt(pipe_name: &str, request: &StartupRequest) -> ForwardAttempt {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::windows::named_pipe::ClientOptions;
    use tokio::time::timeout;

    let payload = match codec::encode_request(request) {
        Ok(payload) => payload,
        Err(error) => return ForwardAttempt::Rejected(error),
    };
    let payload_length = match u32::try_from(payload.len()) {
        Ok(length) => length,
        Err(_) => {
            return ForwardAttempt::Rejected(codec::invalid_data("startup request is too large"));
        }
    };

    let mut connection = match ClientOptions::new().open(pipe_name) {
        Ok(connection) => connection,
        Err(error) if is_connect_retryable(&error) => return ForwardAttempt::Unreachable(error),
        Err(error) => return ForwardAttempt::Rejected(error),
    };

    let send = async {
        connection.write_all(&payload_length.to_le_bytes()).await?;
        connection.write_all(&payload).await?;
        connection.flush().await
    };
    if let Err(elapsed) = timeout(REQUEST_IO_TIMEOUT, send).await {
        return ForwardAttempt::Rejected(timeout_error(
            "timed out sending the startup request",
            elapsed,
        ));
    }

    let mut acknowledgement = [0u8; 1];
    // tokio 的 `read_exact` 返回读到的字节数（不是 `()`），这里只关心是否成功。
    match timeout(
        REQUEST_IO_TIMEOUT,
        connection.read_exact(&mut acknowledgement),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => return ForwardAttempt::Rejected(error),
        Err(elapsed) => {
            return ForwardAttempt::Rejected(timeout_error(
                "timed out waiting for the primary instance to acknowledge the startup request",
                elapsed,
            ));
        }
    }

    match acknowledgement {
        [ACKNOWLEDGEMENT_ACCEPTED] => ForwardAttempt::Delivered,
        [ACKNOWLEDGEMENT_REJECTED] => ForwardAttempt::Rejected(codec::invalid_data(
            "the primary instance could not accept the startup request",
        )),
        _ => ForwardAttempt::Rejected(codec::invalid_data(
            "the primary instance sent an invalid acknowledgement",
        )),
    }
}

fn timeout_error(message: &'static str, elapsed: tokio::time::error::Elapsed) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, format!("{message}: {elapsed}"))
}
