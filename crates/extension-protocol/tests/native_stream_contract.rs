use extension_protocol::blob::{
    BlobReadParams, BlobReadResult, DEFAULT_BLOB_CHUNK_BYTES, INLINE_BLOB_THRESHOLD_BYTES,
    MAX_BLOB_CHUNK_BYTES, WireBytes, should_stream_blob,
};
use extension_protocol::event_stream::{
    DEFAULT_EVENT_MAX_EVENTS, EventReadParams, EventReadResult, MAX_EVENT_MAX_EVENTS,
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
