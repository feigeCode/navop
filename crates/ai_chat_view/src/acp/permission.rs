use agent_client_protocol::schema::{
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome,
};
use gpui::{App, Global};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type AcpPermissionFuture = Pin<Box<dyn Future<Output = AcpPermissionOutcome> + Send + 'static>>;
pub type AcpPermissionProvider =
    Arc<dyn Fn(AcpPermissionRequest) -> AcpPermissionFuture + Send + Sync + 'static>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpPermissionRequest {
    pub session_id: String,
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

struct GlobalAcpPermissionProvider {
    provider: AcpPermissionProvider,
}

impl Global for GlobalAcpPermissionProvider {}

pub fn set_acp_permission_provider(
    cx: &mut App,
    provider: impl Fn(AcpPermissionRequest) -> AcpPermissionFuture + Send + Sync + 'static,
) {
    cx.set_global(GlobalAcpPermissionProvider {
        provider: Arc::new(provider),
    });
}

pub(crate) fn acp_permission_provider(cx: &App) -> Option<AcpPermissionProvider> {
    cx.try_global::<GlobalAcpPermissionProvider>()
        .map(|global| global.provider.clone())
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
    let tool_name = request
        .tool_call
        .fields
        .title
        .clone()
        .unwrap_or_else(|| "ACP tool".to_string());
    AcpPermissionRequest {
        session_id: request.session_id.0.to_string(),
        summary: format!("ACP agent requests permission for {tool_name}"),
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
        AcpPermissionFuture, AcpPermissionOption, AcpPermissionOutcome, AcpPermissionRequest,
        resolve_acp_permission_request,
    };
    use agent_client_protocol::schema::{
        PermissionOption, PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
        ToolCallUpdate, ToolCallUpdateFields,
    };
    use std::sync::Arc;

    #[test]
    fn preferred_allow_option_uses_allow_before_reject() {
        let request = AcpPermissionRequest {
            session_id: "s".to_string(),
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
