use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message};

use crate::http::{PreparedRequest, RequestMethod};
use crate::websocket::{
    ConnectionCommand, ConnectionEvent, ConnectionEventEnvelope, build_client_request,
    start_connection,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

fn prepared_request(url: String) -> PreparedRequest {
    PreparedRequest {
        method: RequestMethod::Get,
        url,
        headers: Vec::new(),
        body: Vec::new(),
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

#[tokio::test]
async fn transport_connects_and_exchanges_text_messages() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::text("hello")
        );
        socket.send(Message::text("echo")).await.unwrap();
        let _ = socket.next().await;
    });
    let request = build_client_request(&prepared_request(format!("ws://{address}"))).unwrap();

    let mut connection = start_connection(&tokio::runtime::Handle::current(), request);

    assert_eq!(
        next_event(&mut connection.events).await,
        ConnectionEventEnvelope::Connecting
    );
    assert!(matches!(
        next_event(&mut connection.events).await,
        ConnectionEventEnvelope::Connected { status: 101, .. }
    ));
    connection
        .commands
        .send(ConnectionCommand::Text("hello".into()))
        .await
        .unwrap();
    assert_eq!(
        next_event(&mut connection.events).await,
        ConnectionEventEnvelope::Message(ConnectionEvent::Text("echo".into()))
    );
    connection
        .commands
        .send(ConnectionCommand::Close)
        .await
        .unwrap();
    timeout(TEST_TIMEOUT, server).await.unwrap().unwrap();
}

#[tokio::test]
async fn close_command_cancels_an_incomplete_handshake() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stalled_server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(TEST_TIMEOUT).await;
    });
    let request = build_client_request(&prepared_request(format!("ws://{address}"))).unwrap();

    let mut connection = start_connection(&tokio::runtime::Handle::current(), request);

    assert_eq!(
        next_event(&mut connection.events).await,
        ConnectionEventEnvelope::Connecting
    );
    connection
        .commands
        .send(ConnectionCommand::Close)
        .await
        .unwrap();
    assert!(
        timeout(TEST_TIMEOUT, connection.events.recv())
            .await
            .expect("transport exits before timeout")
            .is_none()
    );
    stalled_server.abort();
}

#[tokio::test]
async fn cancellation_signal_stops_a_connected_transport() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        assert!(matches!(
            timeout(TEST_TIMEOUT, socket.next()).await.unwrap(),
            Some(Ok(Message::Close(_)))
        ));
    });
    let request = build_client_request(&prepared_request(format!("ws://{address}"))).unwrap();

    let mut connection = start_connection(&tokio::runtime::Handle::current(), request);

    assert_eq!(
        next_event(&mut connection.events).await,
        ConnectionEventEnvelope::Connecting
    );
    assert!(matches!(
        next_event(&mut connection.events).await,
        ConnectionEventEnvelope::Connected { status: 101, .. }
    ));
    connection.cancel.send(()).unwrap();
    assert!(
        timeout(TEST_TIMEOUT, connection.events.recv())
            .await
            .expect("transport exits before timeout")
            .is_none()
    );
    timeout(TEST_TIMEOUT, server).await.unwrap().unwrap();
}
