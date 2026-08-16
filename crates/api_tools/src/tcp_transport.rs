use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

pub const COMMAND_CHANNEL_CAPACITY: usize = 64;
pub const EVENT_CHANNEL_CAPACITY: usize = 256;
const READ_BUFFER_BYTES: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionCommand {
    Send(Vec<u8>),
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    Connecting,
    Connected { peer: String },
    Data(Vec<u8>),
    Closed,
    Error(String),
}

pub struct ConnectionTask {
    pub commands: mpsc::Sender<ConnectionCommand>,
    pub events: mpsc::Receiver<ConnectionEvent>,
    pub cancel: oneshot::Sender<()>,
}

pub fn start_connection(handle: &tokio::runtime::Handle, target: String) -> ConnectionTask {
    let (command_tx, command_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let (cancel_tx, cancel_rx) = oneshot::channel();
    handle.spawn(run_connection(target, command_rx, cancel_rx, event_tx));
    ConnectionTask {
        commands: command_tx,
        events: event_rx,
        cancel: cancel_tx,
    }
}

async fn run_connection(
    target: String,
    mut commands: mpsc::Receiver<ConnectionCommand>,
    mut cancel: oneshot::Receiver<()>,
    events: mpsc::Sender<ConnectionEvent>,
) {
    if !send_event(&events, ConnectionEvent::Connecting).await {
        return;
    }
    let Some(stream) = connect(&target, &mut commands, &mut cancel, &events).await else {
        return;
    };
    let peer = stream
        .peer_addr()
        .map(|address| address.to_string())
        .unwrap_or(target);
    if !send_event(&events, ConnectionEvent::Connected { peer }).await {
        return;
    }
    run_stream(stream, commands, cancel, events).await;
}

async fn connect(
    target: &str,
    commands: &mut mpsc::Receiver<ConnectionCommand>,
    cancel: &mut oneshot::Receiver<()>,
    events: &mpsc::Sender<ConnectionEvent>,
) -> Option<TcpStream> {
    let result = tokio::select! {
        result = TcpStream::connect(target) => result,
        _ = wait_for_close(commands) => return None,
        _ = cancel => return None,
    };
    match result {
        Ok(stream) => Some(stream),
        Err(error) => {
            let _ = send_event(events, ConnectionEvent::Error(error.to_string())).await;
            None
        }
    }
}

async fn run_stream(
    mut stream: TcpStream,
    mut commands: mpsc::Receiver<ConnectionCommand>,
    mut cancel: oneshot::Receiver<()>,
    events: mpsc::Sender<ConnectionEvent>,
) {
    let mut buffer = vec![0; READ_BUFFER_BYTES];
    loop {
        tokio::select! {
            _ = &mut cancel => {
                let _ = stream.shutdown().await;
                break;
            }
            command = commands.recv() => {
                if !handle_command(command, &mut stream, &events).await {
                    break;
                }
            }
            result = stream.read(&mut buffer) => {
                if !handle_read(result, &buffer, &events).await {
                    break;
                }
            }
        }
    }
}

async fn wait_for_close(commands: &mut mpsc::Receiver<ConnectionCommand>) {
    while let Some(command) = commands.recv().await {
        if matches!(command, ConnectionCommand::Close) {
            break;
        }
    }
}

async fn handle_command(
    command: Option<ConnectionCommand>,
    stream: &mut TcpStream,
    events: &mpsc::Sender<ConnectionEvent>,
) -> bool {
    match command {
        Some(ConnectionCommand::Send(bytes)) => {
            if let Err(error) = stream.write_all(&bytes).await {
                let _ = send_event(events, ConnectionEvent::Error(error.to_string())).await;
                return false;
            }
            true
        }
        Some(ConnectionCommand::Close) => {
            let _ = stream.shutdown().await;
            let _ = send_event(events, ConnectionEvent::Closed).await;
            false
        }
        None => {
            let _ = stream.shutdown().await;
            false
        }
    }
}

async fn handle_read(
    result: std::io::Result<usize>,
    buffer: &[u8],
    events: &mpsc::Sender<ConnectionEvent>,
) -> bool {
    match result {
        Ok(0) => {
            let _ = send_event(events, ConnectionEvent::Closed).await;
            false
        }
        Ok(count) => send_event(events, ConnectionEvent::Data(buffer[..count].to_vec())).await,
        Err(error) => {
            let _ = send_event(events, ConnectionEvent::Error(error.to_string())).await;
            false
        }
    }
}

async fn send_event(events: &mpsc::Sender<ConnectionEvent>, event: ConnectionEvent) -> bool {
    events.send(event).await.is_ok()
}
