use tokio_tungstenite::tungstenite::Message;

use crate::http::{PreparedRequest, RequestMethod};
use crate::websocket::{
    ConnectionEvent, MessageDirection, Timeline, TimelineEntry, build_client_request,
    event_from_message,
};

const TEST_TIMELINE_LIMIT: usize = 2;

fn prepared_request(headers: Vec<(String, String)>) -> PreparedRequest {
    PreparedRequest {
        method: RequestMethod::Get,
        url: "ws://example.test/socket".into(),
        headers,
        body: Vec::new(),
    }
}

#[test]
fn client_request_preserves_generated_handshake_headers_and_custom_headers() {
    let prepared = prepared_request(vec![("X-Trace".into(), "trace-1".into())]);

    let request = build_client_request(&prepared).expect("valid WebSocket request");

    assert_eq!(request.headers()["x-trace"], "trace-1");
    assert_eq!(request.headers()["upgrade"], "websocket");
    assert_eq!(request.headers()["connection"], "Upgrade");
    assert!(request.headers().contains_key("sec-websocket-key"));
    assert_eq!(request.headers()["sec-websocket-version"], "13");
}

#[test]
fn client_request_does_not_allow_custom_headers_to_break_the_handshake() {
    let prepared = prepared_request(vec![
        ("Upgrade".into(), "not-websocket".into()),
        ("Sec-WebSocket-Version".into(), "12".into()),
    ]);

    let request = build_client_request(&prepared).expect("valid WebSocket request");

    assert_eq!(request.headers()["upgrade"], "websocket");
    assert_eq!(request.headers()["sec-websocket-version"], "13");
}

#[test]
fn client_request_rejects_invalid_header_names() {
    let prepared = prepared_request(vec![("bad header".into(), "value".into())]);

    let error = build_client_request(&prepared).expect_err("invalid header must fail");

    assert!(error.to_string().contains("bad header"));
}

#[test]
fn tungstenite_messages_preserve_text_binary_and_control_frames() {
    assert_eq!(
        event_from_message(Message::text("hello")),
        ConnectionEvent::Text("hello".into())
    );
    assert_eq!(
        event_from_message(Message::binary(vec![0_u8, 1, 255])),
        ConnectionEvent::Binary(vec![0, 1, 255])
    );
    assert_eq!(
        event_from_message(Message::Ping(vec![1, 2].into())),
        ConnectionEvent::Ping(vec![1, 2])
    );
    assert_eq!(
        event_from_message(Message::Pong(vec![3, 4].into())),
        ConnectionEvent::Pong(vec![3, 4])
    );
}

#[test]
fn close_frames_keep_code_and_reason() {
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

    let message = Message::Close(Some(CloseFrame {
        code: CloseCode::Normal,
        reason: "done".into(),
    }));

    assert_eq!(
        event_from_message(message),
        ConnectionEvent::Closed {
            code: Some(1000),
            reason: "done".into(),
        }
    );
}

#[test]
fn timeline_keeps_only_the_newest_entries() {
    let mut timeline = Timeline::new(TEST_TIMELINE_LIMIT);
    timeline.push(TimelineEntry::system("connecting"));
    timeline.push(TimelineEntry::text(MessageDirection::Received, "one"));
    timeline.push(TimelineEntry::text(MessageDirection::Sent, "two"));

    assert_eq!(timeline.entries().len(), TEST_TIMELINE_LIMIT);
    assert_eq!(timeline.entries()[0].display_text(), "one");
    assert_eq!(timeline.entries()[1].display_text(), "two");
}

#[test]
fn binary_timeline_entries_keep_bytes_and_show_a_hex_preview() {
    let entry = TimelineEntry::binary(MessageDirection::Received, vec![0, 1, 255]);

    assert_eq!(entry.binary_bytes(), Some(&[0, 1, 255][..]));
    assert_eq!(entry.display_text(), "3 bytes · 00 01 ff");
}
