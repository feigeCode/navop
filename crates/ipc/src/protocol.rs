use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(default = "default_version")]
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: default_version(),
            id,
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    #[serde(default = "default_version")]
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn result(id: u64, result: Value) -> Self {
        Self {
            jsonrpc: default_version(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: u64, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: default_version(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub data: Option<Value>,
}

fn default_version() -> String {
    "2.0".to_string()
}
