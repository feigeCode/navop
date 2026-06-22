mod internal;
mod redis;
mod registry;
mod terminal;
mod tool_runtime_adapter;

pub use internal::{
    InternalFunctionDefinition, InternalFunctionFuture, InternalFunctionToolProvider,
};
pub use redis::{RedisConnectionSnapshot, RedisConnectionSnapshotProvider, RedisToolProvider};
pub use registry::{PublicMcpToolRegistry, PublicMcpToolRegistryError};
pub use terminal::TerminalToolProvider;
pub use tool_runtime_adapter::ToolRuntimeMcpProvider;

use crate::approval::{PublicMcpApprovalManager, PublicMcpApprovalOutcome};
use crate::permissions::{PermissionMode, PublicMcpOperationKind};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject, Tool},
};
use serde_json::Value;
use std::{future::Future, pin::Pin};

pub type PublicMcpToolFuture =
    Pin<Box<dyn Future<Output = Result<CallToolResult, McpError>> + Send + 'static>>;

#[derive(Clone)]
pub struct PublicMcpToolContext {
    pub permission_mode: PermissionMode,
    pub approver: PublicMcpApprovalManager,
}

impl PublicMcpToolContext {
    pub async fn request_approval(
        &self,
        operation: PublicMcpOperationKind,
        tool_name: impl Into<String>,
        summary: impl Into<String>,
        details: Value,
    ) -> PublicMcpApprovalOutcome {
        self.approver
            .request(operation, tool_name, summary, details)
            .await
    }
}

pub trait PublicMcpToolProvider: Send + Sync + 'static {
    fn tools(&self) -> Vec<Tool>;

    fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Option<PublicMcpToolFuture>;
}
