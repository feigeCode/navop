use super::{
    DetectedZmodem, ZmodemDirection, ZmodemPickerResponse, ZmodemResponder, checked_file_size,
    run_transfer,
};
use anyhow::Result;
use async_trait::async_trait;
use ssh::{ChannelEvent, ForwardRequest, PtyConfig, SshChannel};
use std::{
    collections::VecDeque,
    future::pending,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::unbounded_channel;
use tokio_util::sync::CancellationToken;
use zmodem2::{Action, Event, Receiver};

struct ReceiverPeerChannel {
    receiver: Receiver,
    pending: Vec<u8>,
    outgoing: VecDeque<Vec<u8>>,
    received: Arc<Mutex<Vec<u8>>>,
    session_complete: bool,
    trailing: Vec<u8>,
}

impl ReceiverPeerChannel {
    fn new(trailing: Vec<u8>) -> (Self, Vec<u8>, Arc<Mutex<Vec<u8>>>) {
        let mut receiver = Receiver::new().expect("receiver");
        let initial = match receiver.poll() {
            Action::WriteWire(bytes) => bytes.to_vec(),
            action => panic!("expected receiver handshake, got {action:?}"),
        };
        receiver.wire_written(initial.len());
        let received = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                receiver,
                pending: Vec::new(),
                outgoing: VecDeque::new(),
                received: received.clone(),
                session_complete: false,
                trailing,
            },
            initial,
            received,
        )
    }

    fn drive(&mut self) -> Result<()> {
        let mut response = Vec::new();
        loop {
            match self.receiver.poll() {
                Action::WriteWire(bytes) => {
                    let len = bytes.len();
                    response.extend_from_slice(bytes);
                    self.receiver.wire_written(len);
                }
                Action::WriteFile(bytes) => {
                    let len = bytes.len();
                    self.received.lock().unwrap().extend_from_slice(bytes);
                    self.receiver.file_written(len)?;
                }
                Action::Event(Event::SessionCompleted) => {
                    self.session_complete = true;
                }
                Action::Event(Event::Aborted) => panic!("receiver aborted"),
                Action::Event(_) => {}
                Action::Idle => {
                    if !self.submit_pending()? {
                        break;
                    }
                }
                Action::ReadFile { .. } => panic!("receiver requested file data"),
                _ => {}
            }
        }
        if self.session_complete {
            response.extend(std::mem::take(&mut self.trailing));
        }
        if !response.is_empty() {
            self.outgoing.push_back(response);
        }
        Ok(())
    }

    fn submit_pending(&mut self) -> Result<bool> {
        if self.pending.is_empty() {
            return Ok(false);
        }
        let consumed = self.receiver.submit_wire(&self.pending)?;
        if consumed == 0 {
            return Ok(false);
        }
        self.pending.drain(..consumed);
        Ok(true)
    }
}

#[async_trait]
impl SshChannel for ReceiverPeerChannel {
    async fn request_pty(&mut self, _config: &PtyConfig) -> Result<()> {
        Ok(())
    }

    async fn exec(&mut self, _command: &str) -> Result<()> {
        Ok(())
    }

    async fn request_shell(&mut self) -> Result<()> {
        Ok(())
    }

    async fn request_x11_forwarding(&mut self, _request: &ForwardRequest) -> Result<()> {
        Ok(())
    }

    async fn set_env(&mut self, _name: &str, _value: &str) -> Result<()> {
        Ok(())
    }

    async fn send_data(&mut self, data: &[u8]) -> Result<()> {
        self.pending.extend_from_slice(data);
        self.drive()
    }

    async fn resize_pty(&mut self, _width: u32, _height: u32) -> Result<()> {
        Ok(())
    }

    async fn recv(&mut self) -> Option<ChannelEvent> {
        if self.drive().is_err() {
            return None;
        }
        if let Some(data) = self.outgoing.pop_front() {
            return Some(ChannelEvent::Data(data));
        }
        pending().await
    }

    async fn eof(&mut self) -> Result<()> {
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn upload_rejects_files_larger_than_zmodem_position() {
    let error = checked_file_size(u64::from(u32::MAX) + 1).expect_err("oversized file");
    assert!(error.to_string().contains("4 GiB"));
}

#[tokio::test]
async fn upload_round_trip_preserves_trailing_terminal_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("escaped.bin");
    let payload = escaped_payload();
    tokio::fs::write(&path, &payload).await.unwrap();
    let trailing = b"\r\nremote-shell$ ".to_vec();
    let (mut channel, initial_wire, received) = ReceiverPeerChannel::new(trailing.clone());
    let (event_tx, mut event_rx) = unbounded_channel();
    let responder = ZmodemResponder::new(event_tx);
    let task_responder = responder.clone();
    let response_task = tokio::spawn(async move {
        event_rx.recv().await.expect("picker request event");
        assert!(task_responder.submit(ZmodemPickerResponse::UploadFiles(vec![path])));
    });

    let result = run_transfer(
        &mut channel,
        DetectedZmodem {
            direction: ZmodemDirection::Upload,
            wire: initial_wire,
        },
        &responder,
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    response_task.await.unwrap();
    assert_eq!(received.lock().unwrap().as_slice(), payload);
    assert_eq!(result, trailing);
}

fn escaped_payload() -> Vec<u8> {
    const ESCAPED: [u8; 12] = [
        0x00, 0x0d, 0x10, 0x11, 0x13, 0x18, 0x7f, 0x8d, 0x90, 0x91, 0x93, 0xff,
    ];
    (0..512 * 1024)
        .map(|index| ESCAPED[index % ESCAPED.len()])
        .collect()
}
