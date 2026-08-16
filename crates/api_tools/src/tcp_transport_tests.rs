use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::tcp::{ConnectionCommand, ConnectionEvent, start_connection};

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

async fn next_event(events: &mut tokio::sync::mpsc::Receiver<ConnectionEvent>) -> ConnectionEvent {
    timeout(TEST_TIMEOUT, events.recv())
        .await
        .expect("event arrived before timeout")
        .expect("connection event channel remains open")
}

#[tokio::test]
async fn transport_connects_and_preserves_binary_data_in_both_directions() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.write_all(&[0, 255, 16, 128]).await.unwrap();
        let mut received = [0; 4];
        stream.read_exact(&mut received).await.unwrap();
        received
    });

    let mut connection = start_connection(&tokio::runtime::Handle::current(), address.to_string());

    assert_eq!(
        next_event(&mut connection.events).await,
        ConnectionEvent::Connecting
    );
    assert!(matches!(
        next_event(&mut connection.events).await,
        ConnectionEvent::Connected { .. }
    ));
    assert_eq!(
        next_event(&mut connection.events).await,
        ConnectionEvent::Data(vec![0, 255, 16, 128])
    );
    connection
        .commands
        .send(ConnectionCommand::Send(vec![1, 2, 3, 4]))
        .await
        .unwrap();
    assert_eq!(
        timeout(TEST_TIMEOUT, server).await.unwrap().unwrap(),
        [1, 2, 3, 4]
    );
}

#[tokio::test]
async fn peer_eof_emits_closed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        drop(stream);
    });

    let mut connection = start_connection(&tokio::runtime::Handle::current(), address.to_string());

    assert_eq!(
        next_event(&mut connection.events).await,
        ConnectionEvent::Connecting
    );
    assert!(matches!(
        next_event(&mut connection.events).await,
        ConnectionEvent::Connected { .. }
    ));
    assert_eq!(
        next_event(&mut connection.events).await,
        ConnectionEvent::Closed
    );
    timeout(TEST_TIMEOUT, server).await.unwrap().unwrap();
}

#[tokio::test]
async fn close_command_stops_the_transport_and_closes_the_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut byte = [0; 1];
        stream.read(&mut byte).await.unwrap()
    });

    let mut connection = start_connection(&tokio::runtime::Handle::current(), address.to_string());
    assert_eq!(
        next_event(&mut connection.events).await,
        ConnectionEvent::Connecting
    );
    assert!(matches!(
        next_event(&mut connection.events).await,
        ConnectionEvent::Connected { .. }
    ));
    connection
        .commands
        .send(ConnectionCommand::Close)
        .await
        .unwrap();

    assert_eq!(
        next_event(&mut connection.events).await,
        ConnectionEvent::Closed
    );
    assert_eq!(timeout(TEST_TIMEOUT, server).await.unwrap().unwrap(), 0);
}

#[tokio::test]
async fn cancellation_stops_a_connected_transport() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut byte = [0; 1];
        stream.read(&mut byte).await.unwrap()
    });

    let mut connection = start_connection(&tokio::runtime::Handle::current(), address.to_string());
    assert_eq!(
        next_event(&mut connection.events).await,
        ConnectionEvent::Connecting
    );
    assert!(matches!(
        next_event(&mut connection.events).await,
        ConnectionEvent::Connected { .. }
    ));
    connection.cancel.send(()).unwrap();

    assert!(
        timeout(TEST_TIMEOUT, connection.events.recv())
            .await
            .expect("transport exits before timeout")
            .is_none()
    );
    assert_eq!(timeout(TEST_TIMEOUT, server).await.unwrap().unwrap(), 0);
}
