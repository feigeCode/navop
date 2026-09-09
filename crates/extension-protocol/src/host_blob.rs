//! Reverse Host API contract for provider uploads into host-owned blob storage.
//!
//! Upload handles are intentionally distinct from readable blob ids. A
//! provider cannot read or publish a partially written upload and never gets to
//! choose the host-side owner or filesystem path.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::blob::BlobId;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HostBlobBeginParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostBlobBeginResult {
    pub upload_id: String,
    pub max_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostBlobWriteParams {
    pub upload_id: String,
    pub sequence: u64,
    /// Base64-encoded raw bytes.
    pub data: String,
    /// Declared decoded length, checked before accepting the chunk.
    pub bytes_written: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostBlobWriteResult {
    pub total_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostBlobFinishParams {
    pub upload_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostBlobFinishResult {
    pub blob_id: BlobId,
    pub total_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostBlobAbortParams {
    pub upload_id: String,
}
