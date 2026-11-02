//! gpui-shell 平面的错误适配层。
//!
//! 稳定 code、envelope 编码、provider/服务/工作台错误映射都在
//! [`crate::host_error`]；这里只负责把 `NavopError` 变成 gpui-shell 的
//! `HostError`（它只能携带一句 message）。

use gpui_shell::HostError;

pub(crate) use crate::host_error::{ERROR_MARKER, ErrorCode, NavopError};

impl NavopError {
    pub(crate) fn into_host_error(self) -> HostError {
        HostError::new(self.render())
    }
}

/// 便捷构造:`navop_error(ErrorCode::InvalidHandle, "…")`。
pub(crate) fn navop_error(code: ErrorCode, message: impl Into<String>) -> HostError {
    NavopError::new(code, message).into_host_error()
}

/// 把 provider 侧错误映射为结构化 navop 错误。
pub(crate) fn host_error(error: extension_host::HostError) -> HostError {
    NavopError::from(error).into_host_error()
}

/// 把激活/服务层错误映射为结构化 navop 错误。
pub(crate) fn service_error(error: extension_plugin_adapter::ActivationError) -> HostError {
    crate::host_error::service_error(error).into_host_error()
}

/// 把 `WorkbenchDispatchError` 映射为稳定 code。
pub(crate) fn workbench_error(
    error: extension_plugin_adapter::WorkbenchDispatchError,
) -> HostError {
    crate::host_error::workbench_error(error).into_host_error()
}
