use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 协议版本号，用于连接双方协商兼容性。
///
/// 主版本不匹配 = 不兼容，必须断开。
/// 次版本递增 = 向后兼容的新增功能。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    pub const fn is_compatible_with(self, other: Self) -> bool {
        self.major == other.major
    }
}

/// 当前 IPC 协议版本。
pub const IPC_VERSION: ProtocolVersion = ProtocolVersion::new(1, 0);

/// 请求信封。
#[derive(Debug, Serialize, Deserialize)]
pub struct IpcRequest {
    pub protocol_version: ProtocolVersion,
    pub request_id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl IpcRequest {
    pub fn new(request_id: u64, method: impl Into<String>, params: Value) -> Self {
        Self {
            protocol_version: IPC_VERSION,
            request_id,
            method: method.into(),
            params,
        }
    }
}

/// 响应信封。
#[derive(Debug, Serialize, Deserialize)]
pub struct IpcResponse {
    pub protocol_version: ProtocolVersion,
    pub request_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<IpcError>,
}

impl IpcResponse {
    pub fn result(request_id: u64, result: Value) -> Self {
        Self {
            protocol_version: IPC_VERSION,
            request_id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(request_id: u64, code: IpcErrorCode, message: impl Into<String>) -> Self {
        Self {
            protocol_version: IPC_VERSION,
            request_id,
            result: None,
            error: Some(IpcError {
                code,
                message: message.into(),
                retriable: code.is_retriable(),
            }),
        }
    }
}

/// 结构化错误码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpcErrorCode {
    /// 请求格式错误
    InvalidRequest,
    /// 不支持的方法
    UnsupportedMethod,
    /// 协议版本不兼容
    VersionMismatch,
    /// 内部错误
    Internal,
    /// 传输层错误
    Transport,
    /// 超时
    Timeout,
}

impl IpcErrorCode {
    fn is_retriable(self) -> bool {
        matches!(self, Self::Timeout | Self::Transport)
    }
}

/// IPC 错误。
#[derive(Debug, Serialize, Deserialize)]
pub struct IpcError {
    pub code: IpcErrorCode,
    pub message: String,
    #[serde(default)]
    pub retriable: bool,
}
