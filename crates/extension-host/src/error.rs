//! 宿主侧错误类型。
//!
//! 把以下三类错误统一到一个 `HostError`,方便调用方匹配:
//!
//! 1. **Transport 错误**:`io::Error` 包装(socket 断开、读不到帧、消息超大)。
//! 2. **Protocol 错误**:扩展返回了 `Response.error`,带结构化 `ProtocolError`。
//! 3. **Client 内部错误**:超时、取消、客户端已关闭、未初始化等。

use extension_protocol::error::ProtocolError;
use thiserror::Error;

/// 通用 host 端错误。
#[derive(Debug, Error)]
pub enum HostError {
    /// 底层 IO 错误(socket / stdio / 文件)。
    #[error("transport io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON 序列化/反序列化失败。
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),

    /// 扩展子进程返回的协议错误(Response.error 字段)。
    ///
    /// 装箱避免每个 `HostError` 都按 248-byte 最大 variant 分配。
    #[error("protocol error: {0}")]
    Protocol(Box<ProtocolError>),

    /// 请求超时,本地侧主动放弃等待。
    #[error("request timed out after {timeout_ms}ms (method `{method}`)")]
    Timeout { method: String, timeout_ms: u64 },

    /// 请求被取消(用户主动 cancel,或 client 关闭)。
    #[error("request cancelled (method `{method}`)")]
    Cancelled { method: String },

    /// 客户端已被关闭,所有后续请求立刻失败。
    #[error("rpc client is closed")]
    Closed,

    /// 握手前发起了请求。
    #[error("rpc client is not initialized")]
    NotInitialized,

    /// 子进程异常退出。
    #[error("extension process exited unexpectedly: {0}")]
    ProcessExited(String),

    /// 配置不合法(spawn 时缺关键字段、socket 名错误等)。
    #[error("invalid host config: {0}")]
    Config(String),

    /// 子进程没在 deadline 内 ready(未建立 socket 连接)。
    #[error("extension process did not become ready within {deadline_ms}ms")]
    ProcessNotReady {
        deadline_ms: u64,
        stderr_tail: Option<String>,
    },

    /// 协议契约层面的不兼容(版本不匹配、缺必备 capability 等)。
    #[error("compatibility error: {0}")]
    Incompatible(String),

    /// 功能未实现。
    #[error("not implemented: {0}")]
    NotImplemented(String),
}

impl HostError {
    /// 把协议错误装到 HostError(避免每个 caller 都写 `HostError::Protocol(...)`)。
    pub fn protocol(error: ProtocolError) -> Self {
        Self::Protocol(Box::new(error))
    }

    /// 是否值得重试(网络抖动 / 超时类)。
    pub fn is_retriable(&self) -> bool {
        match self {
            Self::Io(_) | Self::Timeout { .. } | Self::ProcessNotReady { .. } => true,
            Self::Protocol(e) => {
                e.is_connection_error()
                    || e.data.as_ref().and_then(|d| d.retryable).unwrap_or(false)
            }
            _ => false,
        }
    }
}

pub type HostResult<T> = Result<T, HostError>;

#[cfg(test)]
mod tests {
    use super::*;
    use extension_protocol::error::{ErrorData, error_codes};

    #[test]
    fn io_error_round_trips() {
        let e: HostError = std::io::Error::other("boom").into();
        match e {
            HostError::Io(io) => assert!(io.to_string().contains("boom")),
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn serde_error_via_from() {
        let s = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let e: HostError = s.into();
        assert!(matches!(e, HostError::Serde(_)));
    }

    #[test]
    fn protocol_error_helper() {
        let pe = ProtocolError::new(error_codes::SQL_SYNTAX_ERROR, "syntax");
        let e = HostError::protocol(pe);
        assert!(matches!(e, HostError::Protocol(_)));
    }

    #[test]
    fn is_retriable_for_io_and_timeout() {
        let io: HostError = std::io::Error::other("x").into();
        assert!(io.is_retriable());
        let to = HostError::Timeout {
            method: "x".into(),
            timeout_ms: 100,
        };
        assert!(to.is_retriable());
    }

    #[test]
    fn is_retriable_for_connection_protocol_error() {
        let pe = ProtocolError::new(error_codes::IO_CONNECTION_REFUSED, "no");
        assert!(HostError::protocol(pe).is_retriable());
    }

    #[test]
    fn is_retriable_uses_retryable_data_flag() {
        let pe = ProtocolError::new(error_codes::SQL_SYNTAX_ERROR, "syntax")
            .with_data(ErrorData::new().retryable(true));
        assert!(HostError::protocol(pe).is_retriable());
    }

    #[test]
    fn is_retriable_false_for_sql_without_flag() {
        let pe = ProtocolError::new(error_codes::SQL_SYNTAX_ERROR, "syntax");
        assert!(!HostError::protocol(pe).is_retriable());
    }

    #[test]
    fn is_retriable_false_for_logical_errors() {
        assert!(!HostError::Closed.is_retriable());
        assert!(!HostError::NotInitialized.is_retriable());
        assert!(!HostError::Cancelled { method: "x".into() }.is_retriable());
    }

    #[test]
    fn display_contains_method_and_timeout() {
        let e = HostError::Timeout {
            method: "query/start".into(),
            timeout_ms: 5_000,
        };
        let s = format!("{e}");
        assert!(s.contains("query/start"));
        assert!(s.contains("5000"));
    }

    #[test]
    fn display_for_cancelled() {
        let e = HostError::Cancelled {
            method: "exec/run".into(),
        };
        assert!(format!("{e}").contains("exec/run"));
    }
}
