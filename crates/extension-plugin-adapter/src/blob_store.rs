//! Host-owned bounded storage and reverse uploads for provider result blobs.

mod backend;
mod operations;
mod state;

use std::{
    num::TryFromIntError,
    sync::Arc,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use extension_protocol::{
    blob::{BlobOpenParams, BlobOpenResult},
    error::{ProtocolError, error_codes},
    host_blob::{
        HostBlobBeginParams, HostBlobBeginResult, HostBlobFinishParams, HostBlobFinishResult,
        HostBlobWriteParams, HostBlobWriteResult,
    },
};
use parking_lot::RwLock;
use uuid::Uuid;

use backend::{BlobBackend, UploadData};
use state::{BlobStoreState, PendingUpload, StoredBlob};

pub const DEFAULT_MAX_BLOB_BYTES: usize = 32 * 1024 * 1024;
pub const DEFAULT_MAX_TOTAL_BLOB_BYTES: usize = 128 * 1024 * 1024;
pub const DEFAULT_BLOB_SPILL_THRESHOLD_BYTES: usize = 4 * 1024 * 1024;
pub const DEFAULT_BLOB_TTL_MS: u64 = 15 * 60 * 1000;
pub const MAX_BLOB_TTL_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobOwner {
    pub runtime_id: String,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobStoreLimits {
    pub max_blob_bytes: usize,
    pub max_total_bytes: usize,
}
impl Default for BlobStoreLimits {
    fn default() -> Self {
        Self {
            max_blob_bytes: DEFAULT_MAX_BLOB_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BLOB_BYTES,
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum BlobStoreError {
    #[error("blob or upload `{0}` is closed or unknown")]
    Unknown(String),
    #[error("blob or upload `{0}` is owned by another runtime generation")]
    OwnerMismatch(String),
    #[error("blob size {actual} exceeds the host limit {limit}")]
    BlobTooLarge {
        blob_id: String,
        actual: usize,
        limit: usize,
    },
    #[error("host blob storage is full; limit is {limit} bytes")]
    TotalBytesExceeded { limit: usize },
    #[error("upload `{upload_id}` expected sequence {expected}, got {actual}")]
    SequenceMismatch {
        upload_id: String,
        expected: u64,
        actual: u64,
    },
    #[error("upload `{upload_id}` expected {expected} bytes, got {actual}")]
    ExpectedBytesMismatch {
        upload_id: String,
        expected: u64,
        actual: u64,
    },
    #[error("failed to encode or decode blob chunk: {0}")]
    Encoding(String),
    #[error("blob storage I/O failed: {0}")]
    Io(String),
    #[error("protocol error: {0}")]
    Protocol(ProtocolError),
}
impl BlobStoreError {
    fn code(&self) -> i32 {
        match self {
            Self::Unknown(_) | Self::OwnerMismatch(_) => error_codes::RESOURCE_CLOSED,
            Self::BlobTooLarge { .. }
            | Self::TotalBytesExceeded { .. }
            | Self::ExpectedBytesMismatch { .. } => error_codes::DATA_VALUE_OUT_OF_RANGE,
            Self::SequenceMismatch { .. } | Self::Encoding(_) => error_codes::DATA_INVALID_ENCODING,
            Self::Io(_) => error_codes::IO_WRITE_FAILED,
            Self::Protocol(error) => error.code,
        }
    }
}
impl From<BlobStoreError> for ProtocolError {
    fn from(error: BlobStoreError) -> Self {
        ProtocolError::new(error.code(), error.to_string())
    }
}
impl From<TryFromIntError> for BlobStoreError {
    fn from(error: TryFromIntError) -> Self {
        Self::Encoding(error.to_string())
    }
}

#[derive(Clone)]
pub struct BlobStore {
    limits: BlobStoreLimits,
    spill_threshold_bytes: usize,
    default_ttl: Duration,
    state: Arc<RwLock<BlobStoreState>>,
}
impl Default for BlobStore {
    fn default() -> Self {
        Self::new(BlobStoreLimits::default())
    }
}
impl BlobStore {
    pub fn new(limits: BlobStoreLimits) -> Self {
        Self {
            limits,
            spill_threshold_bytes: DEFAULT_BLOB_SPILL_THRESHOLD_BYTES,
            default_ttl: Duration::from_millis(DEFAULT_BLOB_TTL_MS),
            state: Arc::default(),
        }
    }
    pub fn with_spill_threshold(mut self, bytes: usize) -> Self {
        self.spill_threshold_bytes = bytes;
        self
    }
    pub fn with_default_ttl(mut self, ttl: Duration) -> Self {
        self.default_ttl = ttl.min(Duration::from_millis(MAX_BLOB_TTL_MS));
        self
    }

    pub fn open(
        &self,
        owner: &BlobOwner,
        params: &BlobOpenParams,
        data: impl Into<Arc<[u8]>>,
    ) -> Result<BlobOpenResult, BlobStoreError> {
        let data = data.into();
        self.check_blob_size("", data.len())?;
        let blob_id = new_id("host-blob");
        let mut state = self.state.write();
        state.remove_expired(Instant::now());
        state.reserve_with_lru(data.len(), self.limits.max_total_bytes)?;
        state.access_clock += 1;
        let mut blob = StoredBlob::memory(
            owner.clone(),
            data,
            params,
            Some(Instant::now() + self.default_ttl),
        );
        blob.last_access = state.access_clock;
        let total_bytes = blob.backend.len();
        state.insert_blob(blob_id.clone(), blob);
        Ok(BlobOpenResult {
            blob_id,
            total_bytes: Some(total_bytes as u64),
            content_type: params.content_type.clone(),
        })
    }

    pub fn begin_upload(
        &self,
        owner: &BlobOwner,
        params: HostBlobBeginParams,
    ) -> Result<HostBlobBeginResult, BlobStoreError> {
        if let Some(expected) = params.expected_bytes {
            self.check_blob_size("", usize::try_from(expected)?)?;
        }
        let ttl = Duration::from_millis(
            params
                .ttl_ms
                .unwrap_or(self.default_ttl.as_millis() as u64)
                .min(MAX_BLOB_TTL_MS),
        );
        let upload_id = new_id("host-upload");
        let mut state = self.state.write();
        state.remove_expired(Instant::now());
        state.uploads.insert(
            upload_id.clone(),
            PendingUpload::new(owner.clone(), params, Some(Instant::now() + ttl)),
        );
        Ok(HostBlobBeginResult {
            upload_id,
            max_bytes: self.limits.max_blob_bytes as u64,
        })
    }

    pub fn write_upload(
        &self,
        owner: &BlobOwner,
        params: HostBlobWriteParams,
    ) -> Result<HostBlobWriteResult, BlobStoreError> {
        let chunk = BASE64
            .decode(&params.data)
            .map_err(|error| BlobStoreError::Encoding(error.to_string()))?;
        if chunk.len() != params.bytes_written as usize {
            return Err(BlobStoreError::Encoding(
                "decoded length does not match bytes_written".into(),
            ));
        }
        let mut state = self.state.write();
        state.remove_expired(Instant::now());
        let next = {
            let upload = state
                .uploads
                .get(&params.upload_id)
                .ok_or_else(|| BlobStoreError::Unknown(params.upload_id.clone()))?;
            upload.ensure_owner(&params.upload_id, owner)?;
            upload.ensure_sequence(&params.upload_id, params.sequence)?;
            upload.next_len(chunk.len(), self.limits.max_blob_bytes)?
        };
        let victims = state.plan_reclaim(chunk.len(), self.limits.max_total_bytes)?;
        state
            .uploads
            .get_mut(&params.upload_id)
            .expect("validated upload")
            .append(&chunk, next, self.spill_threshold_bytes)?;
        state.commit_reclaim(victims);
        state.commit_pending(chunk.len());
        Ok(HostBlobWriteResult {
            total_bytes: next as u64,
        })
    }

    pub fn finish_upload(
        &self,
        owner: &BlobOwner,
        params: HostBlobFinishParams,
    ) -> Result<HostBlobFinishResult, BlobStoreError> {
        let mut state = self.state.write();
        state.remove_expired(Instant::now());
        {
            let upload = state
                .uploads
                .get(&params.upload_id)
                .ok_or_else(|| BlobStoreError::Unknown(params.upload_id.clone()))?;
            upload.ensure_owner(&params.upload_id, owner)?;
            upload.ensure_expected(&params.upload_id)?;
        }
        let upload = state
            .uploads
            .remove(&params.upload_id)
            .expect("validated upload");
        let result = HostBlobFinishResult {
            blob_id: new_id("host-blob"),
            total_bytes: upload.total_bytes as u64,
            content_type: upload.content_type.clone(),
        };
        state.publish_upload(result.blob_id.clone(), upload);
        Ok(result)
    }

    pub fn abort_upload(&self, owner: &BlobOwner, upload_id: &str) -> Result<(), BlobStoreError> {
        let mut state = self.state.write();
        state.remove_expired(Instant::now());
        let Some(upload) = state.uploads.get(upload_id) else {
            return Ok(());
        };
        upload.ensure_owner(upload_id, owner)?;
        state.remove_upload(upload_id);
        Ok(())
    }

    fn check_blob_size(&self, id: &str, actual: usize) -> Result<(), BlobStoreError> {
        if actual > self.limits.max_blob_bytes {
            Err(BlobStoreError::BlobTooLarge {
                blob_id: id.into(),
                actual,
                limit: self.limits.max_blob_bytes,
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobInfo {
    pub owner: BlobOwner,
    pub total_bytes: usize,
    pub content_type: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub read_offset: usize,
    pub spilled: bool,
}
fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4().simple())
}
