use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message};

use crate::http::{PreparedRequest, RequestMethod};
use crate::socket_io::{
    EnginePacket, SocketPacket, encode_event_input, parse_engine_packet, prepare_socket_io_request,
    socket_io_websocket_url,
};
use crate::websocket::{
    ConnectionCommand, ConnectionEvent, ConnectionEventEnvelope, build_client_request,
    start_connection,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

fn prepared_request(url: impl Into<String>) -> PreparedRequest {
    PreparedRequest {
        method: RequestMethod::Post,
        url: url.into(),
        headers: Vec::new(),
        body: b"ignored".to_vec(),
    }
}

async fn next_event(
    events: &mut tokio::sync::mpsc::Receiver<ConnectionEventEnvelope>,
) -> ConnectionEventEnvelope {
    timeout(TEST_TIMEOUT, events.recv())
        .await
        .expect("event arrived before timeout")
        .expect("connection event channel remains open")
}

async fn next_text(events: &mut tokio::sync::mpsc::Receiver<ConnectionEventEnvelope>) -> String {
    match next_event(events).await {
        ConnectionEventEnvelope::Message(ConnectionEvent::Text(text)) => text,
        event => panic!("expected a text packet, got {event:?}"),
    }
}

#[test]
fn socket_io_urls_are_normalized_for_direct_eio4_websocket_connections() {
    assert_eq!(
        socket_io_websocket_url("example.com").unwrap(),
        "ws://example.com/socket.io/?EIO=4&transport=websocket"
    );
    assert_eq!(
        socket_io_websocket_url("socketio://example.com/api?token=abc").unwrap(),
        "ws://example.com/api/socket.io/?token=abc&EIO=4&transport=websocket"
    );
    assert_eq!(
        socket_io_websocket_url("https://example.com/socket.io/?EIO=3&transport=polling&token=abc")
            .unwrap(),
        "wss://example.com/socket.io/?token=abc&EIO=4&transport=websocket"
    );
    assert_eq!(
        socket_io_websocket_url("socketios://example.com/socket.io").unwrap(),
        "wss://example.com/socket.io/?EIO=4&transport=websocket"
    );
}

#[test]
fn preparing_socket_io_forces_get_and_clears_the_http_body() {
    let prepared = prepare_socket_io_request(prepared_request("http://example.com")).unwrap();

    assert_eq!(prepared.method, RequestMethod::Get);
    assert!(prepared.body.is_empty());
    assert_eq!(
        prepared.url,
        "ws://example.com/socket.io/?EIO=4&transport=websocket"
    );
}

#[test]
fn engine_and_socket_packets_cover_open_connect_event_ack_and_error() {
    let open = parse_engine_packet(
        r#"0{"sid":"engine","upgrades":[],"pingInterval":25000,"pingTimeout":20000}"#,
    )
    .unwrap();
    assert!(matches!(
        open,
        EnginePacket::Open(packet) if packet.sid == "engine" && packet.ping_interval == 25_000
    ));
    assert!(matches!(
        parse_engine_packet(r#"40{"sid":"socket"}"#).unwrap(),
        EnginePacket::Message(SocketPacket::Connect { data: Some(_), .. })
    ));
    assert!(matches!(
        parse_engine_packet(r#"42/admin,12["chat",{"hello":"world"}]"#).unwrap(),
        EnginePacket::Message(SocketPacket::Event {
            namespace,
            id: Some(12),
            ..
        }) if namespace == "/admin"
    ));
    assert!(matches!(
        parse_engine_packet(r#"43/admin,12["ok"]"#).unwrap(),
        EnginePacket::Message(SocketPacket::Ack {
            namespace,
            id: Some(12),
            ..
        }) if namespace == "/admin"
    ));
    assert!(matches!(
        parse_engine_packet(r#"44{"message":"denied"}"#).unwrap(),
        EnginePacket::Message(SocketPacket::ConnectError { .. })
    ));
}

#[test]
fn engine_packet_parser_rejects_non_ascii_prefixes_without_panicking() {
    assert!(parse_engine_packet("你好").is_err());
    assert!(parse_engine_packet("😀").is_err());
}

#[test]
fn event_input_accepts_packets_arrays_and_plain_text() {
    assert_eq!(
        encode_event_input("42[\"chat\",\"hi\"]").unwrap(),
        "42[\"chat\",\"hi\"]"
    );
    assert_eq!(
        encode_event_input("[\"chat\", {\"hello\":\"world\"}]").unwrap(),
        "42[\"chat\",{\"hello\":\"world\"}]"
    );
    assert_eq!(
        encode_event_input("hello").unwrap(),
        "42[\"message\",\"hello\"]"
    );
}

#[tokio::test]
async fn direct_websocket_transport_completes_socket_io_handshake_and_heartbeat() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        socket
            .send(Message::text(
                r#"0{"sid":"engine-test","upgrades":[],"pingInterval":25000,"pingTimeout":20000}"#,
            ))
            .await
            .unwrap();
        assert_eq!(socket.next().await.unwrap().unwrap(), Message::text("40"));
        socket
            .send(Message::text(r#"40{"sid":"socket-test"}"#))
            .await
            .unwrap();
        socket.send(Message::text("2heartbeat")).await.unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::text("3heartbeat")
        );
        socket
            .send(Message::text(r#"42["welcome",{"ready":true}]"#))
            .await
            .unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::text(r#"42["message","hello"]"#)
        );
        let _ = socket.next().await;
    });

    let prepared = prepare_socket_io_request(prepared_request(format!("ws://{address}"))).unwrap();
    let request = build_client_request(&prepared).unwrap();
    let mut connection = start_connection(&tokio::runtime::Handle::current(), request);

    assert_eq!(
        next_event(&mut connection.events).await,
        ConnectionEventEnvelope::Connecting
    );
    assert!(matches!(
        next_event(&mut connection.events).await,
        ConnectionEventEnvelope::Connected { status: 101, .. }
    ));

    let open = next_text(&mut connection.events).await;
    assert!(matches!(
        parse_engine_packet(&open).unwrap(),
        EnginePacket::Open(packet) if packet.sid == "engine-test"
    ));
    connection
        .commands
        .send(ConnectionCommand::Text("40".into()))
        .await
        .unwrap();

    assert!(matches!(
        parse_engine_packet(&next_text(&mut connection.events).await).unwrap(),
        EnginePacket::Message(SocketPacket::Connect { .. })
    ));
    let ping = next_text(&mut connection.events).await;
    let EnginePacket::Ping(payload) = parse_engine_packet(&ping).unwrap() else {
        panic!("expected Engine.IO ping");
    };
    connection
        .commands
        .send(ConnectionCommand::Text(format!("3{payload}")))
        .await
        .unwrap();

    assert!(matches!(
        parse_engine_packet(&next_text(&mut connection.events).await).unwrap(),
        EnginePacket::Message(SocketPacket::Event { .. })
    ));
    connection
        .commands
        .send(ConnectionCommand::Text(
            encode_event_input("hello").unwrap(),
        ))
        .await
        .unwrap();
    connection
        .commands
        .send(ConnectionCommand::Close)
        .await
        .unwrap();
    timeout(TEST_TIMEOUT, server).await.unwrap().unwrap();
}
