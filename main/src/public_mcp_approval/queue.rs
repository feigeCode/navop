use super::ApprovalEnvelope;
use public_mcp::approval::{PublicMcpApprovalOutcome, PublicMcpApprovalRequest};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct ApprovalQueueState {
    active: Option<ApprovalEnvelope>,
    pending: VecDeque<ApprovalEnvelope>,
    presenting: bool,
}

pub(super) struct ApprovalQueueSnapshot {
    pub active: PublicMcpApprovalRequest,
    pub pending_count: usize,
}

impl ApprovalQueueState {
    pub fn enqueue(&mut self, envelope: ApprovalEnvelope) {
        if self.active.is_none() {
            self.active = Some(envelope);
            return;
        }
        self.pending.push_back(envelope);
    }

    pub fn begin_presentation(&mut self) -> Option<ApprovalQueueSnapshot> {
        if self.presenting {
            return None;
        }
        let active = self.active.as_ref()?;
        self.presenting = true;
        Some(ApprovalQueueSnapshot {
            active: active.request.clone(),
            pending_count: self.pending.len(),
        })
    }

    pub fn resolve_active(&mut self, outcome: PublicMcpApprovalOutcome) -> bool {
        if let Some(active) = self.active.take() {
            active.resolve(outcome);
        }
        self.active = self.pending.pop_front();
        self.presenting = false;
        self.active.is_some()
    }

    pub fn deny_all(&mut self, reason: &'static str) {
        self.presenting = false;
        if let Some(active) = self.active.take() {
            active.deny(reason);
        }
        for pending in self.pending.drain(..) {
            pending.deny(reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use public_mcp::approval::{PublicMcpApprovalOutcome, PublicMcpApprovalRequest};
    use public_mcp::permissions::PublicMcpOperationKind;
    use serde_json::json;
    use tokio::sync::oneshot;

    fn request() -> PublicMcpApprovalRequest {
        PublicMcpApprovalRequest {
            operation: PublicMcpOperationKind::WriteTerminal,
            tool_name: "public_mcp.terminal_write".to_string(),
            summary: "Write to terminal".to_string(),
            details: json!({ "session_id": "ssh-1" }),
        }
    }

    fn envelope(
        summary: &str,
    ) -> (
        ApprovalEnvelope,
        oneshot::Receiver<PublicMcpApprovalOutcome>,
    ) {
        let (response_tx, response_rx) = oneshot::channel();
        (
            ApprovalEnvelope {
                request: PublicMcpApprovalRequest {
                    summary: summary.to_string(),
                    ..request()
                },
                response_tx,
            },
            response_rx,
        )
    }

    #[test]
    fn approval_queue_presents_one_active_request_and_advances_after_resolution() {
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

        assert!(queue.resolve_active(PublicMcpApprovalOutcome::Approved));
        assert_eq!(
            PublicMcpApprovalOutcome::Approved,
            first_rx.try_recv().expect("first approval should resolve")
        );

        let second_snapshot = queue
            .begin_presentation()
            .expect("queue should advance to the next approval");
        assert_eq!("second request", second_snapshot.active.summary);
        assert_eq!(0, second_snapshot.pending_count);
        assert!(second_rx.try_recv().is_err());
    }

    #[test]
    fn approval_queue_denies_active_and_pending_when_no_window_is_available() {
        let mut queue = ApprovalQueueState::default();
        let (first, mut first_rx) = envelope("first request");
        let (second, mut second_rx) = envelope("second request");

        queue.enqueue(first);
        queue.enqueue(second);
        queue.deny_all("no active window");

        assert_eq!(
            PublicMcpApprovalOutcome::Denied {
                reason: Some("no active window".to_string())
            },
            first_rx.try_recv().expect("first request should be denied")
        );
        assert_eq!(
            PublicMcpApprovalOutcome::Denied {
                reason: Some("no active window".to_string())
            },
            second_rx
                .try_recv()
                .expect("second request should be denied")
        );
        assert!(queue.begin_presentation().is_none());
    }
}
