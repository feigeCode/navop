use std::time::Instant;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use extension_protocol::blob::{BlobCloseParams, BlobReadParams, BlobReadResult};

use super::{BlobInfo, BlobOwner, BlobStore, BlobStoreError};

impl BlobStore {
    pub fn read(
        &self,
        owner: &BlobOwner,
        params: &BlobReadParams,
    ) -> Result<BlobReadResult, BlobStoreError> {
        self.reclaim_expired();
        let mut state = self.state.write();
        state.access_clock += 1;
        let clock = state.access_clock;
        let blob = state
            .blobs
            .get_mut(&params.blob_id)
            .ok_or_else(|| BlobStoreError::Unknown(params.blob_id.clone()))?;
        ensure_owner(&params.blob_id, &blob.owner, owner)?;
        blob.last_access = clock;
        let start = blob.offset.min(blob.backend.len());
        let end = (start + params.effective_max_bytes() as usize).min(blob.backend.len());
        let bytes = blob.backend.read(start, end)?;
        blob.offset = end;
        Ok(BlobReadResult {
            data: BASE64.encode(bytes),
            bytes_read: u32::try_from(end - start)?,
            done: end == blob.backend.len(),
        })
    }

    pub fn close(&self, owner: &BlobOwner, params: &BlobCloseParams) -> Result<(), BlobStoreError> {
        self.remove_owned_blob(owner, &params.blob_id)
    }

    pub fn remove_owned_blob(
        &self,
        owner: &BlobOwner,
        blob_id: &str,
    ) -> Result<(), BlobStoreError> {
        let mut state = self.state.write();
        let Some(blob) = state.blobs.get(blob_id) else {
            return Ok(());
        };
        ensure_owner(blob_id, &blob.owner, owner)?;
        state.remove_blob(blob_id);
        Ok(())
    }

    pub fn info(&self, owner: &BlobOwner, blob_id: &str) -> Result<BlobInfo, BlobStoreError> {
        self.reclaim_expired();
        let state = self.state.read();
        let blob = state
            .blobs
            .get(blob_id)
            .ok_or_else(|| BlobStoreError::Unknown(blob_id.into()))?;
        ensure_owner(blob_id, &blob.owner, owner)?;
        Ok(BlobInfo {
            owner: blob.owner.clone(),
            total_bytes: blob.backend.len(),
            content_type: blob.content_type.clone(),
            metadata: blob.metadata.clone(),
            read_offset: blob.offset,
            spilled: blob.backend.spilled(),
        })
    }

    pub fn remove_generation(&self, runtime_id: &str, generation: u64) {
        self.remove_matching(|owner| {
            owner.runtime_id == runtime_id && owner.generation == generation
        });
    }

    pub fn remove_runtime(&self, runtime_id: &str) {
        self.remove_matching(|owner| owner.runtime_id == runtime_id);
    }

    fn remove_matching(&self, predicate: impl Fn(&BlobOwner) -> bool) {
        let mut state = self.state.write();
        let blobs = state
            .blobs
            .iter()
            .filter(|(_, blob)| predicate(&blob.owner))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let uploads = state
            .uploads
            .iter()
            .filter(|(_, upload)| predicate(&upload.owner))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in blobs {
            state.remove_blob(&id);
        }
        for id in uploads {
            state.remove_upload(&id);
        }
    }

    pub fn reclaim_expired(&self) -> usize {
        self.state.write().remove_expired(Instant::now())
    }

    pub fn len(&self) -> usize {
        self.state.read().blobs.len()
    }

    pub fn pending_len(&self) -> usize {
        self.state.read().uploads.len()
    }

    pub fn is_empty(&self) -> bool {
        let state = self.state.read();
        state.blobs.is_empty() && state.uploads.is_empty()
    }

    pub fn total_bytes(&self) -> usize {
        let state = self.state.read();
        state.total_bytes + state.pending_bytes
    }

    pub fn runtime_total_bytes(&self) -> std::collections::BTreeMap<String, usize> {
        self.state.read().runtime_totals()
    }

    pub fn evict_until_total_bytes(&self, target: usize) -> usize {
        let mut state = self.state.write();
        let mut removed = 0;
        while state.total_bytes + state.pending_bytes > target && state.evict_lru() {
            removed += 1;
        }
        removed
    }
}

fn ensure_owner(id: &str, actual: &BlobOwner, expected: &BlobOwner) -> Result<(), BlobStoreError> {
    if actual == expected {
        Ok(())
    } else {
        Err(BlobStoreError::OwnerMismatch(id.into()))
    }
}
