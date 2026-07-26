use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::ingress_queue::{
    BoundedTerminalSender, ReservedTerminalIngressItem, TerminalControlSendError,
    TerminalDataSendError, TerminalIngressBudget, TerminalIngressBudgetError, TerminalIngressItem,
    bounded_terminal_queue,
};

fn budget(bytes: u64, chunks: usize, controls: usize) -> TerminalIngressBudget {
    TerminalIngressBudget::new(bytes, chunks, controls).expect("budget should be valid")
}

async fn wait_until(condition: impl Fn() -> bool) {
    timeout(Duration::from_secs(1), async {
        loop {
            if condition() {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("condition should be reached");
}

async fn assert_blocked<T>(task: &JoinHandle<T>) {
    tokio::task::yield_now().await;
    assert!(!task.is_finished(), "operation should remain backpressured");
}

#[test]
fn rejects_invalid_budgets() {
    assert_eq!(
        Err(TerminalIngressBudgetError::ZeroPendingBytes),
        TerminalIngressBudget::new(0, 1, 1)
    );
    assert_eq!(
        Err(TerminalIngressBudgetError::ZeroPendingChunks),
        TerminalIngressBudget::new(1, 0, 1)
    );
    assert_eq!(
        Err(TerminalIngressBudgetError::ZeroPendingControls),
        TerminalIngressBudget::new(1, 1, 0)
    );
    assert!(matches!(
        TerminalIngressBudget::new(u64::from(u32::MAX) + 1, 1, 1),
        Err(TerminalIngressBudgetError::PendingBytesTooLarge { .. })
    ));

    let budget = TerminalIngressBudget::new(1024, 8, 3).expect("budget should be valid");
    assert_eq!(budget.max_pending_bytes(), 1024);
    assert_eq!(budget.max_pending_chunks(), 8);
    assert_eq!(budget.max_pending_controls(), 3);
}

#[tokio::test]
async fn rejects_empty_and_oversized_data_without_waiting() {
    let (sender, _receiver) = bounded_terminal_queue::<()>(budget(4, 1, 1));

    assert!(matches!(
        timeout(Duration::from_millis(100), sender.send_data(Vec::new()))
            .await
            .expect("empty data must fail immediately"),
        Err(TerminalDataSendError::Empty(_))
    ));
    assert!(matches!(
        timeout(Duration::from_millis(100), sender.send_data(vec![0; 5]),)
            .await
            .expect("oversized data must fail immediately"),
        Err(TerminalDataSendError::Oversized { max_bytes: 4, .. })
    ));
    assert_eq!(sender.pending_bytes(), 0);
}

#[tokio::test]
async fn errors_and_items_redact_terminal_payloads() {
    let (sender, mut receiver) = bounded_terminal_queue::<String>(budget(4, 1, 1));

    let error = sender
        .send_data(b"secret".to_vec())
        .await
        .expect_err("oversized data should be rejected");
    assert!(!format!("{error:?}").contains("secret"));

    sender
        .send_data(b"key".to_vec())
        .await
        .expect("data should be accepted");
    let item = receiver.recv().await.expect("item should be available");
    assert!(!format!("{item:?}").contains("key"));

    sender
        .send_control("control-secret".to_owned())
        .await
        .expect("control should be accepted");
    let item = receiver.recv().await.expect("control should be available");
    assert!(!format!("{item:?}").contains("control-secret"));

    let (closed_sender, closed_receiver) = bounded_terminal_queue::<String>(budget(4, 1, 1));
    drop(closed_receiver);
    let error = closed_sender
        .send_control("closed-secret".to_owned())
        .await
        .expect_err("closed control should be rejected");
    assert!(!format!("{error:?}").contains("closed-secret"));

    let (closed_data_sender, closed_data_receiver) = bounded_terminal_queue::<()>(budget(32, 1, 1));
    drop(closed_data_receiver);
    let error = closed_data_sender
        .send_data(b"closed-data-secret".to_vec())
        .await
        .expect_err("closed data should be rejected");
    assert!(!format!("{error:?}").contains("closed-data-secret"));
}

#[tokio::test]
async fn accounts_exact_bytes_and_releases_before_delivery() {
    let (sender, mut receiver) = bounded_terminal_queue::<()>(budget(4, 2, 1));

    sender
        .send_data(vec![1, 2, 3])
        .await
        .expect("data should be accepted");
    assert_eq!(sender.pending_bytes(), 3);
    assert_eq!(sender.peak_pending_bytes(), 3);
    assert_eq!(receiver.pending_bytes(), 3);
    assert_eq!(receiver.peak_pending_bytes(), 3);

    assert_eq!(
        receiver.recv().await,
        Some(TerminalIngressItem::Data(vec![1, 2, 3]))
    );
    assert_eq!(sender.pending_bytes(), 0);
    assert_eq!(receiver.pending_bytes(), 0);
    assert_eq!(receiver.peak_pending_bytes(), 3);
}

#[tokio::test]
async fn reserved_delivery_holds_bytes_until_consumer_drops_guard() {
    let (sender, mut receiver) = bounded_terminal_queue::<()>(budget(8, 2, 1));
    sender
        .send_data(b"secret".to_vec())
        .await
        .expect("data should be accepted");

    let blocked = tokio::spawn({
        let sender = sender.clone();
        async move { sender.send_data(vec![9, 10, 11]).await }
    });
    assert_blocked(&blocked).await;

    let item = receiver
        .recv_reserved()
        .await
        .expect("reserved item should be available");
    let ReservedTerminalIngressItem::Data(data) = item else {
        panic!("expected data item");
    };
    assert_eq!(data.as_slice(), b"secret");
    assert_eq!(data.len(), 6);
    assert!(!format!("{data:?}").contains("secret"));
    assert_eq!(receiver.pending_bytes(), 6);
    assert_blocked(&blocked).await;

    drop(data);
    blocked
        .await
        .expect("sender task should finish")
        .expect("data should be accepted after parser consumption");
    assert_eq!(sender.pending_bytes(), 3);
}

#[tokio::test]
async fn reserved_guard_into_vec_releases_bytes_before_returning() {
    let (sender, mut receiver) = bounded_terminal_queue::<()>(budget(4, 1, 1));
    sender
        .send_data(vec![7, 8, 9])
        .await
        .expect("data should be accepted");

    let item = receiver
        .recv_reserved()
        .await
        .expect("reserved item should be available");
    let ReservedTerminalIngressItem::Data(data) = item else {
        panic!("expected data item");
    };
    let bytes = data.into_vec();

    assert_eq!(bytes, vec![7, 8, 9]);
    assert_eq!(sender.pending_bytes(), 0);
    assert_eq!(receiver.pending_bytes(), 0);
}

#[tokio::test]
async fn abort_does_not_double_release_a_consuming_guard() {
    let (sender, mut receiver) = bounded_terminal_queue::<()>(budget(4, 1, 1));
    sender
        .send_data(vec![1, 2])
        .await
        .expect("data should be accepted");

    let item = receiver
        .recv_reserved()
        .await
        .expect("reserved item should be available");
    let ReservedTerminalIngressItem::Data(data) = item else {
        panic!("expected data item");
    };
    receiver.abort();

    assert!(receiver.recv_reserved().await.is_none());
    assert_eq!(sender.pending_bytes(), 2);
    drop(data);
    assert_eq!(sender.pending_bytes(), 0);
}

#[tokio::test]
async fn byte_budget_backpressures_until_data_is_received() {
    let (sender, mut receiver) = bounded_terminal_queue::<()>(budget(4, 2, 1));
    sender
        .send_data(vec![1; 3])
        .await
        .expect("initial data should be accepted");

    let blocked = tokio::spawn({
        let sender = sender.clone();
        async move { sender.send_data(vec![2; 2]).await }
    });
    assert_blocked(&blocked).await;
    assert_eq!(sender.pending_bytes(), 3);

    assert!(matches!(
        receiver.recv().await,
        Some(TerminalIngressItem::Data(_))
    ));
    blocked
        .await
        .expect("sender task should finish")
        .expect("data should be accepted after capacity is released");
    assert_eq!(sender.pending_bytes(), 2);
    assert_eq!(sender.peak_pending_bytes(), 3);
}

#[tokio::test]
async fn chunk_budget_backpressures_independently() {
    let (sender, mut receiver) = bounded_terminal_queue::<()>(budget(8, 1, 1));
    sender
        .send_data(vec![1])
        .await
        .expect("initial data should be accepted");

    let blocked = tokio::spawn({
        let sender = sender.clone();
        async move { sender.send_data(vec![2]).await }
    });
    wait_until(|| sender.pending_bytes() == 2).await;
    assert_blocked(&blocked).await;

    assert!(matches!(
        receiver.recv().await,
        Some(TerminalIngressItem::Data(_))
    ));
    blocked
        .await
        .expect("sender task should finish")
        .expect("data should be accepted after a chunk slot is released");
    assert_eq!(sender.pending_bytes(), 1);
    assert_eq!(sender.peak_pending_bytes(), 2);
}

#[tokio::test]
async fn control_budget_backpressures_independently() {
    let (sender, mut receiver) = bounded_terminal_queue::<u8>(budget(1, 1, 1));
    sender
        .send_control(1)
        .await
        .expect("initial control should be accepted");

    let blocked = tokio::spawn({
        let sender = sender.clone();
        async move { sender.send_control(2).await }
    });
    assert_blocked(&blocked).await;
    assert_eq!(sender.pending_bytes(), 0);

    assert_eq!(receiver.recv().await, Some(TerminalIngressItem::Control(1)));
    blocked
        .await
        .expect("control task should finish")
        .expect("control should be accepted after capacity is released");
}

#[tokio::test]
async fn ready_control_bypasses_blocked_data_and_is_preferred() {
    let (sender, mut receiver) = bounded_terminal_queue::<u8>(budget(1, 1, 1));
    sender
        .send_data(vec![1])
        .await
        .expect("initial data should be accepted");
    let blocked = tokio::spawn({
        let sender = sender.clone();
        async move { sender.send_data(vec![2]).await }
    });
    sender
        .send_control(9)
        .await
        .expect("control should bypass data byte pressure");
    assert_blocked(&blocked).await;

    assert_eq!(receiver.recv().await, Some(TerminalIngressItem::Control(9)));
    assert_eq!(
        receiver.recv().await,
        Some(TerminalIngressItem::Data(vec![1]))
    );
    blocked
        .await
        .expect("data task should finish")
        .expect("data should be accepted after capacity is released");
}

#[tokio::test]
async fn abort_wakes_every_waiter_and_discards_backlog() {
    let (sender, mut receiver) = bounded_terminal_queue::<u8>(budget(3, 1, 1));
    sender
        .send_data(vec![1])
        .await
        .expect("initial data should be accepted");
    sender
        .send_control(1)
        .await
        .expect("initial control should be accepted");

    let chunk_waiter = spawn_data(&sender, vec![2]);
    wait_until(|| sender.pending_bytes() == 2).await;
    let byte_waiter = spawn_data(&sender, vec![3; 2]);
    let control_waiter = spawn_control(&sender, 2);
    assert_blocked(&byte_waiter).await;
    assert_blocked(&control_waiter).await;

    sender.abort();
    assert!(matches!(
        timeout(Duration::from_secs(1), chunk_waiter)
            .await
            .expect("chunk waiter should wake")
            .expect("chunk waiter should not panic"),
        Err(TerminalDataSendError::Closed(_))
    ));
    assert!(matches!(
        timeout(Duration::from_secs(1), byte_waiter)
            .await
            .expect("byte waiter should wake")
            .expect("byte waiter should not panic"),
        Err(TerminalDataSendError::Closed(_))
    ));
    assert!(matches!(
        timeout(Duration::from_secs(1), control_waiter)
            .await
            .expect("control waiter should wake")
            .expect("control waiter should not panic"),
        Err(TerminalControlSendError::Closed(2))
    ));
    assert!(matches!(
        sender.send_data(vec![4]).await,
        Err(TerminalDataSendError::Closed(_))
    ));
    assert!(matches!(
        sender.send_control(4).await,
        Err(TerminalControlSendError::Closed(4))
    ));
    assert_eq!(receiver.recv().await, None);
    assert_eq!(sender.pending_bytes(), 0);
    assert_eq!(receiver.peak_pending_bytes(), 2);
}

#[tokio::test]
async fn receiver_abort_discards_backlog_immediately() {
    let (sender, mut receiver) = bounded_terminal_queue::<u8>(budget(4, 2, 1));
    sender
        .send_data(vec![1, 2])
        .await
        .expect("data should be accepted");
    sender
        .send_control(7)
        .await
        .expect("control should be accepted");

    receiver.abort();

    assert_eq!(receiver.pending_bytes(), 0);
    assert_eq!(receiver.recv().await, None);
    assert_eq!(sender.pending_bytes(), 0);
}

#[tokio::test]
async fn receiver_drop_aborts_waiters() {
    let (sender, receiver) = bounded_terminal_queue::<u8>(budget(3, 1, 1));
    sender
        .send_data(vec![1])
        .await
        .expect("initial data should be accepted");
    sender
        .send_control(1)
        .await
        .expect("initial control should be accepted");
    let chunk_waiter = spawn_data(&sender, vec![2]);
    wait_until(|| sender.pending_bytes() == 2).await;
    let byte_waiter = spawn_data(&sender, vec![3; 2]);
    let control_waiter = spawn_control(&sender, 2);
    assert_blocked(&byte_waiter).await;
    assert_blocked(&control_waiter).await;

    drop(receiver);

    assert!(matches!(
        timeout(Duration::from_secs(1), chunk_waiter)
            .await
            .expect("chunk waiter should wake")
            .expect("chunk waiter should not panic"),
        Err(TerminalDataSendError::Closed(_))
    ));
    assert!(matches!(
        timeout(Duration::from_secs(1), byte_waiter)
            .await
            .expect("byte waiter should wake")
            .expect("byte waiter should not panic"),
        Err(TerminalDataSendError::Closed(_))
    ));
    assert!(matches!(
        timeout(Duration::from_secs(1), control_waiter)
            .await
            .expect("control waiter should wake")
            .expect("control waiter should not panic"),
        Err(TerminalControlSendError::Closed(2))
    ));
    assert!(matches!(
        sender.send_control(3).await,
        Err(TerminalControlSendError::Closed(3))
    ));
    assert_eq!(sender.pending_bytes(), 0);
}

#[tokio::test]
async fn last_sender_drop_gracefully_drains_accepted_items() {
    let (sender, mut receiver) = bounded_terminal_queue::<u8>(budget(4, 1, 1));
    sender
        .send_data(vec![1, 2])
        .await
        .expect("data should be accepted");
    sender
        .send_control(7)
        .await
        .expect("control should be accepted");
    drop(sender);

    assert_eq!(receiver.recv().await, Some(TerminalIngressItem::Control(7)));
    assert_eq!(
        receiver.recv().await,
        Some(TerminalIngressItem::Data(vec![1, 2]))
    );
    assert_eq!(receiver.pending_bytes(), 0);
    assert_eq!(receiver.recv().await, None);
}

#[tokio::test]
async fn cancelled_send_releases_reserved_bytes_without_double_release() {
    let (sender, mut receiver) = bounded_terminal_queue::<()>(budget(4, 1, 1));
    sender
        .send_data(vec![1])
        .await
        .expect("initial data should be accepted");
    let blocked = spawn_data(&sender, vec![2; 2]);
    wait_until(|| sender.pending_bytes() == 3).await;

    blocked.abort();
    assert!(
        blocked
            .await
            .expect_err("task should be cancelled")
            .is_cancelled()
    );
    wait_until(|| sender.pending_bytes() == 1).await;

    let full_send = tokio::spawn({
        let sender = sender.clone();
        async move { sender.send_data(vec![9; 4]).await }
    });
    assert_blocked(&full_send).await;

    assert!(matches!(
        receiver.recv().await,
        Some(TerminalIngressItem::Data(_))
    ));
    full_send
        .await
        .expect("full sender should finish")
        .expect("all permits should be available exactly once");
    assert_eq!(sender.pending_bytes(), 4);
    assert_eq!(sender.peak_pending_bytes(), 4);
}

fn spawn_data<C: Send + 'static>(
    sender: &BoundedTerminalSender<C>,
    data: Vec<u8>,
) -> JoinHandle<Result<(), TerminalDataSendError>> {
    let sender = sender.clone();
    tokio::spawn(async move { sender.send_data(data).await })
}

fn spawn_control<C: Send + 'static>(
    sender: &BoundedTerminalSender<C>,
    control: C,
) -> JoinHandle<Result<(), TerminalControlSendError<C>>> {
    let sender = sender.clone();
    tokio::spawn(async move { sender.send_control(control).await })
}
