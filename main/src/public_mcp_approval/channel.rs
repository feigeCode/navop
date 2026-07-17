use super::ApprovalEnvelope;
use public_mcp::approval::{
    PublicMcpApprovalFuture, PublicMcpApprovalOutcome, PublicMcpApprovalRequest, PublicMcpApprover,
};
use public_mcp::approval_grants::PublicMcpApprovalGrantStore;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub(super) struct ChannelApprover {
    sender: mpsc::UnboundedSender<ApprovalEnvelope>,
    timeout: Duration,
    grants: PublicMcpApprovalGrantStore,
}

impl PublicMcpApprover for ChannelApprover {
    fn request_approval(&self, request: PublicMcpApprovalRequest) -> PublicMcpApprovalFuture {
        if self.grants.consume(&request) {
            return Box::pin(async { PublicMcpApprovalOutcome::Approved });
        }
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
    grants: PublicMcpApprovalGrantStore,
) -> (ChannelApprover, mpsc::UnboundedReceiver<ApprovalEnvelope>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (
        ChannelApprover {
            sender,
            timeout,
            grants,
        },
        receiver,
    )
}

#[cfg(test)]
fn channel_approver_for_tests() -> (ChannelApprover, mpsc::UnboundedReceiver<ApprovalEnvelope>) {
    channel_approver(
        Duration::from_secs(10),
        PublicMcpApprovalGrantStore::new(Duration::from_secs(10)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use public_mcp::approval::{PublicMcpApprovalOutcome, PublicMcpApprovalRequest};
    use public_mcp::permissions::PublicMcpOperationKind;
    use serde_json::json;

    fn request() -> PublicMcpApprovalRequest {
        PublicMcpApprovalRequest {
            operation: PublicMcpOperationKind::ExecuteRemoteCommand,
            tool_name: "ssh.exec".to_string(),
            summary: "Execute remote command".to_string(),
            details: json!({ "target": "ssh-1" }),
        }
    }

    fn runtime_request() -> PublicMcpApprovalRequest {
        PublicMcpApprovalRequest {
            operation: PublicMcpOperationKind::CallToolRuntimeTool,
            tool_name: "ssh.exec".to_string(),
            summary: "Call Execute remote command".to_string(),
            details: json!({
                "tool": "ssh.exec",
                "requestArguments": { "target": "ssh-1" },
                "arguments": { "target": "ssh-1" },
            }),
        }
    }

    #[tokio::test]
    async fn channel_approver_resolves_with_queue_response() {
        let (approver, mut receiver) = channel_approver_for_tests();
        let approval = tokio::spawn(async move { approver.request_approval(request()).await });

        let envelope = receiver.recv().await.expect("request should be queued");
        assert_eq!("ssh.exec", envelope.request.tool_name);
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

    #[tokio::test]
    async fn matching_acp_grant_skips_exactly_one_dialog_queue_request() {
        let grants = PublicMcpApprovalGrantStore::new(Duration::from_secs(10));
        grants
            .register(json!({ "target": "ssh-1" }))
            .expect("valid ACP arguments should create a grant");
        let (approver, mut receiver) = channel_approver(Duration::from_secs(10), grants);

        assert_eq!(
            PublicMcpApprovalOutcome::Approved,
            approver.request_approval(runtime_request()).await
        );
        assert!(receiver.try_recv().is_err());

        let approval = tokio::spawn({
            let approver = approver.clone();
            async move { approver.request_approval(runtime_request()).await }
        });
        let envelope = receiver
            .recv()
            .await
            .expect("the consumed grant must not approve a second request");
        envelope.approve();
        assert_eq!(
            PublicMcpApprovalOutcome::Approved,
            approval.await.expect("approval future should finish")
        );
    }
}
