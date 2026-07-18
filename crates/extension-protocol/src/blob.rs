//! 通用二进制 blob 流。
//!
//! 普通 JSON-RPC result 适合小值；MongoDB BSON 和 Redis binary value 可能超过
//! framing 上限，因此通过 blob id 分块读取。每次 read 都由 caller 给出上限，
//! 避免 receiver 按远端声明长度一次性分配大块内存。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::conn::ConnId;

pub type BlobId = String;

pub const INLINE_BLOB_THRESHOLD_BYTES: u64 = 4 * 1024 * 1024;
pub const DEFAULT_BLOB_CHUNK_BYTES: u32 = 256 * 1024;
pub const MAX_BLOB_CHUNK_BYTES: u32 = 4 * 1024 * 1024;

pub fn should_stream_blob(bytes: u64) -> bool {
    bytes > INLINE_BLOB_THRESHOLD_BYTES
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "encoding", content = "value", rename_all = "snake_case")]
pub enum WireBytes {
    Utf8(String),
    Base64(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BlobOpenParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conn_id: Option<ConnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobOpenResult {
    pub blob_id: BlobId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobReadParams {
    pub blob_id: BlobId,
    /// receiver 能接受的最大 raw bytes，driver 可以返回更少但不能更多。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u32>,
}

impl BlobReadParams {
    pub fn effective_max_bytes(&self) -> u32 {
        self.max_bytes
            .unwrap_or(DEFAULT_BLOB_CHUNK_BYTES)
            .clamp(1, MAX_BLOB_CHUNK_BYTES)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobReadResult {
    /// 当前 chunk 的 Base64 数据；避免 JSON 对任意二进制做 UTF-8 假设。
    #[serde(default)]
    pub data: String,
    pub bytes_read: u32,
    #[serde(default)]
    pub done: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobCloseParams {
    pub blob_id: BlobId,
}
