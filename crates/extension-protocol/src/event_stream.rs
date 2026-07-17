//! 通用有界事件流。
//!
//! 高频事件不直接堆积在无界 JSON-RPC notification channel；driver 持有有界
//! buffer，host 通过 read 拉取 batch，并能在 UI/调用方关闭时明确释放资源。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::conn::ConnId;

pub type EventStreamId = String;

pub const DEFAULT_EVENT_MAX_EVENTS: u32 = 128;
pub const MAX_EVENT_MAX_EVENTS: u32 = 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventOpenParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conn_id: Option<ConnId>,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventOpenResult {
    pub stream_id: EventStreamId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventReadParams {
    pub stream_id: EventStreamId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_events: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u32>,
}

impl EventReadParams {
    pub fn effective_max_events(&self) -> u32 {
        self.max_events
            .unwrap_or(DEFAULT_EVENT_MAX_EVENTS)
            .clamp(1, MAX_EVENT_MAX_EVENTS)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventReadResult {
    #[serde(default)]
    pub events: Vec<Value>,
    #[serde(default)]
    pub closed: bool,
    /// buffer overflow 时累计丢弃的事件数量；正常为 0。
    #[serde(default)]
    pub dropped_count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventCloseParams {
    pub stream_id: EventStreamId,
}
