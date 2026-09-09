//! Bounded background pull loop for provider-owned event streams.

use std::time::Duration;

use extension_host::{CancellationToken, HostError, RequestOptions};
use extension_protocol::event_stream::{EventCloseParams, EventReadParams, EventReadResult};
use serde_json::Value;
use tokio::{sync::mpsc, task::JoinHandle, time::sleep};

use crate::ManagedUniversalPluginClient;

pub const DEFAULT_EVENT_BRIDGE_CAPACITY: usize = 8;
const DEFAULT_EVENT_BATCH_SIZE: u32 = 128;
const DEFAULT_EVENT_WAIT_MS: u32 = 1_000;
const EMPTY_READ_DELAY: Duration = Duration::from_millis(10);
const EVENT_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq)]
pub struct EventStreamBatch {
    pub events: Vec<Value>,
    pub dropped_count: u64,
    pub closed: bool,
}

#[derive(Debug, Clone)]
pub struct EventStreamSubscriptionConfig {
    pub channel_capacity: usize,
    pub max_events: u32,
    pub wait_ms: u32,
}

impl Default for EventStreamSubscriptionConfig {
    fn default() -> Self {
        Self {
            channel_capacity: DEFAULT_EVENT_BRIDGE_CAPACITY,
            max_events: DEFAULT_EVENT_BATCH_SIZE,
            wait_ms: DEFAULT_EVENT_WAIT_MS,
        }
    }
}

pub struct EventStreamSubscription {
    stream_id: String,
    receiver: mpsc::Receiver<Result<EventStreamBatch, HostError>>,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
}

#[async_trait::async_trait]
pub(crate) trait EventStreamClient: Clone + Send + Sync + 'static {
    async fn read(
        &self,
        params: &EventReadParams,
        options: RequestOptions,
    ) -> Result<EventReadResult, HostError>;

    async fn close(&self, params: &EventCloseParams) -> Result<(), HostError>;
}

#[async_trait::async_trait]
impl EventStreamClient for ManagedUniversalPluginClient {
    async fn read(
        &self,
        params: &EventReadParams,
        options: RequestOptions,
    ) -> Result<EventReadResult, HostError> {
        self.read_event_stream_with_options(params, options).await
    }

    async fn close(&self, params: &EventCloseParams) -> Result<(), HostError> {
        self.close_event_stream(params).await
    }
}

impl EventStreamSubscription {
    pub fn spawn(
        client: ManagedUniversalPluginClient,
        stream_id: impl Into<String>,
        config: EventStreamSubscriptionConfig,
    ) -> Self {
        Self::spawn_with_client(client, stream_id, config)
    }

    pub(crate) fn spawn_with_client<C>(
        client: C,
        stream_id: impl Into<String>,
        config: EventStreamSubscriptionConfig,
    ) -> Self
    where
        C: EventStreamClient,
    {
        let stream_id = stream_id.into();
        let cancel = CancellationToken::new();
        let (sender, receiver) = mpsc::channel(config.channel_capacity.max(1));
        let task = tokio::spawn(run_pull_loop(
            client,
            stream_id.clone(),
            config,
            cancel.clone(),
            sender,
        ));
        Self {
            stream_id,
            receiver,
            cancel,
            task: Some(task),
        }
    }

    pub fn stream_id(&self) -> &str {
        &self.stream_id
    }

    pub async fn recv(&mut self) -> Option<Result<EventStreamBatch, HostError>> {
        self.receiver.recv().await
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub async fn close(mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for EventStreamSubscription {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn run_pull_loop(
    client: impl EventStreamClient,
    stream_id: String,
    config: EventStreamSubscriptionConfig,
    cancel: CancellationToken,
    sender: mpsc::Sender<Result<EventStreamBatch, HostError>>,
) {
    loop {
        let read_cancel = CancellationToken::new();
        let read_params = EventReadParams {
            stream_id: stream_id.clone(),
            max_events: Some(config.max_events.max(1)),
            wait_ms: Some(config.wait_ms),
        };
        let read = client.read(
            &read_params,
            RequestOptions::default().with_cancel(read_cancel.clone()),
        );
        tokio::pin!(read);
        let result = tokio::select! {
            result = &mut read => result,
            _ = cancel.cancelled() => {
                read_cancel.cancel();
                break;
            }
            _ = sender.closed() => {
                read_cancel.cancel();
                break;
            }
        };

        let should_stop = match result {
            Ok(result) => {
                let is_empty = result.events.is_empty() && result.dropped_count == 0;
                let closed = result.closed;
                let batch = EventStreamBatch {
                    events: result.events,
                    dropped_count: result.dropped_count,
                    closed,
                };
                if !is_empty || closed {
                    tokio::select! {
                        result = sender.send(Ok(batch)) => {
                            if result.is_err() {
                                true
                            } else {
                                closed
                            }
                        }
                        _ = cancel.cancelled() => true,
                        _ = sender.closed() => true,
                    }
                } else {
                    tokio::select! {
                        _ = sleep(EMPTY_READ_DELAY) => false,
                        _ = cancel.cancelled() => true,
                        _ = sender.closed() => true,
                    }
                }
            }
            Err(HostError::Cancelled { .. }) if cancel.is_cancelled() || sender.is_closed() => true,
            Err(error) => {
                let _ = sender.send(Err(error)).await;
                true
            }
        };
        if should_stop {
            break;
        }
    }

    let _ = tokio::time::timeout(
        EVENT_CLOSE_TIMEOUT,
        client.close(&EventCloseParams { stream_id }),
    )
    .await;
}
