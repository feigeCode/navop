use agent_client_protocol::schema::{
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome,
};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

pub type AcpPermissionFuture = Pin<Box<dyn Future<Output = AcpPermissionOutcome> + Send + 'static>>;
pub type AcpPermissionProvider =
    Arc<dyn Fn(AcpPermissionRequest) -> AcpPermissionFuture + Send + Sync + 'static>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpPermissionRequest {
    pub request_id: String,
    pub session_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub summary: String,
    pub details: Value,
    pub options: Vec<AcpPermissionOption>,
}

impl AcpPermissionRequest {
    pub fn preferred_allow_option_id(&self) -> Option<String> {
        self.options
            .iter()
            .find(|option| option.kind.starts_with("allow"))
            .or_else(|| self.options.first())
            .map(|option| option.option_id.clone())
    }

    pub fn raw_input(&self) -> Option<&Value> {
        self.details
            .get("rawInput")
            .or_else(|| self.details.get("raw_input"))
            .filter(|arguments| arguments.is_object())
    }

    pub(crate) fn use_fallback_raw_input(&mut self, arguments: Value) {
        if self.raw_input().is_some() || !arguments.is_object() {
            return;
        }
        if let Some(details) = self.details.as_object_mut() {
            details.insert("rawInput".to_string(), arguments);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpPermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcpPermissionOutcome {
    Selected { option_id: String },
    Cancelled,
}

pub(crate) enum AcpPermissionMessage {
    Requested(AcpPermissionEnvelope),
    Expired { request_id: String },
}

pub(crate) struct AcpPermissionEnvelope {
    request: AcpPermissionRequest,
    response_tx: oneshot::Sender<AcpPermissionOutcome>,
}

impl AcpPermissionEnvelope {
    pub(crate) fn new(
        request: AcpPermissionRequest,
    ) -> (Self, oneshot::Receiver<AcpPermissionOutcome>) {
        let (response_tx, response_rx) = oneshot::channel();
        (
            Self {
                request,
                response_tx,
            },
            response_rx,
        )
    }

    pub(crate) fn request(&self) -> &AcpPermissionRequest {
        &self.request
    }

    pub(crate) fn resolve(self, outcome: AcpPermissionOutcome) -> bool {
        self.response_tx.send(outcome).is_ok()
    }
}

pub(crate) fn acp_permission_channel() -> (
    AcpPermissionProvider,
    mpsc::UnboundedReceiver<AcpPermissionMessage>,
) {
    acp_permission_channel_with_timeout(APPROVAL_TIMEOUT)
}

fn acp_permission_channel_with_timeout(
    timeout: Duration,
) -> (
    AcpPermissionProvider,
    mpsc::UnboundedReceiver<AcpPermissionMessage>,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let channel_id = uuid::Uuid::new_v4().to_string();
    let provider: AcpPermissionProvider = Arc::new(move |mut request| {
        let sender = sender.clone();
        request.request_id = format!("{channel_id}:{}", request.request_id);
        Box::pin(async move {
            let (envelope, response_rx) = AcpPermissionEnvelope::new(request);
            let request_id = envelope.request().request_id.clone();
            if sender
                .send(AcpPermissionMessage::Requested(envelope))
                .is_err()
            {
                return AcpPermissionOutcome::Cancelled;
            }
            match tokio::time::timeout(timeout, response_rx).await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(_)) => AcpPermissionOutcome::Cancelled,
                Err(_) => {
                    let _ = sender.send(AcpPermissionMessage::Expired { request_id });
                    AcpPermissionOutcome::Cancelled
                }
            }
        })
    });
    (provider, receiver)
}

pub(crate) async fn resolve_acp_permission_request(
    provider: Option<AcpPermissionProvider>,
    request: RequestPermissionRequest,
) -> RequestPermissionResponse {
    let Some(provider) = provider else {
        return cancelled_permission_response();
    };
    let allowed_ids = request
        .options
        .iter()
        .map(|option| option.option_id.0.to_string())
        .collect::<Vec<_>>();
    let outcome = provider(acp_permission_request(request)).await;
    match outcome {
        AcpPermissionOutcome::Selected { option_id } if allowed_ids.contains(&option_id) => {
            RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
                SelectedPermissionOutcome::new(option_id),
            ))
        }
        _ => cancelled_permission_response(),
    }
}

fn cancelled_permission_response() -> RequestPermissionResponse {
    RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled)
}

fn acp_permission_request(request: RequestPermissionRequest) -> AcpPermissionRequest {
    let session_id = request.session_id.0.to_string();
    let tool_call_id = request.tool_call.tool_call_id.0.to_string();
    let tool_name = request
        .tool_call
        .fields
        .title
        .clone()
        .unwrap_or_else(|| "ACP tool".to_string());
    AcpPermissionRequest {
        request_id: format!("{session_id}:{tool_call_id}"),
        session_id,
        tool_call_id,
        summary: format!("ACP Agent 请求执行工具：{tool_name}"),
        tool_name,
        details: serde_json::to_value(&request.tool_call).unwrap_or_else(|_| serde_json::json!({})),
        options: request
            .options
            .into_iter()
            .map(|option| AcpPermissionOption {
                option_id: option.option_id.0.to_string(),
                name: option.name,
                kind: serde_json::to_value(option.kind)
                    .ok()
                    .and_then(|value| value.as_str().map(ToString::to_string))
                    .unwrap_or_else(|| "unknown".to_string()),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AcpPermissionFuture, AcpPermissionMessage, AcpPermissionOption, AcpPermissionOutcome,
        AcpPermissionRequest, acp_permission_channel, acp_permission_channel_with_timeout,
        acp_permission_request, resolve_acp_permission_request,
    };
    use agent_client_protocol::schema::{
        PermissionOption, PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
        ToolCallUpdate, ToolCallUpdateFields,
    };
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn preferred_allow_option_uses_allow_before_reject() {
        let request = AcpPermissionRequest {
            request_id: "s:call".to_string(),
            session_id: "s".to_string(),
            tool_call_id: "call".to_string(),
            tool_name: "tool".to_string(),
            summary: "summary".to_string(),
            details: serde_json::json!({}),
            options: vec![
                AcpPermissionOption {
                    option_id: "reject".to_string(),
                    name: "Reject".to_string(),
                    kind: "reject_once".to_string(),
                },
                AcpPermissionOption {
                    option_id: "allow".to_string(),
                    name: "Allow".to_string(),
                    kind: "allow_once".to_string(),
                },
            ],
        };

        assert_eq!(
            Some("allow".to_string()),
            request.preferred_allow_option_id()
        );
    }

    #[test]
    fn protocol_permission_request_exposes_stable_request_and_tool_call_ids() {
        let request = acp_permission_request(permission_request());

        assert_eq!("session:call", request.request_id);
        assert_eq!("session", request.session_id);
        assert_eq!("call", request.tool_call_id);
        assert_eq!(
            Some(&serde_json::json!({"path": "/tmp/file"})),
            request.raw_input()
        );
    }

    #[test]
    fn fallback_raw_input_only_fills_missing_protocol_arguments() {
        let mut request = acp_permission_request(permission_request());
        request
            .details
            .as_object_mut()
            .expect("tool call details")
            .remove("rawInput");

        request.use_fallback_raw_input(serde_json::json!({"command": "pwd"}));
        request.use_fallback_raw_input(serde_json::json!({"command": "ignored"}));

        assert_eq!(
            Some(&serde_json::json!({"command": "pwd"})),
            request.raw_input()
        );
    }

    #[tokio::test]
    async fn permission_channel_delivers_request_and_returns_selected_option() {
        let (provider, mut receiver) = acp_permission_channel();
        let request = acp_permission_request(permission_request());
        let outcome = tokio::spawn(provider(request.clone()));

        let message = receiver.recv().await.expect("permission request");
        let AcpPermissionMessage::Requested(envelope) = message else {
            panic!("expected permission request");
        };
        assert!(
            envelope
                .request()
                .request_id
                .ends_with(&format!(":{}", request.request_id))
        );
        assert!(envelope.resolve(AcpPermissionOutcome::Selected {
            option_id: "allow".to_string(),
        }));

        assert_eq!(
            AcpPermissionOutcome::Selected {
                option_id: "allow".to_string(),
            },
            outcome.await.expect("permission outcome")
        );
    }

    #[tokio::test]
    async fn separate_permission_channels_use_distinct_routing_ids() {
        let (first_provider, mut first_receiver) = acp_permission_channel();
        let (second_provider, mut second_receiver) = acp_permission_channel();
        let request = acp_permission_request(permission_request());
        let first_outcome = tokio::spawn(first_provider(request.clone()));
        let second_outcome = tokio::spawn(second_provider(request));

        let AcpPermissionMessage::Requested(first) =
            first_receiver.recv().await.expect("first request")
        else {
            panic!("expected first request");
        };
        let AcpPermissionMessage::Requested(second) =
            second_receiver.recv().await.expect("second request")
        else {
            panic!("expected second request");
        };
        assert_ne!(first.request().request_id, second.request().request_id);
        first.resolve(AcpPermissionOutcome::Cancelled);
        second.resolve(AcpPermissionOutcome::Cancelled);
        assert_eq!(
            AcpPermissionOutcome::Cancelled,
            first_outcome.await.expect("first outcome")
        );
        assert_eq!(
            AcpPermissionOutcome::Cancelled,
            second_outcome.await.expect("second outcome")
        );
    }

    #[tokio::test]
    async fn permission_timeout_notifies_view_and_returns_cancelled() {
        let (provider, mut receiver) = acp_permission_channel_with_timeout(Duration::ZERO);
        let outcome = tokio::spawn(provider(acp_permission_request(permission_request())));

        let AcpPermissionMessage::Requested(envelope) =
            receiver.recv().await.expect("permission request")
        else {
            panic!("expected permission request");
        };
        let request_id = envelope.request().request_id.clone();
        let AcpPermissionMessage::Expired {
            request_id: expired_id,
        } = receiver.recv().await.expect("expiration notification")
        else {
            panic!("expected expiration notification");
        };

        assert_eq!(request_id, expired_id);
        assert!(!envelope.resolve(AcpPermissionOutcome::Cancelled));
        assert_eq!(
            AcpPermissionOutcome::Cancelled,
            outcome.await.expect("timeout outcome")
        );
    }

    #[tokio::test]
    async fn permission_request_is_cancelled_without_provider() {
        let response = resolve_acp_permission_request(None, permission_request()).await;

        assert!(matches!(
            response.outcome,
            RequestPermissionOutcome::Cancelled
        ));
    }

    #[tokio::test]
    async fn permission_request_uses_provider_selected_option() {
        let provider = Arc::new(|request: AcpPermissionRequest| {
            Box::pin(async move {
                AcpPermissionOutcome::Selected {
                    option_id: request.preferred_allow_option_id().unwrap(),
                }
            }) as AcpPermissionFuture
        });

        let response = resolve_acp_permission_request(Some(provider), permission_request()).await;

        assert!(matches!(
            response.outcome,
            RequestPermissionOutcome::Selected(selected) if selected.option_id.0.as_ref() == "allow"
        ));
    }

    fn permission_request() -> RequestPermissionRequest {
        let mut fields = ToolCallUpdateFields::default();
        fields.title = Some("Write file".to_string());
        fields.raw_input = Some(serde_json::json!({"path": "/tmp/file"}));
        RequestPermissionRequest::new(
            "session",
            ToolCallUpdate::new("call", fields),
            vec![
                PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
                PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
            ],
        )
    }
}
