use extension_protocol::blob::{
    BlobReadParams, BlobReadResult, DEFAULT_BLOB_CHUNK_BYTES, INLINE_BLOB_THRESHOLD_BYTES,
    MAX_BLOB_CHUNK_BYTES, WireBytes, should_stream_blob,
};
use extension_protocol::event_stream::{
    DEFAULT_EVENT_MAX_EVENTS, EventReadParams, EventReadResult, MAX_EVENT_MAX_EVENTS,
};
use extension_protocol::host_blob::{
    HostBlobBeginParams, HostBlobFinishResult, HostBlobWriteParams,
};
use extension_protocol::method;

#[test]
fn blob_wire_bytes_round_trip_preserves_encoding_kind() {
    let values = [
        WireBytes::Utf8("hello".into()),
        WireBytes::Base64("AP8=".into()),
    ];

    for expected in values {
        let encoded = serde_json::to_value(&expected).unwrap();
        let actual: WireBytes = serde_json::from_value(encoded).unwrap();
        assert_eq!(expected, actual);
    }
}

#[test]
fn blob_and_event_stream_methods_are_public_protocol_constants() {
    assert_eq!("blob/read", method::BLOB_READ);
    assert_eq!("event/read", method::EVENT_READ);
    assert!(method::is_known(method::BLOB_CLOSE));
    assert!(method::is_known(method::EVENT_CLOSE));
}

#[test]
fn host_blob_methods_are_public_protocol_constants() {
    assert_eq!("host/blob/begin", method::HOST_BLOB_BEGIN);
    assert_eq!("host/blob/write", method::HOST_BLOB_WRITE);
    assert_eq!("host/blob/finish", method::HOST_BLOB_FINISH);
    assert_eq!("host/blob/abort", method::HOST_BLOB_ABORT);
    assert!(method::is_known(method::HOST_BLOB_BEGIN));
    assert!(method::is_known(method::HOST_BLOB_WRITE));
    assert!(method::is_known(method::HOST_BLOB_FINISH));
    assert!(method::is_known(method::HOST_BLOB_ABORT));
}

#[test]
fn host_blob_upload_contract_round_trips() {
    let begin = HostBlobBeginParams {
        content_type: Some("application/json".into()),
        metadata: Some(serde_json::json!({"source": "provider"})),
        expected_bytes: Some(42),
        ttl_ms: Some(30_000),
    };
    let begin_value = serde_json::to_value(&begin).unwrap();
    assert_eq!(
        begin,
        serde_json::from_value::<HostBlobBeginParams>(begin_value).unwrap()
    );

    let write = HostBlobWriteParams {
        upload_id: "upload-1".into(),
        sequence: 7,
        data: "AAE=".into(),
        bytes_written: 2,
    };
    let write_value = serde_json::to_value(&write).unwrap();
    assert_eq!(
        write,
        serde_json::from_value::<HostBlobWriteParams>(write_value).unwrap()
    );

    let finish = HostBlobFinishResult {
        blob_id: "host-blob-1".into(),
        total_bytes: 42,
        content_type: Some("application/json".into()),
    };
    let finish_value = serde_json::to_value(&finish).unwrap();
    assert_eq!(
        finish,
        serde_json::from_value::<HostBlobFinishResult>(finish_value).unwrap()
    );
}

#[test]
fn blob_and_event_results_have_bounded_read_fields() {
    let blob: BlobReadParams = serde_json::from_value(serde_json::json!({
        "blob_id": "b-1",
        "max_bytes": 4096
    }))
    .unwrap();
    assert_eq!(Some(4096), blob.max_bytes);
    assert_eq!(4096, blob.effective_max_bytes());
    assert_eq!(
        DEFAULT_BLOB_CHUNK_BYTES,
        BlobReadParams {
            blob_id: "b-2".into(),
            max_bytes: None
        }
        .effective_max_bytes()
    );
    assert_eq!(
        MAX_BLOB_CHUNK_BYTES,
        BlobReadParams {
            blob_id: "b-3".into(),
            max_bytes: Some(u32::MAX)
        }
        .effective_max_bytes()
    );
    assert!(!should_stream_blob(INLINE_BLOB_THRESHOLD_BYTES));
    assert!(should_stream_blob(INLINE_BLOB_THRESHOLD_BYTES + 1));

    let result = BlobReadResult {
        data: "AAE=".into(),
        bytes_read: 2,
        done: false,
    };
    assert_eq!(2, serde_json::to_value(result).unwrap()["bytes_read"]);

    let event: EventReadParams = serde_json::from_value(serde_json::json!({
        "stream_id": "events-1",
        "max_events": 10,
        "wait_ms": 5000
    }))
    .unwrap();
    assert_eq!(Some(10), event.max_events);
    assert_eq!(Some(5000), event.wait_ms);
    assert_eq!(10, event.effective_max_events());
    assert_eq!(
        DEFAULT_EVENT_MAX_EVENTS,
        EventReadParams {
            stream_id: "events-2".into(),
            max_events: None,
            wait_ms: None
        }
        .effective_max_events()
    );
    assert_eq!(
        MAX_EVENT_MAX_EVENTS,
        EventReadParams {
            stream_id: "events-3".into(),
            max_events: Some(u32::MAX),
            wait_ms: None
        }
        .effective_max_events()
    );

    let event_result: EventReadResult = serde_json::from_value(serde_json::json!({
        "events": [{"kind": "message"}],
        "closed": false,
        "dropped_count": 0
    }))
    .unwrap();
    assert_eq!(1, event_result.events.len());
}
