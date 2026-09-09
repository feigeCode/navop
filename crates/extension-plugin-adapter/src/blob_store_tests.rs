use crate::blob_store::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use extension_protocol::blob::{BlobOpenParams, BlobReadParams};
use extension_protocol::host_blob::{
    HostBlobBeginParams, HostBlobFinishParams, HostBlobWriteParams, HostBlobWriteResult,
};
use std::{sync::Arc, thread, time::Duration};

fn owner(generation: u64) -> BlobOwner {
    BlobOwner {
        runtime_id: "provider::main".into(),
        generation,
    }
}
fn write(
    store: &BlobStore,
    owner: &BlobOwner,
    upload_id: &str,
    sequence: u64,
    bytes: &[u8],
) -> Result<HostBlobWriteResult, BlobStoreError> {
    store.write_upload(
        owner,
        HostBlobWriteParams {
            upload_id: upload_id.into(),
            sequence,
            data: BASE64.encode(bytes),
            bytes_written: bytes.len() as u32,
        },
    )
}

#[test]
fn reverse_upload_is_hidden_until_finished_and_readable_afterwards() {
    let store = BlobStore::default();
    let owner = owner(0);
    let upload = store
        .begin_upload(
            &owner,
            HostBlobBeginParams {
                expected_bytes: Some(6),
                ..Default::default()
            },
        )
        .unwrap();
    write(&store, &owner, &upload.upload_id, 0, b"abc").unwrap();
    assert!(matches!(
        store.info(&owner, &upload.upload_id),
        Err(BlobStoreError::Unknown(_))
    ));
    write(&store, &owner, &upload.upload_id, 1, b"def").unwrap();
    let finished = store
        .finish_upload(
            &owner,
            HostBlobFinishParams {
                upload_id: upload.upload_id,
            },
        )
        .unwrap();
    let result = store
        .read(
            &owner,
            &BlobReadParams {
                blob_id: finished.blob_id,
                max_bytes: Some(8),
            },
        )
        .unwrap();
    assert_eq!(BASE64.encode(b"abcdef"), result.data);
    assert!(result.done);
}

#[test]
fn upload_rejects_bad_sequence_length_and_expected_size() {
    let store = BlobStore::default();
    let owner = owner(0);
    let upload = store
        .begin_upload(
            &owner,
            HostBlobBeginParams {
                expected_bytes: Some(3),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(matches!(
        write(&store, &owner, &upload.upload_id, 1, b"a"),
        Err(BlobStoreError::SequenceMismatch { .. })
    ));
    assert!(matches!(
        store.write_upload(
            &owner,
            HostBlobWriteParams {
                upload_id: upload.upload_id.clone(),
                sequence: 0,
                data: BASE64.encode(b"a"),
                bytes_written: 2
            }
        ),
        Err(BlobStoreError::Encoding(_))
    ));
    write(&store, &owner, &upload.upload_id, 0, b"ab").unwrap();
    assert!(matches!(
        store.finish_upload(
            &owner,
            HostBlobFinishParams {
                upload_id: upload.upload_id
            }
        ),
        Err(BlobStoreError::ExpectedBytesMismatch { .. })
    ));
}

#[test]
fn abort_is_idempotent_and_generation_cleanup_removes_pending_and_sealed() {
    let store = BlobStore::default();
    let old = owner(0);
    let current = owner(1);
    let aborted = store.begin_upload(&old, Default::default()).unwrap();
    write(&store, &old, &aborted.upload_id, 0, b"a").unwrap();
    store.abort_upload(&old, &aborted.upload_id).unwrap();
    store.abort_upload(&old, &aborted.upload_id).unwrap();
    let pending = store.begin_upload(&old, Default::default()).unwrap();
    write(&store, &old, &pending.upload_id, 0, b"old").unwrap();
    let sealed_upload = store.begin_upload(&old, Default::default()).unwrap();
    write(&store, &old, &sealed_upload.upload_id, 0, b"old").unwrap();
    let sealed = store
        .finish_upload(
            &old,
            HostBlobFinishParams {
                upload_id: sealed_upload.upload_id,
            },
        )
        .unwrap();
    let kept = store
        .open(&current, &BlobOpenParams::default(), b"new".to_vec())
        .unwrap();
    store.remove_generation(&old.runtime_id, 0);
    assert!(store.is_empty() == false);
    assert_eq!(0, store.pending_len());
    assert!(store.info(&old, &sealed.blob_id).is_err());
    assert!(store.info(&current, &kept.blob_id).is_ok());
}

#[test]
fn expired_upload_rejects_writes() {
    let store = BlobStore::default();
    let owner = owner(0);
    let upload = store
        .begin_upload(
            &owner,
            HostBlobBeginParams {
                ttl_ms: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(matches!(
        write(&store, &owner, &upload.upload_id, 0, b"expired"),
        Err(BlobStoreError::Unknown(_))
    ));
    assert_eq!(0, store.total_bytes());
}

#[test]
fn disk_spilling_reads_across_chunks_and_close_releases_storage() {
    let store = BlobStore::new(BlobStoreLimits {
        max_blob_bytes: 32,
        max_total_bytes: 32,
    })
    .with_spill_threshold(4);
    let owner = owner(0);
    let upload = store
        .begin_upload(&owner, HostBlobBeginParams::default())
        .unwrap();
    let bytes = b"abcdefghij".to_vec();
    write(&store, &owner, &upload.upload_id, 0, &bytes).unwrap();
    let finished = store
        .finish_upload(
            &owner,
            HostBlobFinishParams {
                upload_id: upload.upload_id,
            },
        )
        .unwrap();
    assert!(store.info(&owner, &finished.blob_id).unwrap().spilled);
    let first = store
        .read(
            &owner,
            &BlobReadParams {
                blob_id: finished.blob_id.clone(),
                max_bytes: Some(6),
            },
        )
        .unwrap();
    let second = store
        .read(
            &owner,
            &BlobReadParams {
                blob_id: finished.blob_id.clone(),
                max_bytes: Some(6),
            },
        )
        .unwrap();
    assert_eq!(BASE64.encode(b"abcdef"), first.data);
    assert_eq!(BASE64.encode(b"ghij"), second.data);
    assert!(second.done);
    store
        .close(
            &owner,
            &extension_protocol::blob::BlobCloseParams {
                blob_id: finished.blob_id,
            },
        )
        .unwrap();
    assert_eq!(0, store.total_bytes());
}

#[test]
fn owner_and_quota_are_enforced_for_uploads() {
    let store = BlobStore::new(BlobStoreLimits {
        max_blob_bytes: 4,
        max_total_bytes: 4,
    });
    let owner_value = owner(0);
    let other = owner(1);
    let upload = store
        .begin_upload(&owner_value, Default::default())
        .unwrap();
    assert!(matches!(
        write(&store, &other, &upload.upload_id, 0, b"a"),
        Err(BlobStoreError::OwnerMismatch(_))
    ));
    write(&store, &owner_value, &upload.upload_id, 0, b"1234").unwrap();
    assert!(matches!(
        write(&store, &owner_value, &upload.upload_id, 1, b"5"),
        Err(BlobStoreError::BlobTooLarge { .. })
    ));
}

#[test]
fn pending_bytes_count_towards_runtime_and_global_quota() {
    let store = BlobStore::new(BlobStoreLimits {
        max_blob_bytes: 8,
        max_total_bytes: 4,
    });
    let owner = owner(0);
    let first = store.begin_upload(&owner, Default::default()).unwrap();
    let second = store.begin_upload(&owner, Default::default()).unwrap();
    write(&store, &owner, &first.upload_id, 0, b"123").unwrap();
    assert_eq!(Some(&3), store.runtime_total_bytes().get(&owner.runtime_id));
    assert!(matches!(
        write(&store, &owner, &second.upload_id, 0, b"12"),
        Err(BlobStoreError::TotalBytesExceeded { .. })
    ));
}

#[test]
fn failed_upload_reservation_does_not_evict_existing_blobs() {
    let store = BlobStore::new(BlobStoreLimits {
        max_blob_bytes: 8,
        max_total_bytes: 8,
    });
    let owner = owner(0);
    let existing = store
        .open(&owner, &BlobOpenParams::default(), b"kept".to_vec())
        .unwrap();
    let reserved = store.begin_upload(&owner, Default::default()).unwrap();
    write(&store, &owner, &reserved.upload_id, 0, b"1234").unwrap();
    let upload = store.begin_upload(&owner, Default::default()).unwrap();

    assert!(matches!(
        write(&store, &owner, &upload.upload_id, 0, b"extra"),
        Err(BlobStoreError::TotalBytesExceeded { .. })
    ));
    assert!(store.info(&owner, &existing.blob_id).is_ok());
    assert_eq!(1, store.len());
    assert_eq!(2, store.pending_len());
    assert_eq!(8, store.total_bytes());
}

#[test]
fn concurrent_finish_publishes_exactly_once() {
    let store = Arc::new(BlobStore::default());
    let owner = owner(0);
    let upload = store.begin_upload(&owner, Default::default()).unwrap();
    write(&store, &owner, &upload.upload_id, 0, b"once").unwrap();
    let finishes = (0..2)
        .map(|_| {
            let store = store.clone();
            let owner = owner.clone();
            let upload_id = upload.upload_id.clone();
            thread::spawn(move || store.finish_upload(&owner, HostBlobFinishParams { upload_id }))
        })
        .map(|task| task.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(1, finishes.iter().filter(|result| result.is_ok()).count());
    assert_eq!(1, store.len());
    assert_eq!(4, store.total_bytes());
}

#[test]
fn owner_mismatch_abort_does_not_remove_upload() {
    let store = BlobStore::default();
    let owner_value = owner(0);
    let other = owner(1);
    let upload = store
        .begin_upload(&owner_value, Default::default())
        .unwrap();
    write(&store, &owner_value, &upload.upload_id, 0, b"kept").unwrap();
    assert!(matches!(
        store.abort_upload(&other, &upload.upload_id),
        Err(BlobStoreError::OwnerMismatch(_))
    ));
    assert_eq!(1, store.pending_len());
    store.abort_upload(&owner_value, &upload.upload_id).unwrap();
}

#[test]
fn open_blobs_expire_and_lru_evicts_the_least_recently_used() {
    let expiring = BlobStore::new(BlobStoreLimits {
        max_blob_bytes: 8,
        max_total_bytes: 8,
    })
    .with_default_ttl(Duration::from_millis(1));
    let owner = owner(0);
    let blob = expiring
        .open(&owner, &BlobOpenParams::default(), b"x".to_vec())
        .unwrap();
    thread::sleep(Duration::from_millis(5));
    assert!(matches!(
        expiring.info(&owner, &blob.blob_id),
        Err(BlobStoreError::Unknown(_))
    ));

    let store = BlobStore::new(BlobStoreLimits {
        max_blob_bytes: 4,
        max_total_bytes: 6,
    });
    let first = store
        .open(&owner, &BlobOpenParams::default(), b"aa".to_vec())
        .unwrap();
    let second = store
        .open(&owner, &BlobOpenParams::default(), b"bb".to_vec())
        .unwrap();
    store
        .read(
            &owner,
            &BlobReadParams {
                blob_id: first.blob_id.clone(),
                max_bytes: Some(1),
            },
        )
        .unwrap();
    store
        .open(&owner, &BlobOpenParams::default(), b"cccc".to_vec())
        .unwrap();
    assert!(store.info(&owner, &first.blob_id).is_ok());
    assert!(matches!(
        store.info(&owner, &second.blob_id),
        Err(BlobStoreError::Unknown(_))
    ));
}

#[test]
fn invalid_base64_and_oversized_expected_length_are_rejected() {
    let store = BlobStore::new(BlobStoreLimits {
        max_blob_bytes: 4,
        max_total_bytes: 8,
    });
    let owner = owner(0);
    assert!(matches!(
        store.begin_upload(
            &owner,
            HostBlobBeginParams {
                expected_bytes: Some(5),
                ..Default::default()
            }
        ),
        Err(BlobStoreError::BlobTooLarge { .. })
    ));
    let upload = store.begin_upload(&owner, Default::default()).unwrap();
    assert!(matches!(
        store.write_upload(
            &owner,
            HostBlobWriteParams {
                upload_id: upload.upload_id,
                sequence: 0,
                data: "***".into(),
                bytes_written: 1,
            }
        ),
        Err(BlobStoreError::Encoding(_))
    ));
}
