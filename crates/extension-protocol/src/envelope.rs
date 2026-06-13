//! JSON-RPC 2.0 envelope。
//!
//! 所有 wire 消息都是以下三种之一:
//!
//! - [`Request`]: 带 `id` 的请求,期望对方返回匹配的 `Response`
//! - [`Response`]: 对某个 `Request` 的响应,要么 `result` 要么 `error`
//! - [`Notification`]: 无 `id` 的单向消息,不要求响应(事件 / 通知)
//!
//! `params` 与 `result` 字段保持为 [`serde_json::Value`],由更高层根据
//! `method` 名字反序列化为具体类型。这样可以避免 envelope 层做巨型枚举,
//! 也方便扩展不认识的方法时优雅返回 `MethodNotFound` 错误。
//!
//! 取消请求复用 LSP / VS Code 风格的 `$/cancelRequest`:
//!
//! ```json
//! { "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 42 } }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ProtocolError;

/// JSON-RPC 请求 id,允许 number / string / null。
///
/// 服务端把同一 id 在 Response 中回带。不同实现喜欢用 number(性能),
/// 也有用 UUID 字符串的(调试方便),都得支持。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
    /// 仅用于解析 `null` id 的 Response。JSON-RPC 规定通知不带 id,但有些
    /// 实现会发 `"id": null`,我们解析时不报错。
    Null,
}

impl RequestId {
    /// 是否 `null`,辅助判断「非通知但 id 缺失」的异常情况。
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

impl From<i64> for RequestId {
    fn from(v: i64) -> Self {
        Self::Number(v)
    }
}

impl From<i32> for RequestId {
    fn from(v: i32) -> Self {
        Self::Number(v as i64)
    }
}

impl From<u64> for RequestId {
    fn from(v: u64) -> Self {
        Self::Number(v as i64)
    }
}

impl From<u32> for RequestId {
    fn from(v: u32) -> Self {
        Self::Number(v as i64)
    }
}

impl From<&str> for RequestId {
    fn from(v: &str) -> Self {
        Self::String(v.to_string())
    }
}

impl From<String> for RequestId {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

/// JSON-RPC 2.0 请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// 固定 `"2.0"`。
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    /// 类型由 `method` 名决定,envelope 层不解析。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Request {
    pub fn new(id: impl Into<RequestId>, method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: crate::JSONRPC_VERSION.to_string(),
            id: id.into(),
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 通知(无 id,不要求响应)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Notification {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: crate::JSONRPC_VERSION.to_string(),
            method: method.into(),
            params,
        }
    }
}

/// 响应正文,只能是 result 或 error 之一。
///
/// 反序列化时按字段存在性区分,内部 tag 模式("status")避免与
/// JSON-RPC 标准歧义。
///
/// `Err` 变体的 `ProtocolError` 装箱,因为其 `ErrorData` 字段较大
/// (~248 bytes vs `Ok` 的 32 bytes),避免每个响应都按最大 variant 分配。
/// 装箱不影响 wire 表现——untagged enum 透过 Box 序列化。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseBody {
    Ok { result: Value },
    Err { error: Box<ProtocolError> },
}

impl ResponseBody {
    pub fn ok(result: Value) -> Self {
        Self::Ok { result }
    }
    pub fn err(error: ProtocolError) -> Self {
        Self::Err {
            error: Box::new(error),
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }

    pub fn is_err(&self) -> bool {
        matches!(self, Self::Err { .. })
    }
}

/// JSON-RPC 2.0 响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(flatten)]
    pub body: ResponseBody,
}

impl Response {
    pub fn ok(id: impl Into<RequestId>, result: Value) -> Self {
        Self {
            jsonrpc: crate::JSONRPC_VERSION.to_string(),
            id: id.into(),
            body: ResponseBody::Ok { result },
        }
    }

    pub fn err(id: impl Into<RequestId>, error: ProtocolError) -> Self {
        Self {
            jsonrpc: crate::JSONRPC_VERSION.to_string(),
            id: id.into(),
            body: ResponseBody::Err {
                error: Box::new(error),
            },
        }
    }

    pub fn result(&self) -> Option<&Value> {
        match &self.body {
            ResponseBody::Ok { result } => Some(result),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<&ProtocolError> {
        match &self.body {
            ResponseBody::Err { error } => Some(error.as_ref()),
            _ => None,
        }
    }
}

/// 统一的 wire 消息枚举,用于反序列化未知方向的消息。
///
/// 业务代码通常直接构造 [`Request`] / [`Response`] / [`Notification`],
/// 此枚举主要给路由器 / 测试工具使用。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcMessage {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

impl RpcMessage {
    pub fn is_request(&self) -> bool {
        matches!(self, Self::Request(_))
    }
    pub fn is_response(&self) -> bool {
        matches!(self, Self::Response(_))
    }
    pub fn is_notification(&self) -> bool {
        matches!(self, Self::Notification(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_with_required_fields() {
        let req = Request::new(1, "init", serde_json::json!({"k": "v"}));
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains(r#""jsonrpc":"2.0""#));
        assert!(s.contains(r#""id":1"#));
        assert!(s.contains(r#""method":"init""#));
        assert!(s.contains(r#""params":{"k":"v"}"#));
    }

    #[test]
    fn request_skips_null_params() {
        let req = Request::new(1, "shutdown", Value::Null);
        let s = serde_json::to_string(&req).unwrap();
        assert!(!s.contains("params"));
    }

    #[test]
    fn request_id_supports_number_and_string() {
        let by_num = serde_json::from_str::<Request>(
            r#"{"jsonrpc":"2.0","id":42,"method":"x","params":null}"#,
        )
        .unwrap();
        assert!(matches!(by_num.id, RequestId::Number(42)));

        let by_str =
            serde_json::from_str::<Request>(r#"{"jsonrpc":"2.0","id":"u1","method":"x"}"#).unwrap();
        assert!(matches!(by_str.id, RequestId::String(ref s) if s == "u1"));
    }

    #[test]
    fn response_with_result_round_trips() {
        let resp = Response::ok(1, serde_json::json!({"ok": true}));
        let s = serde_json::to_string(&resp).unwrap();
        let parsed: Response = serde_json::from_str(&s).unwrap();
        assert!(parsed.body.is_ok());
        assert_eq!(parsed.result().unwrap(), &serde_json::json!({"ok": true}));
    }

    #[test]
    fn response_with_error_round_trips() {
        let resp = Response::err(
            1,
            ProtocolError::new(crate::error::error_codes::METHOD_NOT_FOUND, "unknown"),
        );
        let s = serde_json::to_string(&resp).unwrap();
        let parsed: Response = serde_json::from_str(&s).unwrap();
        assert!(parsed.body.is_err());
        let err = parsed.error().unwrap();
        assert_eq!(err.code, crate::error::error_codes::METHOD_NOT_FOUND);
        assert_eq!(err.message, "unknown");
    }

    #[test]
    fn notification_has_no_id() {
        let n = Notification::new("log", serde_json::json!({"msg": "hi"}));
        let s = serde_json::to_string(&n).unwrap();
        assert!(!s.contains(r#""id":"#));
        assert!(s.contains(r#""method":"log""#));
    }

    #[test]
    fn rpc_message_round_trip_request() {
        let req = Request::new(1, "init", serde_json::json!({}));
        let env = RpcMessage::Request(req.clone());
        let s = serde_json::to_string(&env).unwrap();
        let parsed: RpcMessage = serde_json::from_str(&s).unwrap();
        assert!(parsed.is_request());
    }

    #[test]
    fn rpc_message_round_trip_notification() {
        let n = Notification::new("log", serde_json::json!({}));
        let env = RpcMessage::Notification(n);
        let s = serde_json::to_string(&env).unwrap();
        let parsed: RpcMessage = serde_json::from_str(&s).unwrap();
        assert!(parsed.is_notification());
    }

    #[test]
    fn rpc_message_round_trip_response_ok() {
        let r = Response::ok(1, serde_json::json!({"ok": true}));
        let env = RpcMessage::Response(r);
        let s = serde_json::to_string(&env).unwrap();
        let parsed: RpcMessage = serde_json::from_str(&s).unwrap();
        assert!(parsed.is_response());
    }

    #[test]
    fn rpc_message_round_trip_response_err() {
        let r = Response::err(1, ProtocolError::new(-32000, "x"));
        let env = RpcMessage::Response(r);
        let s = serde_json::to_string(&env).unwrap();
        let parsed: RpcMessage = serde_json::from_str(&s).unwrap();
        match parsed {
            RpcMessage::Response(r) => assert!(r.body.is_err()),
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn request_id_from_helpers_work() {
        let id_n: RequestId = 5i64.into();
        let id_u: RequestId = 5u64.into();
        let id_s: RequestId = "abc".into();
        assert!(matches!(id_n, RequestId::Number(5)));
        assert!(matches!(id_u, RequestId::Number(5)));
        assert!(matches!(id_s, RequestId::String(s) if s == "abc"));
    }
}
