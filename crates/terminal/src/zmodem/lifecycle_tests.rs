use super::{
    DetectedZmodem, ZCAN, ZmodemDirection, ZmodemPickerResponse, ZmodemResponder,
    is_channel_closed, run_transfer,
};
use anyhow::Result;
use async_trait::async_trait;
use ssh::{ChannelEvent, ForwardRequest, PtyConfig, SshChannel};
use std::{
    future::pending,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::mpsc::unbounded_channel,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct MockChannel {
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
    disconnect_delay: Option<Duration>,
}

#[async_trait]
impl SshChannel for MockChannel {
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
        self.sent.lock().unwrap().push(data.to_vec());
        Ok(())
    }

    async fn resize_pty(&mut self, _width: u32, _height: u32) -> Result<()> {
        Ok(())
    }

    async fn recv(&mut self) -> Option<ChannelEvent> {
        if let Some(delay) = self.disconnect_delay {
            sleep(delay).await;
            return Some(ChannelEvent::Eof);
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
async fn cancelled_picker_sends_zcan() {
    let (event_tx, mut event_rx) = unbounded_channel();
    let responder = ZmodemResponder::new(event_tx);
    let task_responder = responder.clone();
    let response_task = tokio::spawn(async move {
        event_rx.recv().await.expect("picker request event");
        assert!(task_responder.submit(ZmodemPickerResponse::Cancel));
    });
    let mut channel = MockChannel::default();
    let sent = channel.sent.clone();

    let result = run_transfer(
        &mut channel,
        DetectedZmodem {
            direction: ZmodemDirection::Upload,
            wire: Vec::new(),
        },
        &responder,
        &CancellationToken::new(),
    )
    .await;

    assert!(result.is_err());
    response_task.await.unwrap();
    assert_eq!(sent.lock().unwrap().as_slice(), &[ZCAN.to_vec()]);
}

#[tokio::test]
async fn remote_disconnect_while_picker_is_pending_does_not_hang() {
    let (event_tx, mut event_rx) = unbounded_channel();
    let responder = ZmodemResponder::new(event_tx);
    let mut channel = MockChannel {
        disconnect_delay: Some(Duration::from_millis(10)),
        ..Default::default()
    };
    let sent = channel.sent.clone();

    let result = timeout(
        Duration::from_millis(250),
        run_transfer(
            &mut channel,
            DetectedZmodem {
                direction: ZmodemDirection::Upload,
                wire: Vec::new(),
            },
            &responder,
            &CancellationToken::new(),
        ),
    )
    .await
    .expect("remote disconnect must interrupt the picker wait");

    let error = result.expect_err("disconnect should fail the transfer");
    assert!(is_channel_closed(&error));
    assert!(event_rx.recv().await.is_some());
    assert!(responder.pending_request().is_none());
    assert_eq!(sent.lock().unwrap().as_slice(), &[ZCAN.to_vec()]);
}
