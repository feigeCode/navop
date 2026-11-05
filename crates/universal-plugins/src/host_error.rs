//! 结构化的 navop 错误（host ABI v2 §6）。
//!
//! 稳定 code + message + retryable + 可选 details；页面侧从 `host.call` 的
//! rejection 上读 `error.code` / `error.retryable` / `error.details`。nsh 的
//! [`HostCall::reject_error`](nsh_engine::HostCall::reject_error) 直接把
//! `JsError` 变成带属性的 JS 错误，所以这里不再编码 markers/base64 信封，
//! 只负责把 `NavopError` 翻译成 `JsError::Fields`。
//!
//! 本模块不依赖任何 UI 平面类型，只产出 `NavopError`；调用方把它翻成
//! `JsError` 再经 `HostCall::reject_error` / `HostReply::reply_error` 交给脚本。

use nsh_core::element::{ErrorClass, ErrorFields, JsError};
use serde_json::Value;

const MAX_MESSAGE_CHARS: usize = 2048;

/// design §16 的稳定错误码。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorCode {
    InvalidArgument,
    PermissionDenied,
    BackendNotFound,
    BackendStartFailed,
    RuntimeUnavailable,
    InvalidHandle,
    StaleHandle,
    RequestCancelled,
    RequestTimeout,
    /// 预留给 provider 结果/请求超过预算的场景。
    ResultTooLarge,
    ProtocolError,
    ExtensionUnloaded,
}

impl ErrorCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::BackendNotFound => "BACKEND_NOT_FOUND",
            Self::BackendStartFailed => "BACKEND_START_FAILED",
            Self::RuntimeUnavailable => "RUNTIME_UNAVAILABLE",
            Self::InvalidHandle => "INVALID_HANDLE",
            Self::StaleHandle => "STALE_HANDLE",
            Self::RequestCancelled => "REQUEST_CANCELLED",
            Self::RequestTimeout => "REQUEST_TIMEOUT",
            Self::ResultTooLarge => "RESULT_TOO_LARGE",
            Self::ProtocolError => "PROTOCOL_ERROR",
            Self::ExtensionUnloaded => "EXTENSION_UNLOADED",
        }
    }
}

/// 一条 navop 错误：code + message + retryable + 可选 details。
#[derive(Clone, Debug)]
pub(crate) struct NavopError {
    pub(crate) code: ErrorCode,
    pub(crate) message: String,
    pub(crate) retryable: bool,
    pub(crate) details: Option<serde_json::Value>,
}

impl NavopError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: matches!(code, ErrorCode::RequestTimeout),
            details: None,
        }
    }

    /// 变成页面侧能按属性读取的 JS 错误：`Error { code, retryable, details? }`。
    pub(crate) fn to_js_error(&self) -> JsError {
        // message 截断到 2K 字符：错误说明不需要整段堆栈，撑爆 IPC 反而
        // 让页面拿不到结构化字段。
        let message = truncate_chars(&self.message, MAX_MESSAGE_CHARS);
        let mut fields = vec![
            ("code", Value::String(self.code.as_str().to_owned())),
            ("retryable", Value::Bool(self.retryable)),
        ];
        if let Some(details) = &self.details {
            fields.push(("details", details.clone()));
        }
        JsError::Fields(Box::new(ErrorFields {
            class: ErrorClass::Error,
            message,
            fields,
        }))
    }
}

impl From<extension_host::HostError> for NavopError {
    fn from(error: extension_host::HostError) -> Self {
        use extension_host::HostError as ProviderError;
        let (code, details) = match &error {
            ProviderError::Cancelled { .. } => (ErrorCode::RequestCancelled, None),
            ProviderError::Timeout { .. } => (ErrorCode::RequestTimeout, None),
            ProviderError::Closed | ProviderError::NotInitialized => {
                (ErrorCode::ExtensionUnloaded, None)
            }
            ProviderError::ProcessExited(_)
            | ProviderError::ProcessNotReady { .. }
            | ProviderError::Config(_) => (ErrorCode::BackendStartFailed, None),
            ProviderError::Io(_) => (ErrorCode::RuntimeUnavailable, None),
            ProviderError::InvalidParams { .. } => (ErrorCode::InvalidArgument, None),
            // 帧超限和"参数非法"不是一回事:参数可能完全合法,只是 payload 撑爆了
            // 传输预算。给专门的 code,前端才能提示"消息太大"而不是"参数错误"。
            ProviderError::FrameTooLarge { limit_bytes } => (
                ErrorCode::ResultTooLarge,
                Some(serde_json::json!({ "limitBytes": limit_bytes })),
            ),
            // provider 用协议错误码表达权限拒绝；页面应该看到稳定的
            // PERMISSION_DENIED，而不是笼统的 PROTOCOL_ERROR。
            ProviderError::Protocol(protocol)
                if protocol.code == extension_protocol::error::error_codes::PERMISSION_DENIED =>
            {
                (
                    ErrorCode::PermissionDenied,
                    Some(serde_json::json!({ "providerMessage": protocol.message })),
                )
            }
            ProviderError::Protocol(protocol) => (
                ErrorCode::ProtocolError,
                Some(serde_json::json!({
                    "providerCode": protocol.code,
                    "providerMessage": protocol.message,
                })),
            ),
            ProviderError::Serde(_)
            | ProviderError::Incompatible(_)
            | ProviderError::NotImplemented(_) => (ErrorCode::ProtocolError, None),
        };
        Self {
            code,
            message: error.to_string(),
            retryable: error.is_retriable(),
            details,
        }
    }
}

/// 把激活/服务层错误映射为结构化 navop 错误。
pub(crate) fn service_error(error: extension_plugin_adapter::ActivationError) -> NavopError {
    use extension_plugin_adapter::ActivationError as A;
    let (code, retryable) = match &error {
        A::RuntimeNotFound { .. } | A::InvalidRuntime { .. } => (ErrorCode::BackendNotFound, false),
        A::SessionStart(_) => (ErrorCode::BackendStartFailed, true),
        A::HostBlob(_) | A::RuntimeNotReady { .. } => (ErrorCode::RuntimeUnavailable, false),
    };
    let mut navop = NavopError::new(code, error.to_string());
    navop.retryable = retryable;
    navop
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_owned();
    }
    let mut truncated: String = value.chars().take(max).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(error: &NavopError) -> String {
        error
            .to_js_error()
            .code()
            .expect("the JS error carries a code")
            .to_owned()
    }

    fn field(error: &NavopError, key: &str) -> Option<Value> {
        match error.to_js_error() {
            JsError::Fields(fields) => fields.field(key).cloned(),
            other => panic!("expected JsError::Fields, got {other:?}"),
        }
    }

    #[test]
    fn invalid_handle_becomes_a_structured_js_error() {
        let error = NavopError::new(ErrorCode::InvalidHandle, "invalid blob handle `blob-1`");
        let js = error.to_js_error();
        assert_eq!(js.code(), Some("INVALID_HANDLE"));
        assert_eq!(js.message(), "invalid blob handle `blob-1`");
        assert_eq!(field(&error, "retryable"), Some(Value::Bool(false)));
        // details 缺省时不出现在映射里。
        assert_eq!(field(&error, "details"), None);
    }

    #[test]
    fn cancelled_maps_to_request_cancelled() {
        let error = NavopError::from(extension_host::HostError::Cancelled {
            method: "job/start".into(),
        });
        assert_eq!(code(&error), "REQUEST_CANCELLED");
    }

    #[test]
    fn timeout_maps_to_retryable_request_timeout() {
        let error = NavopError::from(extension_host::HostError::Timeout {
            method: "resource/invoke".into(),
            timeout_ms: 30_000,
        });
        assert_eq!(code(&error), "REQUEST_TIMEOUT");
        assert_eq!(field(&error, "retryable"), Some(Value::Bool(true)));
    }

    #[test]
    fn frame_too_large_maps_to_result_too_large() {
        let error =
            NavopError::from(extension_host::HostError::FrameTooLarge { limit_bytes: 4096 });
        assert_eq!(code(&error), "RESULT_TOO_LARGE");
        assert_eq!(
            field(&error, "details").unwrap()["limitBytes"],
            serde_json::json!(4096)
        );
    }
}
