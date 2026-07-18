use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

pub type AcpPublicMcpApprovalFuture =
    Pin<Box<dyn Future<Output = AcpPublicMcpApprovalOutcome> + Send + 'static>>;
pub type AcpPublicMcpApprovalProvider =
    Arc<dyn Fn(AcpPublicMcpApprovalRequest) -> AcpPublicMcpApprovalFuture + Send + Sync + 'static>;

#[derive(Clone, Debug, PartialEq)]
pub struct AcpPublicMcpApprovalRequest {
    pub request_id: String,
    pub tool_name: String,
    pub summary: String,
    pub details: Value,
}

impl AcpPublicMcpApprovalRequest {
    pub fn arguments(&self) -> Option<&Value> {
        self.details
            .get("requestArguments")
            .or_else(|| self.details.get("arguments"))
            .filter(|arguments| arguments.is_object())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcpPublicMcpApprovalOutcome {
    Approved,
    Denied,
}

pub(crate) enum AcpPublicMcpApprovalMessage {
    Requested(AcpPublicMcpApprovalEnvelope),
    Expired { request_id: String },
}

pub(crate) struct AcpPublicMcpApprovalEnvelope {
    request: AcpPublicMcpApprovalRequest,
    response_tx: oneshot::Sender<AcpPublicMcpApprovalOutcome>,
}

impl AcpPublicMcpApprovalEnvelope {
    pub(crate) fn new(
        request: AcpPublicMcpApprovalRequest,
    ) -> (Self, oneshot::Receiver<AcpPublicMcpApprovalOutcome>) {
        let (response_tx, response_rx) = oneshot::channel();
        (
            Self {
                request,
                response_tx,
            },
            response_rx,
        )
    }

    pub(crate) fn request(&self) -> &AcpPublicMcpApprovalRequest {
        &self.request
    }

    pub(crate) fn resolve(self, outcome: AcpPublicMcpApprovalOutcome) -> bool {
        self.response_tx.send(outcome).is_ok()
    }
}

pub(crate) fn acp_public_mcp_approval_channel() -> (
    AcpPublicMcpApprovalProvider,
    mpsc::UnboundedReceiver<AcpPublicMcpApprovalMessage>,
) {
    acp_public_mcp_approval_channel_with_timeout(APPROVAL_TIMEOUT)
}

fn acp_public_mcp_approval_channel_with_timeout(
    timeout: Duration,
) -> (
    AcpPublicMcpApprovalProvider,
    mpsc::UnboundedReceiver<AcpPublicMcpApprovalMessage>,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let provider: AcpPublicMcpApprovalProvider = Arc::new(move |request| {
        let sender = sender.clone();
        Box::pin(async move {
            let (envelope, response_rx) = AcpPublicMcpApprovalEnvelope::new(request);
            let request_id = envelope.request().request_id.clone();
            if sender
                .send(AcpPublicMcpApprovalMessage::Requested(envelope))
                .is_err()
            {
                return AcpPublicMcpApprovalOutcome::Denied;
            }
            match tokio::time::timeout(timeout, response_rx).await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(_)) => AcpPublicMcpApprovalOutcome::Denied,
                Err(_) => {
                    let _ = sender.send(AcpPublicMcpApprovalMessage::Expired { request_id });
                    AcpPublicMcpApprovalOutcome::Denied
                }
            }
        })
    });
    (provider, receiver)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AcpPublicMcpApprovalRequest {
        AcpPublicMcpApprovalRequest {
            request_id: "approval-1".to_string(),
            tool_name: "terminal.exec".to_string(),
            summary: "Call Execute in terminal".to_string(),
            details: serde_json::json!({
                "requestArguments": {
                    "target": "ssh-prod",
                    "command": "du -xhd1 /"
                }
            }),
        }
    }

    #[tokio::test]
    async fn approval_channel_delivers_full_arguments_and_returns_user_decision() {
        let (provider, mut receiver) = acp_public_mcp_approval_channel();
        let outcome = tokio::spawn(provider(request()));

        let message = receiver.recv().await.expect("approval request");
        let AcpPublicMcpApprovalMessage::Requested(envelope) = message else {
            panic!("expected requested message");
        };
        assert_eq!(
            Some("du -xhd1 /"),
            envelope
                .request()
                .arguments()
                .and_then(|arguments| arguments.get("command"))
                .and_then(Value::as_str)
        );
        assert!(envelope.resolve(AcpPublicMcpApprovalOutcome::Approved));

        assert_eq!(
            AcpPublicMcpApprovalOutcome::Approved,
            outcome.await.expect("approval task")
        );
    }

    #[tokio::test]
    async fn approval_channel_times_out_and_reports_expiration() {
        let (provider, mut receiver) = acp_public_mcp_approval_channel_with_timeout(Duration::ZERO);
        let outcome = tokio::spawn(provider(request()));

        let AcpPublicMcpApprovalMessage::Requested(envelope) =
            receiver.recv().await.expect("approval request")
        else {
            panic!("expected requested message");
        };
        assert_eq!(
            AcpPublicMcpApprovalOutcome::Denied,
            outcome.await.expect("approval task")
        );
        assert!(matches!(
            receiver.recv().await,
            Some(AcpPublicMcpApprovalMessage::Expired { request_id })
                if request_id == "approval-1"
        ));
        assert!(!envelope.resolve(AcpPublicMcpApprovalOutcome::Denied));
    }
}
