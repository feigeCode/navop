use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

pub use ipc::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};

#[derive(Clone, Debug, Deserialize)]
pub struct DbConnectionConfig {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub extra_params: HashMap<String, String>,
}

pub fn connect_config(params: &Value) -> anyhow::Result<DbConnectionConfig> {
    serde_json::from_value(params["config"].clone()).map_err(Into::into)
}

pub fn string_param<'a>(params: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("{key} is required"))
}
