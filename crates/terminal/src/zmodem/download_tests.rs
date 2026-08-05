use super::{DetectedZmodem, ZmodemDirection, ZmodemPickerResponse, ZmodemResponder, run_transfer};
use anyhow::Result;
use async_trait::async_trait;
use ssh::{ChannelEvent, ForwardRequest, PtyConfig, SshChannel};
use std::{collections::VecDeque, future::pending};
use tokio::sync::mpsc::unbounded_channel;
use tokio_util::sync::CancellationToken;
use zmodem2::{Action, Event, FileInfo, Position, Sender};

struct SenderPeerChannel {
    sender: Sender,
    payload: Vec<u8>,
    pending: Vec<u8>,
    outgoing: VecDeque<Vec<u8>>,
    session_complete: bool,
    trailing: Vec<u8>,
}

impl SenderPeerChannel {
    fn new(payload: Vec<u8>, trailing: Vec<u8>) -> (Self, Vec<u8>) {
        let mut sender = Sender::new().expect("sender");
        sender
            .start_file(FileInfo::new(
                b"remote.bin",
                Some(Position::new(payload.len() as u32)),
            ))
            .expect("start file");
        let initial = take_sender_wire(&mut sender);
        (
            Self {
                sender,
                payload,
                pending: Vec::new(),
                outgoing: VecDeque::new(),
                session_complete: false,
                trailing,
            },
            initial,
        )
    }

    fn drive(&mut self) -> Result<()> {
        let mut response = Vec::new();
        loop {
            let mut idle = false;
            match self.sender.poll() {
                Action::WriteWire(bytes) => {
                    let len = bytes.len();
                    response.extend_from_slice(bytes);
                    self.sender.wire_written(len);
                }
                Action::ReadFile { offset, max_len } => {
                    let start = offset.get() as usize;
                    let end = (start + max_len).min(self.payload.len());
                    self.sender.submit_file(&self.payload[start..end])?;
                }
                Action::Event(Event::FileCompleted) => self.sender.finish()?,
                Action::Event(Event::SessionCompleted) => self.session_complete = true,
                Action::Event(Event::Aborted) => panic!("sender aborted"),
                Action::Event(_) => {}
                Action::Idle => idle = true,
                Action::WriteFile(_) => panic!("sender wrote file data"),
                _ => {}
            }
            if idle && !self.submit_pending()? {
                break;
            }
        }
        if self.session_complete && response.ends_with(b"OO") {
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
        let consumed = self.sender.submit_wire(&self.pending)?;
        if consumed == 0 {
            return Ok(false);
        }
        self.pending.drain(..consumed);
        Ok(true)
    }
}

fn take_sender_wire(sender: &mut Sender) -> Vec<u8> {
    let Action::WriteWire(bytes) = sender.poll() else {
        panic!("expected sender handshake");
    };
    let bytes = bytes.to_vec();
    sender.wire_written(bytes.len());
    bytes
}

#[async_trait]
impl SshChannel for SenderPeerChannel {
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

#[tokio::test]
async fn download_round_trip_consumes_oo_and_preserves_terminal_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let payload: Vec<u8> = (0..8192).map(|index| (index % 251) as u8).collect();
    let trailing = b"\r\nremote-shell$ ".to_vec();
    let (mut channel, initial_wire) = SenderPeerChannel::new(payload.clone(), trailing.clone());
    let (event_tx, mut event_rx) = unbounded_channel();
    let responder = ZmodemResponder::new(event_tx);
    let task_responder = responder.clone();
    let download_directory = directory.path().to_path_buf();
    let response_task = tokio::spawn(async move {
        event_rx.recv().await.expect("picker request event");
        assert!(task_responder.submit(ZmodemPickerResponse::DownloadDirectory(download_directory)));
    });

    let result = run_transfer(
        &mut channel,
        DetectedZmodem {
            direction: ZmodemDirection::Download,
            wire: initial_wire,
        },
        &responder,
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    response_task.await.unwrap();
    let downloaded = tokio::fs::read(directory.path().join("remote.bin"))
        .await
        .unwrap();
    assert_eq!(downloaded, payload);
    assert_eq!(result, trailing);
}
