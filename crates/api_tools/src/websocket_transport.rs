use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, handshake::client::Request},
};

use crate::websocket::{ConnectionEvent, event_from_message};

pub const COMMAND_CHANNEL_CAPACITY: usize = 64;
pub const EVENT_CHANNEL_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionCommand {
    Text(String),
    #[allow(dead_code)]
    Binary(Vec<u8>),
    #[allow(dead_code)]
    Ping(Vec<u8>),
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEventEnvelope {
    Connecting,
    Connected {
        status: u16,
        headers: Vec<(String, String)>,
    },
    Message(ConnectionEvent),
    Error(String),
}

pub struct ConnectionTask {
    pub commands: mpsc::Sender<ConnectionCommand>,
    pub events: mpsc::Receiver<ConnectionEventEnvelope>,
    pub cancel: oneshot::Sender<()>,
}

pub fn start_connection(handle: &tokio::runtime::Handle, request: Request) -> ConnectionTask {
    let (command_tx, command_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let (cancel_tx, cancel_rx) = oneshot::channel();
    handle.spawn(run_connection(request, command_rx, cancel_rx, event_tx));
    ConnectionTask {
        commands: command_tx,
        events: event_rx,
        cancel: cancel_tx,
    }
}

async fn run_connection(
    request: Request,
    mut commands: mpsc::Receiver<ConnectionCommand>,
    mut cancel: oneshot::Receiver<()>,
    events: mpsc::Sender<ConnectionEventEnvelope>,
) {
    if !send_event(&events, ConnectionEventEnvelope::Connecting).await {
        return;
    }

    let Some((socket, response)) =
        connect_socket(request, &mut commands, &mut cancel, &events).await
    else {
        return;
    };
    let headers = response_headers(&response);
    if !send_event(
        &events,
        ConnectionEventEnvelope::Connected {
            status: response.status().as_u16(),
            headers,
        },
    )
    .await
    {
        return;
    }
    run_socket(socket, commands, cancel, events).await;
}

async fn connect_socket(
    request: Request,
    commands: &mut mpsc::Receiver<ConnectionCommand>,
    cancel: &mut oneshot::Receiver<()>,
    events: &mpsc::Sender<ConnectionEventEnvelope>,
) -> Option<(
    WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::handshake::client::Response,
)> {
    let result = tokio::select! {
        result = connect_async(request) => result,
        _ = wait_for_close(commands) => return None,
        _ = cancel => return None,
    };
    match result {
        Ok(connection) => Some(connection),
        Err(error) => {
            let _ = send_event(&events, ConnectionEventEnvelope::Error(error.to_string())).await;
            None
        }
    }
}

fn response_headers(
    response: &tokio_tungstenite::tungstenite::handshake::client::Response,
) -> Vec<(String, String)> {
    response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                value.to_str().unwrap_or("<binary>").to_string(),
            )
        })
        .collect()
}

async fn run_socket(
    socket: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    mut commands: mpsc::Receiver<ConnectionCommand>,
    mut cancel: oneshot::Receiver<()>,
    events: mpsc::Sender<ConnectionEventEnvelope>,
) {
    let (mut writer, mut reader) = socket.split();
    loop {
        tokio::select! {
            _ = &mut cancel => {
                let _ = writer.send(Message::Close(None)).await;
                break;
            }
            command = commands.recv() => {
                if !handle_command(command, &mut writer, &events).await {
                    break;
                }
            }
            message = reader.next() => {
                if !handle_message(message, &events).await {
                    break;
                }
            }
        }
    }
}

async fn wait_for_close(commands: &mut mpsc::Receiver<ConnectionCommand>) -> bool {
    while let Some(command) = commands.recv().await {
        if matches!(command, ConnectionCommand::Close) {
            return true;
        }
    }
    false
}

async fn handle_command<S>(
    command: Option<ConnectionCommand>,
    writer: &mut S,
    events: &mpsc::Sender<ConnectionEventEnvelope>,
) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let Some(command) = command else {
        return false;
    };
    let message = match command {
        ConnectionCommand::Text(text) => Message::text(text),
        ConnectionCommand::Binary(bytes) => Message::binary(bytes),
        ConnectionCommand::Ping(bytes) => Message::Ping(bytes.into()),
        ConnectionCommand::Close => {
            let _ = writer.send(Message::Close(None)).await;
            return false;
        }
    };
    if let Err(error) = writer.send(message).await {
        let _ = send_event(events, ConnectionEventEnvelope::Error(error.to_string())).await;
        return false;
    }
    true
}

async fn handle_message(
    message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    events: &mpsc::Sender<ConnectionEventEnvelope>,
) -> bool {
    let Some(message) = message else {
        let _ = send_event(
            events,
            ConnectionEventEnvelope::Message(ConnectionEvent::Closed {
                code: None,
                reason: "peer disconnected".into(),
            }),
        )
        .await;
        return false;
    };
    let message = match message {
        Ok(message) => message,
        Err(error) => {
            let _ = send_event(events, ConnectionEventEnvelope::Error(error.to_string())).await;
            return false;
        }
    };
    let is_closed = message.is_close();
    let event = ConnectionEventEnvelope::Message(event_from_message(message));
    if !send_event(events, event).await {
        return false;
    }
    !is_closed
}

async fn send_event(
    events: &mpsc::Sender<ConnectionEventEnvelope>,
    event: ConnectionEventEnvelope,
) -> bool {
    events.send(event).await.is_ok()
}
