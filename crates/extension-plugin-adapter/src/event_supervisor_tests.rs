use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use extension_host::{HostError, RequestOptions};
use extension_protocol::event_stream::{EventCloseParams, EventReadParams, EventReadResult};
use parking_lot::Mutex;
use serde_json::json;

use crate::event_supervisor::{
    EventStreamClient, EventStreamSubscription, EventStreamSubscriptionConfig,
};

#[derive(Clone)]
struct TestClient {
    state: Arc<TestClientState>,
}

struct TestClientState {
    reads: Mutex<VecDeque<TestRead>>,
    read_calls: AtomicUsize,
    close_calls: AtomicUsize,
    cancel_observed: AtomicBool,
    last_params: Mutex<Option<EventReadParams>>,
}

enum TestRead {
    Result(Result<EventReadResult, HostError>),
    PendingUntilCancelled,
    NeverCompletes,
}

impl TestClient {
    fn new(reads: impl IntoIterator<Item = TestRead>) -> Self {
        Self {
            state: Arc::new(TestClientState {
                reads: Mutex::new(reads.into_iter().collect()),
                read_calls: AtomicUsize::new(0),
                close_calls: AtomicUsize::new(0),
                cancel_observed: AtomicBool::new(false),
                last_params: Mutex::new(None),
            }),
        }
    }
}

#[async_trait::async_trait]
impl EventStreamClient for TestClient {
    async fn read(
        &self,
        params: &EventReadParams,
        options: RequestOptions,
    ) -> Result<EventReadResult, HostError> {
        self.state.read_calls.fetch_add(1, Ordering::SeqCst);
        *self.state.last_params.lock() = Some(params.clone());
        let read = self
            .state
            .reads
            .lock()
            .pop_front()
            .unwrap_or(TestRead::PendingUntilCancelled);
        match read {
            TestRead::Result(result) => result,
            TestRead::PendingUntilCancelled => {
                let token = options.cancel.expect("supervisor supplies cancellation");
                token.cancelled().await;
                self.state.cancel_observed.store(true, Ordering::SeqCst);
                Err(HostError::Cancelled {
                    method: "event/read".into(),
                })
            }
            TestRead::NeverCompletes => std::future::pending().await,
        }
    }

    async fn close(&self, _params: &EventCloseParams) -> Result<(), HostError> {
        self.state.close_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn read(events: serde_json::Value, dropped_count: u64, closed: bool) -> TestRead {
    TestRead::Result(Ok(EventReadResult {
        events: events.as_array().cloned().unwrap_or_default(),
        dropped_count,
        closed,
    }))
}

#[tokio::test]
async fn forwards_batches_and_read_configuration() {
    let client = TestClient::new([
        read(json!([{"kind": "ready"}]), 3, false),
        TestRead::PendingUntilCancelled,
    ]);
    let mut subscription = EventStreamSubscription::spawn_with_client(
        client.clone(),
        "stream-1",
        EventStreamSubscriptionConfig {
            channel_capacity: 2,
            max_events: 7,
            wait_ms: 23,
        },
    );

    let batch = subscription.recv().await.unwrap().unwrap();
    assert_eq!(vec![json!({"kind": "ready"})], batch.events);
    assert_eq!(3, batch.dropped_count);
    let params = client.state.last_params.lock().clone().unwrap();
    assert_eq!(Some(7), params.max_events);
    assert_eq!(Some(23), params.wait_ms);
    subscription.close().await;
    assert_eq!(1, client.state.close_calls.load(Ordering::SeqCst));
}

#[tokio::test]
async fn suppresses_empty_batches_until_data_arrives() {
    let client = TestClient::new([
        read(json!([]), 0, false),
        read(json!([1]), 0, false),
        TestRead::PendingUntilCancelled,
    ]);
    let mut subscription = EventStreamSubscription::spawn_with_client(
        client.clone(),
        "stream-1",
        EventStreamSubscriptionConfig::default(),
    );

    let batch = subscription.recv().await.unwrap().unwrap();
    assert_eq!(vec![json!(1)], batch.events);
    assert!(client.state.read_calls.load(Ordering::SeqCst) >= 2);
    subscription.close().await;
}

#[tokio::test]
async fn terminal_batch_stops_polling_and_closes_once() {
    let client = TestClient::new([read(json!([]), 0, true)]);
    let mut subscription = EventStreamSubscription::spawn_with_client(
        client.clone(),
        "stream-1",
        EventStreamSubscriptionConfig::default(),
    );

    assert!(subscription.recv().await.unwrap().unwrap().closed);
    subscription.close().await;
    assert_eq!(1, client.state.read_calls.load(Ordering::SeqCst));
    assert_eq!(1, client.state.close_calls.load(Ordering::SeqCst));
}

#[tokio::test]
async fn read_failure_is_forwarded_and_stream_is_closed() {
    let client = TestClient::new([TestRead::Result(Err(HostError::Closed))]);
    let mut subscription = EventStreamSubscription::spawn_with_client(
        client.clone(),
        "stream-1",
        EventStreamSubscriptionConfig::default(),
    );

    assert!(matches!(
        subscription.recv().await.unwrap(),
        Err(HostError::Closed)
    ));
    assert!(subscription.recv().await.is_none());
    subscription.close().await;
    assert_eq!(1, client.state.close_calls.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancellation_interrupts_pending_read_and_cleanup() {
    let client = TestClient::new([TestRead::PendingUntilCancelled]);
    let subscription = EventStreamSubscription::spawn_with_client(
        client.clone(),
        "stream-1",
        EventStreamSubscriptionConfig::default(),
    );

    tokio::time::timeout(Duration::from_secs(1), async {
        while client.state.read_calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    subscription.cancel();
    tokio::time::timeout(Duration::from_secs(1), subscription.close())
        .await
        .unwrap();
    assert_eq!(1, client.state.close_calls.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancel_does_not_wait_for_uncooperative_read() {
    let client = TestClient::new([TestRead::NeverCompletes]);
    let subscription = EventStreamSubscription::spawn_with_client(
        client.clone(),
        "stream-1",
        EventStreamSubscriptionConfig::default(),
    );
    wait_for_count(&client.state.read_calls, 1).await;

    tokio::time::timeout(Duration::from_secs(1), subscription.close())
        .await
        .expect("close must not depend on provider cancellation cooperation");
    assert_eq!(1, client.state.close_calls.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancel_releases_full_channel_backpressure() {
    let client = TestClient::new([
        read(json!([1]), 0, false),
        read(json!([2]), 0, false),
        TestRead::NeverCompletes,
    ]);
    let subscription = EventStreamSubscription::spawn_with_client(
        client.clone(),
        "stream-1",
        EventStreamSubscriptionConfig {
            channel_capacity: 1,
            ..Default::default()
        },
    );
    wait_for_count(&client.state.read_calls, 2).await;

    tokio::time::timeout(Duration::from_secs(1), subscription.close())
        .await
        .expect("cancel must release a blocked bounded-channel send");
    assert_eq!(1, client.state.close_calls.load(Ordering::SeqCst));
}

#[tokio::test]
async fn dropping_subscription_cancels_pending_read_and_closes_stream() {
    let client = TestClient::new([TestRead::NeverCompletes]);
    let subscription = EventStreamSubscription::spawn_with_client(
        client.clone(),
        "stream-1",
        EventStreamSubscriptionConfig::default(),
    );
    wait_for_count(&client.state.read_calls, 1).await;
    drop(subscription);

    tokio::time::timeout(Duration::from_secs(1), async {
        wait_for_count(&client.state.close_calls, 1).await;
    })
    .await
    .expect("drop must detach the read and close the provider stream");
}

async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
    while counter.load(Ordering::SeqCst) < expected {
        tokio::task::yield_now().await;
    }
}
