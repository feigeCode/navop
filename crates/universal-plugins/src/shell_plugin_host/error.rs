//! 结构化错误 envelope(design §16)。
//!
//! gpui-shell 的 `HostError` 只携带一句 message,runtime 还会在外层补
//! `` `module.function`: ``。Navop 把稳定 code 编码为可搜索 marker +
//! Base64URL JSON 追加到 message 末尾,SDK 搜索最后一个 `__NAVOP_ERROR__`
//! 并解码,解码失败则退回普通 JavaScript Error。

use base64::Engine as _;
use gpui_shell::HostError;

/// SDK 用于定位 envelope 的 marker。
pub(crate) const ERROR_MARKER: &str = "__NAVOP_ERROR__";
const MAX_MESSAGE_CHARS: usize = 2048;
const MAX_DETAILS_BYTES: usize = 8192;

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
    /// 预留给 provider 结果超过预算的场景,当前仅作为稳定 code 定义。
    #[allow(dead_code)]
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

/// 一条 navop 错误:code + message + retryable + 可选 details。
pub(crate) struct NavopError {
    code: ErrorCode,
    message: String,
    retryable: bool,
    details: Option<serde_json::Value>,
}

impl NavopError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: false,
            details: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn details(mut self, details: impl Into<serde_json::Value>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// 渲染为 `message\n__NAVOP_ERROR__<base64url-json>`。
    pub(crate) fn render(&self) -> String {
        let message = truncate_chars(&self.message, MAX_MESSAGE_CHARS);
        let mut envelope = serde_json::json!({
            "code": self.code.as_str(),
            "message": message,
            "retryable": self.retryable,
        });
        if let Some(details) = &self.details {
            match serde_json::to_vec(details) {
                Ok(encoded) if encoded.len() <= MAX_DETAILS_BYTES => {
                    envelope["details"] = details.clone();
                }
                _ => {
                    envelope["details"] = serde_json::Value::Null;
                }
            }
        }
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&envelope).unwrap_or_default());
        format!("{message}\n{ERROR_MARKER}{encoded}")
    }

    pub(crate) fn into_host_error(self) -> HostError {
        HostError::new(self.render())
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

/// 便捷构造:`navop_error(ErrorCode::InvalidHandle, \"…\")`。
pub(crate) fn navop_error(code: ErrorCode, message: impl Into<String>) -> HostError {
    NavopError::new(code, message).into_host_error()
}

/// 把 provider 侧错误映射为结构化 navop 错误。
pub(crate) fn host_error(error: extension_host::HostError) -> HostError {
    NavopError::from(error).into_host_error()
}

/// 把激活/服务层错误映射为结构化 navop 错误。
pub(crate) fn service_error(error: extension_plugin_adapter::ActivationError) -> HostError {
    use extension_plugin_adapter::ActivationError as A;
    let (code, retryable) = match &error {
        A::RuntimeNotFound { .. } | A::InvalidRuntime { .. } => (ErrorCode::BackendNotFound, false),
        A::SessionStart(_) => (ErrorCode::BackendStartFailed, true),
        A::HostBlob(_) | A::RuntimeNotReady { .. } => (ErrorCode::RuntimeUnavailable, false),
    };
    let mut navop = NavopError::new(code, error.to_string());
    navop.retryable = retryable;
    navop.into_host_error()
}

/// 把 `WorkbenchDispatchError` 映射为稳定 code。
pub(crate) fn workbench_error(
    error: extension_plugin_adapter::WorkbenchDispatchError,
) -> HostError {
    use extension_plugin_adapter::WorkbenchDispatchError as W;
    let code = match &error {
        W::MissingCapability { .. } | W::ConfirmationRequired(_) => ErrorCode::PermissionDenied,
        W::UnknownOperation(_) | W::BindingMissing { .. } | W::BindingType { .. } => {
            ErrorCode::InvalidArgument
        }
        W::ResultContract { .. }
        | W::Provider(_)
        | W::EventStreamRegistration { .. } => ErrorCode::ProtocolError,
    };
    NavopError::new(code, error.to_string()).into_host_error()
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

    fn decode(error: &HostError) -> serde_json::Value {
        let message = error.message();
        let marker = message
            .rfind(ERROR_MARKER)
            .expect("envelope marker is present");
        let encoded = &message[marker + ERROR_MARKER.len()..];
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .expect("envelope is valid base64url");
        serde_json::from_slice(&bytes).expect("envelope is valid json")
    }

    #[test]
    fn invalid_handle_renders_decodable_envelope() {
        let error = navop_error(ErrorCode::InvalidHandle, "invalid blob handle `blob-1`");
        let envelope = decode(&error);
        assert_eq!(envelope["code"], "INVALID_HANDLE");
        assert_eq!(envelope["retryable"], false);
        assert!(error.message().starts_with("invalid blob handle"));
    }

    #[test]
    fn cancelled_maps_to_request_cancelled() {
        let error = host_error(extension_host::HostError::Cancelled {
            method: "job/start".into(),
        });
        assert_eq!(decode(&error)["code"], "REQUEST_CANCELLED");
    }

    #[test]
    fn timeout_maps_to_retryable_request_timeout() {
        let error = host_error(extension_host::HostError::Timeout {
            method: "resource/invoke".into(),
            timeout_ms: 30_000,
        });
        let envelope = decode(&error);
        assert_eq!(envelope["code"], "REQUEST_TIMEOUT");
        assert_eq!(envelope["retryable"], true);
    }

    #[test]
    fn closed_maps_to_extension_unloaded() {
        let error = host_error(extension_host::HostError::Closed);
        assert_eq!(decode(&error)["code"], "EXTENSION_UNLOADED");
    }

    #[test]
    fn details_over_budget_are_dropped() {
        let error = NavopError::new(ErrorCode::ProtocolError, "boom")
            .details(serde_json::Value::String("x".repeat(MAX_DETAILS_BYTES + 1)))
            .into_host_error();
        assert_eq!(decode(&error)["details"], serde_json::Value::Null);
    }

    #[test]
    fn long_message_is_truncated_before_marker() {
        let error = navop_error(
            ErrorCode::InvalidArgument,
            "y".repeat(MAX_MESSAGE_CHARS + 10),
        );
        // marker 及其后的 base64 必须完整保留,message 前缀被截断。
        decode(&error);
        let message = error.message();
        let marker = message.rfind(ERROR_MARKER).unwrap();
        assert!(message[..marker].trim_end().chars().count() <= MAX_MESSAGE_CHARS + 1);
    }
}
