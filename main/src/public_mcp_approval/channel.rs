use super::ApprovalEnvelope;
use public_mcp::approval::{
    PublicMcpApprovalFuture, PublicMcpApprovalOutcome, PublicMcpApprovalRequest, PublicMcpApprover,
};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub(super) struct ChannelApprover {
    sender: mpsc::UnboundedSender<ApprovalEnvelope>,
    timeout: Duration,
}

impl PublicMcpApprover for ChannelApprover {
    fn request_approval(&self, request: PublicMcpApprovalRequest) -> PublicMcpApprovalFuture {
        let sender = self.sender.clone();
        let timeout = self.timeout;
        Box::pin(async move {
            let (response_tx, response_rx) = oneshot::channel();
            if sender
                .send(ApprovalEnvelope {
                    request,
                    response_tx,
                })
                .is_err()
            {
                return PublicMcpApprovalOutcome::Denied {
                    reason: Some("public MCP approval queue is not available".to_string()),
                };
            }

            match tokio::time::timeout(timeout, response_rx).await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(_)) => PublicMcpApprovalOutcome::Denied {
                    reason: Some("public MCP approval response was dropped".to_string()),
                },
                Err(_) => PublicMcpApprovalOutcome::Denied {
                    reason: Some("public MCP approval request timed out".to_string()),
                },
            }
        })
    }
}

pub(super) fn channel_approver(
    timeout: Duration,
) -> (ChannelApprover, mpsc::UnboundedReceiver<ApprovalEnvelope>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (ChannelApprover { sender, timeout }, receiver)
}

#[cfg(test)]
fn channel_approver_for_tests() -> (ChannelApprover, mpsc::UnboundedReceiver<ApprovalEnvelope>) {
    channel_approver(Duration::from_secs(10))
}

#[cfg(test)]
mod tests {
    use super::*;
    use public_mcp::approval::{PublicMcpApprovalOutcome, PublicMcpApprovalRequest};
    use public_mcp::permissions::PublicMcpOperationKind;
    use serde_json::json;

    fn request() -> PublicMcpApprovalRequest {
        PublicMcpApprovalRequest {
            operation: PublicMcpOperationKind::WriteTerminal,
            tool_name: "public_mcp.terminal_write".to_string(),
            summary: "Write to terminal".to_string(),
            details: json!({ "session_id": "ssh-1" }),
        }
    }

    #[tokio::test]
    async fn channel_approver_resolves_with_queue_response() {
        let (approver, mut receiver) = channel_approver_for_tests();
        let approval = tokio::spawn(async move { approver.request_approval(request()).await });

        let envelope = receiver.recv().await.expect("request should be queued");
        assert_eq!("public_mcp.terminal_write", envelope.request.tool_name);
        envelope.approve();

        assert_eq!(
            PublicMcpApprovalOutcome::Approved,
            approval.await.expect("approval future should finish")
        );
    }

    #[tokio::test]
    async fn channel_approver_denies_when_queue_is_closed() {
        let (approver, receiver) = channel_approver_for_tests();
        drop(receiver);

        let outcome = approver.request_approval(request()).await;

        assert_eq!(
            PublicMcpApprovalOutcome::Denied {
                reason: Some("public MCP approval queue is not available".to_string())
            },
            outcome
        );
    }
}
