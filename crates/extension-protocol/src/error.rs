//! 错误码与 [`ProtocolError`]。
//!
//! 错误码采用 JSON-RPC 2.0 规约的"负整数"约定,按区间分配:
//!
//! | 区间                 | 类别                                              |
//! | -------------------- | ------------------------------------------------- |
//! | `-32700` ~ `-32600`  | JSON-RPC 标准错误(解析 / 请求格式)                |
//! | `-32099` ~ `-32000`  | JSON-RPC 服务端错误                               |
//! | `-32001` ~ `-32099`  | onetcli 协议错误(method 未知、参数不合法)         |
//! | `-33001` ~ `-33099`  | 连接错误(io_connection_refused、tls_handshake_failed) |
//! | `-34001` ~ `-34099`  | SQL 错误(syntax_error、constraint_violation)      |
//! | `-35001` ~ `-35099`  | 认证错误(auth_failed、permission_denied)          |
//! | `-36001` ~ `-36099`  | 事务错误(serialization_failure、deadlock)         |
//! | `-37001` ~ `-37099`  | 数据错误(type_mismatch、value_out_of_range)       |
//! | `-39000` ~ `-39999`  | 扩展自定义(驱动可自由使用)                        |
//!
//! 详见 [`docs/design/extensions/api-database.md`] §17。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 协议错误码类型。i32 即可覆盖,无需 i64。
pub type ErrorCode = i32;

/// 全部内置错误码常量,避免业务代码硬编码数字。
#[allow(non_snake_case)]
pub mod error_codes {
    use super::ErrorCode;

    // -- JSON-RPC 标准错误 --
    pub const PARSE_ERROR: ErrorCode = -32700;
    pub const INVALID_REQUEST: ErrorCode = -32600;
    pub const METHOD_NOT_FOUND: ErrorCode = -32601;
    pub const INVALID_PARAMS: ErrorCode = -32602;
    pub const INTERNAL_ERROR: ErrorCode = -32603;

    // -- JSON-RPC 服务端错误 --
    pub const SERVER_ERROR_RANGE_START: ErrorCode = -32099;
    pub const SERVER_ERROR_RANGE_END: ErrorCode = -32000;

    // -- onetcli 协议错误 (-32001 ~ -32099) --
    /// 协议握手未完成
    pub const NOT_INITIALIZED: ErrorCode = -32001;
    /// init 调用了第二次
    pub const ALREADY_INITIALIZED: ErrorCode = -32002;
    /// 当前 host 不支持扩展声明的 api 版本
    pub const API_VERSION_MISMATCH: ErrorCode = -32003;
    /// 请求的 capability 未启用
    pub const CAPABILITY_DISABLED: ErrorCode = -32004;
    /// 请求超时
    pub const REQUEST_TIMEOUT: ErrorCode = -32005;
    /// 请求被取消(`$/cancelRequest` 或 cursor/cancel)
    pub const REQUEST_CANCELLED: ErrorCode = -32006;
    /// 连接 id 不存在
    pub const UNKNOWN_CONN_ID: ErrorCode = -32007;
    /// cursor id 不存在
    pub const UNKNOWN_CURSOR_ID: ErrorCode = -32008;
    /// transaction id 不存在
    pub const UNKNOWN_TX_ID: ErrorCode = -32009;
    /// import id 不存在
    pub const UNKNOWN_IMPORT_ID: ErrorCode = -32010;
    /// 资源已被关闭
    pub const RESOURCE_CLOSED: ErrorCode = -32011;

    // -- 连接错误 (-33001 ~ -33099) --
    pub const IO_CONNECTION_REFUSED: ErrorCode = -33001;
    pub const IO_DNS_FAILURE: ErrorCode = -33002;
    pub const IO_NETWORK_UNREACHABLE: ErrorCode = -33003;
    pub const IO_TIMEOUT: ErrorCode = -33004;
    pub const IO_READ_FAILED: ErrorCode = -33005;
    pub const IO_WRITE_FAILED: ErrorCode = -33006;
    pub const TLS_HANDSHAKE_FAILED: ErrorCode = -33010;
    pub const TLS_CERT_INVALID: ErrorCode = -33011;
    pub const SSH_TUNNEL_FAILED: ErrorCode = -33020;
    pub const SERVER_CLOSED_CONNECTION: ErrorCode = -33030;

    // -- SQL 错误 (-34001 ~ -34099) --
    pub const SQL_SYNTAX_ERROR: ErrorCode = -34001;
    pub const SQL_UNKNOWN_TABLE: ErrorCode = -34002;
    pub const SQL_UNKNOWN_COLUMN: ErrorCode = -34003;
    pub const SQL_UNKNOWN_FUNCTION: ErrorCode = -34004;
    pub const SQL_CONSTRAINT_VIOLATION: ErrorCode = -34010;
    pub const SQL_UNIQUE_VIOLATION: ErrorCode = -34011;
    pub const SQL_FOREIGN_KEY_VIOLATION: ErrorCode = -34012;
    pub const SQL_NOT_NULL_VIOLATION: ErrorCode = -34013;
    pub const SQL_CHECK_VIOLATION: ErrorCode = -34014;
    pub const SQL_OBJECT_ALREADY_EXISTS: ErrorCode = -34020;
    pub const SQL_OBJECT_NOT_FOUND: ErrorCode = -34021;

    // -- 认证错误 (-35001 ~ -35099) --
    pub const AUTH_FAILED: ErrorCode = -35001;
    pub const PERMISSION_DENIED: ErrorCode = -35002;
    pub const PASSWORD_EXPIRED: ErrorCode = -35003;
    pub const ACCOUNT_LOCKED: ErrorCode = -35004;
    pub const SECRET_NOT_FOUND: ErrorCode = -35010;

    // -- 事务错误 (-36001 ~ -36099) --
    pub const TX_SERIALIZATION_FAILURE: ErrorCode = -36001;
    pub const TX_DEADLOCK: ErrorCode = -36002;
    pub const TX_ROLLBACK_REQUIRED: ErrorCode = -36003;
    pub const TX_NESTED_NOT_SUPPORTED: ErrorCode = -36004;
    pub const TX_ISOLATION_NOT_SUPPORTED: ErrorCode = -36005;

    // -- 数据错误 (-37001 ~ -37099) --
    pub const DATA_TYPE_MISMATCH: ErrorCode = -37001;
    pub const DATA_VALUE_OUT_OF_RANGE: ErrorCode = -37002;
    pub const DATA_INVALID_ENCODING: ErrorCode = -37003;
    pub const DATA_CHECKSUM_MISMATCH: ErrorCode = -37004;

    // -- 扩展自定义区间 --
    pub const EXTENSION_CUSTOM_START: ErrorCode = -39000;
    pub const EXTENSION_CUSTOM_END: ErrorCode = -39999;
}

/// 协议层错误。
///
/// 与 `Box<dyn std::error::Error>` 不同,这里是「可序列化的错误」,
/// 用于跨进程边界传递。`data` 字段允许携带额外结构化信息(如 SQL 错误的
/// table/column 名、TLS 错误的证书 fingerprint 等)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

impl ProtocolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: ErrorData) -> Self {
        self.data = Some(data);
        self
    }

    /// 是否在 onetcli 协议错误段(`-32001..=-32099`)。
    pub fn is_protocol_error(&self) -> bool {
        (-32099..=-32001).contains(&self.code)
    }

    /// 是否在连接错误段(`-33001..=-33099`)。
    pub fn is_connection_error(&self) -> bool {
        (-33099..=-33001).contains(&self.code)
    }

    /// 是否在 SQL 错误段(`-34001..=-34099`)。
    pub fn is_sql_error(&self) -> bool {
        (-34099..=-34001).contains(&self.code)
    }

    /// 是否在认证错误段(`-35001..=-35099`)。
    pub fn is_auth_error(&self) -> bool {
        (-35099..=-35001).contains(&self.code)
    }

    /// 是否在事务错误段(`-36001..=-36099`)。
    pub fn is_tx_error(&self) -> bool {
        (-36099..=-36001).contains(&self.code)
    }

    /// 是否在数据错误段(`-37001..=-37099`)。
    pub fn is_data_error(&self) -> bool {
        (-37099..=-37001).contains(&self.code)
    }

    /// 是否在扩展自定义段(`-39000..=-39999`)。
    pub fn is_extension_custom(&self) -> bool {
        (-39999..=-39000).contains(&self.code)
    }
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ProtocolError {}

/// 附加的结构化错误数据。
///
/// 大多数错误只用 `extra`(自由 JSON);特定类别(如 SQL)固化了几个常用字段,
/// 方便宿主直接渲染高亮 / 跳转 / suggestion。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ErrorData {
    /// 源代码相关:SQL 文本里出错位置的字节偏移(包含 BOM)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_offset: Option<u32>,

    /// 出错的库名 / schema / 表 / 列(如已知)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraint: Option<String>,

    /// 驱动原始错误码(SQLSTATE / vendor code)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sqlstate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_code: Option<i64>,

    /// 是否可重试(网络抖动 / 死锁 vs. 语法错误)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,

    /// 任意自定义字段,优先用上面那些固化字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

impl ErrorData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn at_offset(mut self, start: u32, end: u32) -> Self {
        self.start_offset = Some(start);
        self.end_offset = Some(end);
        self
    }

    pub fn at_object(
        mut self,
        database: Option<String>,
        schema: Option<String>,
        table: Option<String>,
        column: Option<String>,
    ) -> Self {
        self.database = database;
        self.schema = schema;
        self.table = table;
        self.column = column;
        self
    }

    pub fn with_sqlstate(mut self, sqlstate: impl Into<String>) -> Self {
        self.sqlstate = Some(sqlstate.into());
        self
    }

    pub fn with_vendor_code(mut self, vendor_code: i64) -> Self {
        self.vendor_code = Some(vendor_code);
        self
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = Some(retryable);
        self
    }

    pub fn with_extra(mut self, extra: Value) -> Self {
        self.extra = Some(extra);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_error_new_sets_code_and_message() {
        let e = ProtocolError::new(error_codes::METHOD_NOT_FOUND, "no such method");
        assert_eq!(e.code, error_codes::METHOD_NOT_FOUND);
        assert_eq!(e.message, "no such method");
        assert!(e.data.is_none());
    }

    #[test]
    fn protocol_error_serialize_skips_none_data() {
        let e = ProtocolError::new(error_codes::INTERNAL_ERROR, "boom");
        let s = serde_json::to_string(&e).unwrap();
        assert!(!s.contains("data"));
        assert!(s.contains(r#""code":-32603"#));
        assert!(s.contains(r#""message":"boom""#));
    }

    #[test]
    fn protocol_error_serialize_includes_data() {
        let e = ProtocolError::new(error_codes::SQL_SYNTAX_ERROR, "syntax")
            .with_data(ErrorData::new().at_offset(10, 15));
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains(r#""start_offset":10"#));
        assert!(s.contains(r#""end_offset":15"#));
    }

    #[test]
    fn is_protocol_error_classifies_correctly() {
        assert!(ProtocolError::new(-32001, "x").is_protocol_error());
        assert!(ProtocolError::new(-32099, "x").is_protocol_error());
        assert!(!ProtocolError::new(-32100, "x").is_protocol_error());
        assert!(!ProtocolError::new(-32000, "x").is_protocol_error());
    }

    #[test]
    fn is_connection_error_classifies_correctly() {
        assert!(ProtocolError::new(error_codes::IO_CONNECTION_REFUSED, "x").is_connection_error());
        assert!(ProtocolError::new(error_codes::TLS_HANDSHAKE_FAILED, "x").is_connection_error());
        assert!(!ProtocolError::new(error_codes::SQL_SYNTAX_ERROR, "x").is_connection_error());
    }

    #[test]
    fn is_sql_error_classifies_correctly() {
        assert!(ProtocolError::new(error_codes::SQL_SYNTAX_ERROR, "x").is_sql_error());
        assert!(ProtocolError::new(error_codes::SQL_UNIQUE_VIOLATION, "x").is_sql_error());
        assert!(!ProtocolError::new(error_codes::AUTH_FAILED, "x").is_sql_error());
    }

    #[test]
    fn is_auth_error_classifies_correctly() {
        assert!(ProtocolError::new(error_codes::AUTH_FAILED, "x").is_auth_error());
        assert!(ProtocolError::new(error_codes::PERMISSION_DENIED, "x").is_auth_error());
        assert!(!ProtocolError::new(error_codes::IO_TIMEOUT, "x").is_auth_error());
    }

    #[test]
    fn is_tx_error_classifies_correctly() {
        assert!(ProtocolError::new(error_codes::TX_DEADLOCK, "x").is_tx_error());
        assert!(!ProtocolError::new(error_codes::SQL_SYNTAX_ERROR, "x").is_tx_error());
    }

    #[test]
    fn is_data_error_classifies_correctly() {
        assert!(ProtocolError::new(error_codes::DATA_TYPE_MISMATCH, "x").is_data_error());
        assert!(!ProtocolError::new(error_codes::SQL_SYNTAX_ERROR, "x").is_data_error());
    }

    #[test]
    fn is_extension_custom_classifies_correctly() {
        assert!(ProtocolError::new(-39000, "x").is_extension_custom());
        assert!(ProtocolError::new(-39999, "x").is_extension_custom());
        assert!(!ProtocolError::new(-38999, "x").is_extension_custom());
        assert!(!ProtocolError::new(-40000, "x").is_extension_custom());
    }

    #[test]
    fn error_data_builder_chains() {
        let d = ErrorData::new()
            .at_offset(5, 12)
            .at_object(
                Some("db".to_string()),
                None,
                Some("users".to_string()),
                Some("id".to_string()),
            )
            .with_sqlstate("23505")
            .with_vendor_code(1062)
            .retryable(false);
        assert_eq!(d.start_offset, Some(5));
        assert_eq!(d.end_offset, Some(12));
        assert_eq!(d.database.as_deref(), Some("db"));
        assert!(d.schema.is_none());
        assert_eq!(d.table.as_deref(), Some("users"));
        assert_eq!(d.column.as_deref(), Some("id"));
        assert_eq!(d.sqlstate.as_deref(), Some("23505"));
        assert_eq!(d.vendor_code, Some(1062));
        assert_eq!(d.retryable, Some(false));
    }

    #[test]
    fn error_data_round_trip() {
        let d = ErrorData::new()
            .at_offset(0, 4)
            .with_sqlstate("42601")
            .with_extra(serde_json::json!({"hint": "use COMMA"}));
        let s = serde_json::to_string(&d).unwrap();
        let parsed: ErrorData = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.start_offset, Some(0));
        assert_eq!(parsed.end_offset, Some(4));
        assert_eq!(parsed.sqlstate.as_deref(), Some("42601"));
        assert_eq!(parsed.extra, Some(serde_json::json!({"hint": "use COMMA"})));
    }

    #[test]
    fn protocol_error_round_trip_with_data() {
        let e = ProtocolError::new(error_codes::SQL_UNIQUE_VIOLATION, "duplicate key").with_data(
            ErrorData::new()
                .at_object(None, None, Some("users".into()), Some("email".into()))
                .with_sqlstate("23505"),
        );
        let s = serde_json::to_string(&e).unwrap();
        let parsed: ProtocolError = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.code, error_codes::SQL_UNIQUE_VIOLATION);
        assert_eq!(parsed.message, "duplicate key");
        let d = parsed.data.unwrap();
        assert_eq!(d.table.as_deref(), Some("users"));
        assert_eq!(d.column.as_deref(), Some("email"));
        assert_eq!(d.sqlstate.as_deref(), Some("23505"));
    }

    #[test]
    fn protocol_error_display_format() {
        let e = ProtocolError::new(error_codes::METHOD_NOT_FOUND, "no method");
        assert_eq!(format!("{e}"), "[-32601] no method");
    }

    #[test]
    fn protocol_error_implements_std_error() {
        let e: Box<dyn std::error::Error> =
            Box::new(ProtocolError::new(error_codes::INTERNAL_ERROR, "x"));
        assert_eq!(e.to_string(), "[-32603] x");
    }
}
