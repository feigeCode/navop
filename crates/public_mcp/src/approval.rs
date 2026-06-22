use crate::permissions::PublicMcpOperationKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{future::Future, pin::Pin, sync::Arc};

pub type PublicMcpApprovalFuture =
    Pin<Box<dyn Future<Output = PublicMcpApprovalOutcome> + Send + 'static>>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicMcpApprovalRequest {
    pub operation: PublicMcpOperationKind,
    pub tool_name: String,
    pub summary: String,
    pub details: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum PublicMcpApprovalOutcome {
    Approved,
    Denied { reason: Option<String> },
}

pub trait PublicMcpApprover: Send + Sync + 'static {
    fn request_approval(&self, request: PublicMcpApprovalRequest) -> PublicMcpApprovalFuture;
}

#[derive(Clone)]
pub struct PublicMcpApprovalManager {
    approver: Arc<dyn PublicMcpApprover>,
}

impl PublicMcpApprovalManager {
    pub fn new(approver: Arc<dyn PublicMcpApprover>) -> Self {
        Self { approver }
    }

    pub async fn request(
        &self,
        operation: PublicMcpOperationKind,
        tool_name: impl Into<String>,
        summary: impl Into<String>,
        details: Value,
    ) -> PublicMcpApprovalOutcome {
        self.approver
            .request_approval(PublicMcpApprovalRequest {
                operation,
                tool_name: tool_name.into(),
                summary: summary.into(),
                details,
            })
            .await
    }
}

impl Default for PublicMcpApprovalManager {
    fn default() -> Self {
        Self::new(Arc::new(DenyingApprover))
    }
}

struct DenyingApprover;

impl PublicMcpApprover for DenyingApprover {
    fn request_approval(&self, _request: PublicMcpApprovalRequest) -> PublicMcpApprovalFuture {
        Box::pin(async {
            PublicMcpApprovalOutcome::Denied {
                reason: Some("no public MCP approval handler is configured".to_string()),
            }
        })
    }
}
