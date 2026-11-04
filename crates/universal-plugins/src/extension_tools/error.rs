//! 扩展工具的错误承载。
//!
//! v1 按设计文档 §8.6：所有扩展工具失败都映射到
//! [`ToolError::Execution`]，消息以 `[EXT_*]` 码开头，方便模型与日志
//! 归因。这里集中定义错误码与格式化，避免散落各处拼写漂移。

use agent_runtime::ToolError;

/// 扩展工具错误码（设计文档 §8.6）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtensionToolErrorCode {
    UnknownTool,
    NotExposed,
    InvalidArguments,
    MissingConnection,
    WrongConnection,
    ExtensionUnavailable,
    ProviderError,
    Timeout,
    Cancelled,
    ResultTooLarge,
    UnsupportedResult,
}

impl ExtensionToolErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownTool => "EXT_UNKNOWN_TOOL",
            Self::NotExposed => "EXT_NOT_EXPOSED",
            Self::InvalidArguments => "EXT_INVALID_ARGUMENTS",
            Self::MissingConnection => "EXT_MISSING_CONNECTION",
            Self::WrongConnection => "EXT_WRONG_CONNECTION",
            Self::ExtensionUnavailable => "EXT_EXTENSION_UNAVAILABLE",
            Self::ProviderError => "EXT_PROVIDER_ERROR",
            Self::Timeout => "EXT_TIMEOUT",
            Self::Cancelled => "EXT_CANCELLED",
            Self::ResultTooLarge => "EXT_RESULT_TOO_LARGE",
            Self::UnsupportedResult => "EXT_UNSUPPORTED_RESULT",
        }
    }
}

/// provider 错误消息的截断上限。
pub const PROVIDER_ERROR_MAX_CHARS: usize = 512;

/// 构造 `[CODE] message` 形式的执行错误。
pub fn execution_error(code: ExtensionToolErrorCode, message: impl AsRef<str>) -> ToolError {
    ToolError::Execution(format!("[{}] {}", code.as_str(), message.as_ref()))
}

/// provider 错误：按 §8.6 截断到 512 字符（按 char 截，避免劈开多字节字符）。
pub fn provider_error(message: impl AsRef<str>) -> ToolError {
    let message = message.as_ref();
    let truncated: String = message.chars().take(PROVIDER_ERROR_MAX_CHARS).collect();
    let suffix = if truncated.len() < message.len() {
        "…"
    } else {
        ""
    };
    execution_error(
        ExtensionToolErrorCode::ProviderError,
        format!("{truncated}{suffix}"),
    )
}

/// 参数校验错误。
pub fn invalid_arguments(message: impl AsRef<str>) -> ToolError {
    execution_error(ExtensionToolErrorCode::InvalidArguments, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_error_carries_code_prefix() {
        let err = execution_error(ExtensionToolErrorCode::Timeout, "执行超时");
        match err {
            ToolError::Execution(message) => {
                assert!(message.starts_with("[EXT_TIMEOUT] 执行超时"));
            }
            other => panic!("应当是 Execution 错误，实际 {other:?}"),
        }
    }

    #[test]
    fn provider_error_truncates_to_512_chars() {
        let long: String = "错".repeat(600);
        let ToolError::Execution(message) = provider_error(long) else {
            panic!("应当是 Execution 错误");
        };
        // 512 字符 + 省略号。
        assert!(message.ends_with('…'), "截断后应带省略号: {message}");
        let body = message.trim_start_matches("[EXT_PROVIDER_ERROR] ");
        assert_eq!(body.chars().count(), 513);
    }

    #[test]
    fn provider_error_keeps_short_message() {
        let ToolError::Execution(message) = provider_error("boom") else {
            panic!("应当是 Execution 错误");
        };
        assert_eq!(message, "[EXT_PROVIDER_ERROR] boom");
    }
}
