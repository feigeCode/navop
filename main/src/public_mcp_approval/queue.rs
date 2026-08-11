use super::ApprovalEnvelope;
use public_mcp::approval::{PublicMcpApprovalOutcome, PublicMcpApprovalRequest};
use std::collections::VecDeque;

pub(super) const MAX_QUEUED_APPROVALS: usize = 64;

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
    pub fn enqueue(&mut self, envelope: ApprovalEnvelope) -> Result<(), ApprovalEnvelope> {
        if self.active.is_none() {
            self.active = Some(envelope);
            return Ok(());
        }
        if self.pending.len() >= MAX_QUEUED_APPROVALS.saturating_sub(1) {
            return Err(envelope);
        }
        self.pending.push_back(envelope);
        Ok(())
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
            operation: PublicMcpOperationKind::ExecuteRemoteCommand,
            tool_name: "ssh.exec".to_string(),
            summary: "Execute remote command".to_string(),
            details: json!({ "target": "ssh-1" }),
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

        assert!(queue.enqueue(first).is_ok());
        assert!(queue.enqueue(second).is_ok());

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

        assert!(queue.enqueue(first).is_ok());
        assert!(queue.enqueue(second).is_ok());
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

    #[test]
    fn approval_queue_rejects_requests_beyond_capacity() {
        let mut queue = ApprovalQueueState::default();
        let mut accepted_receivers = Vec::new();

        for index in 0..MAX_QUEUED_APPROVALS {
            let (envelope, receiver) = envelope(&format!("request {index}"));
            assert!(queue.enqueue(envelope).is_ok());
            accepted_receivers.push(receiver);
        }

        let (overflow, mut overflow_rx) = envelope("overflow request");
        let overflow = queue
            .enqueue(overflow)
            .expect_err("queue should reject requests beyond its capacity");
        overflow.deny("public MCP approval queue is full");

        assert_eq!(
            PublicMcpApprovalOutcome::Denied {
                reason: Some("public MCP approval queue is full".to_string())
            },
            overflow_rx
                .try_recv()
                .expect("overflow request should be denied")
        );
        assert_eq!(MAX_QUEUED_APPROVALS - 1, queue.pending.len());
        assert!(queue.active.is_some());
        assert!(
            accepted_receivers
                .iter_mut()
                .all(|receiver| receiver.try_recv().is_err())
        );
    }
}
