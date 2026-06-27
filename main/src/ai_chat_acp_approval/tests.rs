use super::*;
use ai_chat_view::{AcpPermissionOption, AcpPermissionOutcome, AcpPermissionRequest};
use serde_json::json;
use tokio::sync::oneshot;

fn request(summary: &str) -> AcpPermissionRequest {
    AcpPermissionRequest {
        session_id: "session-1".to_string(),
        tool_name: "filesystem.write".to_string(),
        summary: summary.to_string(),
        details: json!({ "path": "/tmp/file.txt" }),
        options: vec![
            AcpPermissionOption {
                option_id: "reject-once".to_string(),
                name: "Reject".to_string(),
                kind: "reject_once".to_string(),
            },
            AcpPermissionOption {
                option_id: "allow-once".to_string(),
                name: "Allow".to_string(),
                kind: "allow_once".to_string(),
            },
            AcpPermissionOption {
                option_id: "allow-always".to_string(),
                name: "Always Allow".to_string(),
                kind: "allow_always".to_string(),
            },
        ],
    }
}

fn envelope(summary: &str) -> (ApprovalEnvelope, oneshot::Receiver<AcpPermissionOutcome>) {
    let (response_tx, response_rx) = oneshot::channel();
    (
        ApprovalEnvelope {
            request: request(summary),
            response_tx,
        },
        response_rx,
    )
}

#[test]
fn approval_queue_presents_active_request_and_advances_after_selection() {
    let mut queue = ApprovalQueueState::default();
    let (first, mut first_rx) = envelope("first request");
    let (second, mut second_rx) = envelope("second request");

    queue.enqueue(first);
    queue.enqueue(second);

    let first_snapshot = queue
        .begin_presentation()
        .expect("queue should present the active approval");
    assert_eq!("first request", first_snapshot.active.summary);
    assert_eq!(1, first_snapshot.pending_count);
    assert!(queue.begin_presentation().is_none());

    assert!(queue.resolve_active(AcpPermissionOutcome::Selected {
        option_id: "allow-always".to_string(),
    }));
    assert_eq!(
        AcpPermissionOutcome::Selected {
            option_id: "allow-always".to_string(),
        },
        first_rx.try_recv().expect("first request should resolve")
    );

    let second_snapshot = queue
        .begin_presentation()
        .expect("queue should advance to the next approval");
    assert_eq!("second request", second_snapshot.active.summary);
    assert_eq!(0, second_snapshot.pending_count);
    assert!(second_rx.try_recv().is_err());
}

#[test]
fn approval_queue_cancels_active_and_pending_requests() {
    let mut queue = ApprovalQueueState::default();
    let (first, mut first_rx) = envelope("first request");
    let (second, mut second_rx) = envelope("second request");

    queue.enqueue(first);
    queue.enqueue(second);
    queue.cancel_all();

    assert_eq!(
        AcpPermissionOutcome::Cancelled,
        first_rx
            .try_recv()
            .expect("first request should be cancelled")
    );
    assert_eq!(
        AcpPermissionOutcome::Cancelled,
        second_rx
            .try_recv()
            .expect("second request should be cancelled")
    );
    assert!(queue.begin_presentation().is_none());
}

#[tokio::test]
async fn request_acp_approval_returns_selected_outcome_from_channel() {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let response = tokio::spawn(request_acp_approval(sender, request("channel request")));
    let envelope = receiver
        .recv()
        .await
        .expect("approval request should be queued");

    assert_eq!("channel request", envelope.request.summary);
    envelope.resolve(AcpPermissionOutcome::Selected {
        option_id: "allow-once".to_string(),
    });

    assert_eq!(
        AcpPermissionOutcome::Selected {
            option_id: "allow-once".to_string(),
        },
        response.await.expect("approval task should finish")
    );
}
