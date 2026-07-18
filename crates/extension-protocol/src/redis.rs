//! Redis native sidecar wire contract.

use serde::{Deserialize, Serialize};

use crate::blob::WireBytes;
use crate::conn::ConnId;

pub const MAX_PIPELINE_COMMANDS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedisConnectionConfig {
    pub host: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default)]
    pub database: u8,
    #[serde(default)]
    pub use_tls: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedisCommandParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<u8>,
    pub args: Vec<WireBytes>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RedisCommandResult {
    pub value: RedisRespValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum RedisRespValue {
    Nil,
    Integer(i64),
    Double(f64),
    Boolean(bool),
    Bytes(WireBytes),
    SimpleString(String),
    Error(String),
    Array(Vec<RedisRespValue>),
    Map(Vec<(RedisRespValue, RedisRespValue)>),
    Set(Vec<RedisRespValue>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedisPipelineParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<u8>,
    pub commands: Vec<Vec<WireBytes>>,
}

impl RedisPipelineParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.commands.is_empty() {
            return Err("pipeline requires at least one command");
        }
        if self.commands.len() > MAX_PIPELINE_COMMANDS {
            return Err("pipeline command limit exceeded");
        }
        if self.commands.iter().any(Vec::is_empty) {
            return Err("pipeline commands cannot be empty");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RedisPipelineResult {
    pub values: Vec<RedisRespValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedisPubSubOpenParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "target", rename_all = "snake_case")]
pub enum RedisPubSubControl {
    Subscribe(WireBytes),
    PSubscribe(WireBytes),
    Unsubscribe(WireBytes),
    PUnsubscribe(WireBytes),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedisPubSubControlParams {
    pub conn_id: ConnId,
    pub stream_id: String,
    pub control: RedisPubSubControl,
}
