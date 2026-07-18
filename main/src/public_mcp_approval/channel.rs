use super::{AcpApprovalRoute, ApprovalEnvelope};
use ai_chat_view::{AcpPublicMcpApprovalOutcome, AcpPublicMcpApprovalRequest};
use public_mcp::approval::{
    PublicMcpApprovalFuture, PublicMcpApprovalOutcome, PublicMcpApprovalRequest, PublicMcpApprover,
};
use public_mcp::approval_grants::PublicMcpApprovalGrantStore;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

static NEXT_MESSAGE_APPROVAL_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub(super) struct ChannelApprover {
    sender: mpsc::UnboundedSender<ApprovalEnvelope>,
    timeout: Duration,
    routes: PublicMcpApprovalGrantStore<AcpApprovalRoute>,
}

impl PublicMcpApprover for ChannelApprover {
    fn request_approval(&self, request: PublicMcpApprovalRequest) -> PublicMcpApprovalFuture {
        if let Some(route) = self.routes.take(&request) {
            let request = AcpPublicMcpApprovalRequest {
                request_id: format!(
                    "public-mcp:{}",
                    NEXT_MESSAGE_APPROVAL_ID.fetch_add(1, Ordering::Relaxed)
                ),
                tool_name: request.tool_name,
                summary: request.summary,
                details: request.details,
            };
            return Box::pin(async move {
                match (route.provider)(request).await {
                    AcpPublicMcpApprovalOutcome::Approved => PublicMcpApprovalOutcome::Approved,
                    AcpPublicMcpApprovalOutcome::Denied => PublicMcpApprovalOutcome::Denied {
                        reason: Some("operator denied public MCP request".to_string()),
                    },
                }
            });
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
    routes: PublicMcpApprovalGrantStore<AcpApprovalRoute>,
) -> (ChannelApprover, mpsc::UnboundedReceiver<ApprovalEnvelope>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (
        ChannelApprover {
            sender,
            timeout,
            routes,
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
    use std::sync::{Arc, Mutex};

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
    async fn matching_acp_route_uses_message_approval_before_dialog_fallback() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let provider: ai_chat_view::AcpPublicMcpApprovalProvider = Arc::new({
            let seen = seen.clone();
            move |request| {
                seen.lock().expect("seen lock").push(request);
                Box::pin(async { AcpPublicMcpApprovalOutcome::Approved })
            }
        });
        let routes = PublicMcpApprovalGrantStore::new(Duration::from_secs(10));
        routes
            .register_payload(
                Some(json!({ "target": "ssh-1" })),
                AcpApprovalRoute { provider },
            )
            .expect("valid ACP route should be registered");
        let (approver, mut receiver) = channel_approver(Duration::from_secs(10), routes);

        assert_eq!(
            PublicMcpApprovalOutcome::Approved,
            approver.request_approval(runtime_request()).await
        );
        assert!(receiver.try_recv().is_err());
        let seen = seen.lock().expect("seen lock");
        assert_eq!(1, seen.len());
        assert_eq!("ssh.exec", seen[0].tool_name);
        assert_eq!(
            Some("ssh-1"),
            seen[0]
                .arguments()
                .and_then(|arguments| arguments.get("target"))
                .and_then(serde_json::Value::as_str)
        );
        drop(seen);

        let approval = tokio::spawn({
            let approver = approver.clone();
            async move { approver.request_approval(runtime_request()).await }
        });
        let envelope = receiver
            .recv()
            .await
            .expect("the consumed route must not capture a second request");
        envelope.approve();
        assert_eq!(
            PublicMcpApprovalOutcome::Approved,
            approval.await.expect("approval future should finish")
        );
    }
}
