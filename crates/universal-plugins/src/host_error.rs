//! 结构化的 navop 错误 envelope（design §16）。
//!
//! 两个 UI 平面共用的错误词汇表：gpui-shell 的 `HostError` 只携带一句
//! message，nsh 的 `HostCall::reject` 也一样，所以稳定 code 统一编码成
//! 可搜索 marker + Base64URL JSON 追加到 message 末尾；SDK 搜索最后一个
//! `__NAVOP_ERROR__` 并解码，解码失败则退回普通 JavaScript Error。
//!
//! 本模块不依赖任何 UI 平面类型，只产出 `NavopError`；各平面再决定怎么
//! 把它交给脚本（`HostError::new` / `reject`）。

use base64::Engine as _;

/// SDK 用于定位 envelope 的 marker。
pub(crate) const ERROR_MARKER: &str = "__NAVOP_ERROR__";
const MAX_MESSAGE_CHARS: usize = 2048;
const MAX_DETAILS_BYTES: usize = 8192;

/// design §16 的稳定错误码。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// 与 gpui-shell 平面共用一套 code：本 feature 单独开时，只有另一边用到的
// 变体/映射暂时没人构造（M7 删掉 gpui-shell 后由 workbench 段承接）。
#[cfg_attr(
    not(feature = "nsh-host"),
    allow(
        dead_code,
        reason = "workbench 映射在 M5 接线前只被 gpui-shell 平面使用"
    )
)]
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

    /// 渲染成 `"<message>\n__NAVOP_ERROR__<base64url>"`。
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

/// 把 `WorkbenchDispatchError` 映射为稳定 code。
#[cfg_attr(
    not(feature = "nsh-host"),
    allow(
        dead_code,
        reason = "workbench 分发在 M5 接线（navop.workbench）后才被 nsh 平面使用"
    )
)]
pub(crate) fn workbench_error(
    error: extension_plugin_adapter::WorkbenchDispatchError,
) -> NavopError {
    use extension_plugin_adapter::WorkbenchDispatchError as W;
    let code = match &error {
        W::MissingCapability { .. } | W::ConfirmationRequired(_) => ErrorCode::PermissionDenied,
        W::UnknownOperation(_) | W::BindingMissing { .. } | W::BindingType { .. } => {
            ErrorCode::InvalidArgument
        }
        // 运行时/传输没了只是暂时的:前端可以自愈重试,所以给可重试的稳定 code,
        // 而不是让用户看到一个笼统的 PROTOCOL_ERROR。
        W::ProviderUnavailable(_) => ErrorCode::RuntimeUnavailable,
        W::ResultContract { .. } | W::Provider(_) | W::EventStreamRegistration { .. } => {
            ErrorCode::ProtocolError
        }
    };
    let mut navop = NavopError::new(code, error.to_string());
    navop.retryable = matches!(error, W::ProviderUnavailable(_));
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

    fn decode(error: &NavopError) -> serde_json::Value {
        let rendered = error.render();
        let marker = rendered.rfind(ERROR_MARKER).expect("marker is present");
        let encoded = &rendered[marker + ERROR_MARKER.len()..];
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .expect("envelope is valid base64url");
        serde_json::from_slice(&bytes).expect("envelope is valid json")
    }

    #[test]
    fn invalid_handle_renders_decodable_envelope() {
        let error = NavopError::new(ErrorCode::InvalidHandle, "invalid blob handle `blob-1`");
        let envelope = decode(&error);
        assert_eq!(envelope["code"], "INVALID_HANDLE");
        assert_eq!(envelope["retryable"], false);
        assert!(error.render().starts_with("invalid blob handle"));
    }

    #[test]
    fn cancelled_maps_to_request_cancelled() {
        let error = NavopError::from(extension_host::HostError::Cancelled {
            method: "job/start".into(),
        });
        assert_eq!(decode(&error)["code"], "REQUEST_CANCELLED");
    }

    #[test]
    fn timeout_maps_to_retryable_request_timeout() {
        let error = NavopError::from(extension_host::HostError::Timeout {
            method: "resource/invoke".into(),
            timeout_ms: 30_000,
        });
        let envelope = decode(&error);
        assert_eq!(envelope["code"], "REQUEST_TIMEOUT");
        assert_eq!(envelope["retryable"], true);
    }

    #[test]
    fn frame_too_large_maps_to_result_too_large() {
        let error =
            NavopError::from(extension_host::HostError::FrameTooLarge { limit_bytes: 4096 });
        let envelope = decode(&error);
        assert_eq!(envelope["code"], "RESULT_TOO_LARGE");
        assert_eq!(envelope["details"]["limitBytes"], 4096);
    }
}
