use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Instant,
};

use extension_protocol::{
    blob::{BlobId, BlobOpenParams},
    host_blob::HostBlobBeginParams,
};

use super::{BlobBackend, BlobOwner, BlobStoreError, UploadData};

pub(super) struct StoredBlob {
    pub(super) owner: BlobOwner,
    pub(super) backend: BlobBackend,
    pub(super) offset: usize,
    pub(super) content_type: Option<String>,
    pub(super) metadata: Option<serde_json::Value>,
    pub(super) expires_at: Option<Instant>,
    pub(super) last_access: u64,
}

impl StoredBlob {
    pub(super) fn memory(
        owner: BlobOwner,
        data: Arc<[u8]>,
        params: &BlobOpenParams,
        expires_at: Option<Instant>,
    ) -> Self {
        Self {
            owner,
            backend: BlobBackend::Memory(data),
            offset: 0,
            content_type: params.content_type.clone(),
            metadata: params.metadata.clone(),
            expires_at,
            last_access: 0,
        }
    }
}

pub(super) struct PendingUpload {
    pub(super) owner: BlobOwner,
    pub(super) sequence: u64,
    pub(super) total_bytes: usize,
    pub(super) expected_bytes: Option<u64>,
    pub(super) content_type: Option<String>,
    pub(super) metadata: Option<serde_json::Value>,
    pub(super) expires_at: Option<Instant>,
    pub(super) data: UploadData,
}

impl PendingUpload {
    pub(super) fn new(
        owner: BlobOwner,
        params: HostBlobBeginParams,
        expires_at: Option<Instant>,
    ) -> Self {
        Self {
            owner,
            sequence: 0,
            total_bytes: 0,
            expected_bytes: params.expected_bytes,
            content_type: params.content_type,
            metadata: params.metadata,
            expires_at,
            data: UploadData::Memory(Vec::new()),
        }
    }

    pub(super) fn ensure_owner(&self, id: &str, owner: &BlobOwner) -> Result<(), BlobStoreError> {
        if self.owner == *owner {
            Ok(())
        } else {
            Err(BlobStoreError::OwnerMismatch(id.into()))
        }
    }

    pub(super) fn ensure_sequence(&self, id: &str, actual: u64) -> Result<(), BlobStoreError> {
        if self.sequence == actual {
            Ok(())
        } else {
            Err(BlobStoreError::SequenceMismatch {
                upload_id: id.into(),
                expected: self.sequence,
                actual,
            })
        }
    }

    pub(super) fn next_len(&self, chunk: usize, limit: usize) -> Result<usize, BlobStoreError> {
        let next =
            self.total_bytes
                .checked_add(chunk)
                .ok_or_else(|| BlobStoreError::BlobTooLarge {
                    blob_id: String::new(),
                    actual: usize::MAX,
                    limit,
                })?;
        if next <= limit {
            Ok(next)
        } else {
            Err(BlobStoreError::BlobTooLarge {
                blob_id: String::new(),
                actual: next,
                limit,
            })
        }
    }

    pub(super) fn append(
        &mut self,
        chunk: &[u8],
        next: usize,
        spill_threshold: usize,
    ) -> Result<(), BlobStoreError> {
        self.data.append(chunk, next, spill_threshold)?;
        self.total_bytes = next;
        self.sequence += 1;
        Ok(())
    }

    pub(super) fn ensure_expected(&self, id: &str) -> Result<(), BlobStoreError> {
        if let Some(expected) = self.expected_bytes
            && expected != self.total_bytes as u64
        {
            return Err(BlobStoreError::ExpectedBytesMismatch {
                upload_id: id.into(),
                expected,
                actual: self.total_bytes as u64,
            });
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct BlobStoreState {
    pub(super) blobs: HashMap<BlobId, StoredBlob>,
    pub(super) uploads: HashMap<String, PendingUpload>,
    pub(super) total_bytes: usize,
    pub(super) pending_bytes: usize,
    pub(super) access_clock: u64,
}

impl BlobStoreState {
    pub(super) fn plan_reclaim(
        &self,
        bytes: usize,
        limit: usize,
    ) -> Result<Vec<BlobId>, BlobStoreError> {
        let required = self
            .total_bytes
            .checked_add(self.pending_bytes)
            .and_then(|used| used.checked_add(bytes));
        if required.is_some_and(|required| required <= limit) {
            return Ok(Vec::new());
        }
        self.reclaim_victims(bytes, limit)
    }

    pub(super) fn reserve_with_lru(
        &mut self,
        bytes: usize,
        limit: usize,
    ) -> Result<(), BlobStoreError> {
        let victims = self.plan_reclaim(bytes, limit)?;
        self.commit_reclaim(victims);
        Ok(())
    }

    fn reclaim_victims(&self, bytes: usize, limit: usize) -> Result<Vec<BlobId>, BlobStoreError> {
        let mut reclaimable = self
            .blobs
            .iter()
            .map(|(id, blob)| (id.clone(), blob.backend.len(), blob.last_access))
            .collect::<Vec<_>>();
        reclaimable.sort_by_key(|(_, _, last_access)| *last_access);

        let mut projected = self.total_bytes;
        let mut victims = Vec::new();
        for (id, size, _) in reclaimable {
            projected = projected.saturating_sub(size);
            victims.push(id);
            if projected
                .checked_add(self.pending_bytes)
                .and_then(|used| used.checked_add(bytes))
                .is_some_and(|required| required <= limit)
            {
                return Ok(victims);
            }
        }
        Err(BlobStoreError::TotalBytesExceeded { limit })
    }

    pub(super) fn commit_reclaim(&mut self, victims: Vec<BlobId>) {
        for victim in victims {
            self.remove_blob(&victim);
        }
    }

    pub(super) fn commit_pending(&mut self, bytes: usize) {
        self.pending_bytes += bytes;
    }

    pub(super) fn insert_blob(&mut self, id: BlobId, blob: StoredBlob) {
        self.total_bytes += blob.backend.len();
        self.blobs.insert(id, blob);
    }

    pub(super) fn publish_upload(&mut self, id: BlobId, upload: PendingUpload) {
        self.pending_bytes = self.pending_bytes.saturating_sub(upload.total_bytes);
        let blob = StoredBlob {
            owner: upload.owner,
            backend: upload.data.seal(upload.total_bytes),
            offset: 0,
            content_type: upload.content_type,
            metadata: upload.metadata,
            expires_at: upload.expires_at,
            last_access: 0,
        };
        self.insert_blob(id, blob);
    }

    pub(super) fn remove_upload(&mut self, id: &str) -> bool {
        let Some(upload) = self.uploads.remove(id) else {
            return false;
        };
        self.pending_bytes = self.pending_bytes.saturating_sub(upload.total_bytes);
        true
    }

    pub(super) fn remove_blob(&mut self, id: &str) -> bool {
        let Some(blob) = self.blobs.remove(id) else {
            return false;
        };
        self.total_bytes = self.total_bytes.saturating_sub(blob.backend.len());
        true
    }

    pub(super) fn remove_expired(&mut self, now: Instant) -> usize {
        let blobs = self
            .blobs
            .iter()
            .filter(|(_, blob)| blob.expires_at.is_some_and(|expiry| expiry <= now))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let uploads = self
            .uploads
            .iter()
            .filter(|(_, upload)| upload.expires_at.is_some_and(|expiry| expiry <= now))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let count = blobs.len() + uploads.len();
        for id in blobs {
            self.remove_blob(&id);
        }
        for id in uploads {
            self.remove_upload(&id);
        }
        count
    }

    pub(super) fn evict_lru(&mut self) -> bool {
        let Some(id) = self
            .blobs
            .iter()
            .min_by_key(|(_, blob)| blob.last_access)
            .map(|(id, _)| id.clone())
        else {
            return false;
        };
        self.remove_blob(&id)
    }

    pub(super) fn runtime_totals(&self) -> BTreeMap<String, usize> {
        let mut totals = BTreeMap::new();
        for blob in self.blobs.values() {
            *totals.entry(blob.owner.runtime_id.clone()).or_default() += blob.backend.len();
        }
        for upload in self.uploads.values() {
            *totals.entry(upload.owner.runtime_id.clone()).or_default() += upload.total_bytes;
        }
        totals
    }
}
