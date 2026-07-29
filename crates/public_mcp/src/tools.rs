mod agent_runtime_adapter;
mod internal;
mod mongo;
mod redis;
mod registry;
mod remote_ops;
mod target_adapter;
mod terminal_control;
mod terminal_exec;
mod terminal_read;
mod tool_runtime_adapter;

pub use agent_runtime_adapter::agent_runtime_tool_registry;
pub use internal::{
    InternalFunctionDefinition, InternalFunctionFuture, internal_function_tool_registry,
};
pub use mongo::{
    MongoConnectionSnapshot, MongoConnectionSnapshotProvider, MongoOperation,
    MongoOperationProvider, MongoToolProvider,
};
pub use redis::{
    RedisCommandExecution, RedisCommandExecutionProvider, RedisConnectionSnapshot,
    RedisConnectionSnapshotProvider, RedisToolProvider,
};
pub use registry::{PublicMcpToolRegistry, PublicMcpToolRegistryError};
pub use remote_ops::remote_ops_tool_registry;
pub use terminal_control::terminal_control_tool_registry;
pub use terminal_exec::terminal_exec_tool_registry;
pub use terminal_read::terminal_read_tool_registry;
pub use tool_runtime_adapter::{ResourcePoolProvider, ToolRuntimeMcpProvider};

use crate::approval::{PublicMcpApprovalManager, PublicMcpApprovalOutcome};
use crate::permissions::{PermissionMode, PublicMcpOperationKind};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject, Tool},
};
use serde_json::Value;
use std::{future::Future, pin::Pin};
use tokio_util::sync::CancellationToken;

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

    fn call_tool_with_cancellation(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
        cancellation: CancellationToken,
    ) -> Option<PublicMcpToolFuture> {
        let future = self.call_tool(name, arguments, context)?;
        if cancellation.is_cancelled() {
            return Some(Box::pin(async { Err(tool_call_cancelled_error()) }));
        }
        Some(Box::pin(async move {
            // Dropping a provider future stops cooperative async work, but cannot undo
            // side effects that the provider has already detached or completed.
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => Err(tool_call_cancelled_error()),
                result = future => result,
            }
        }))
    }
}

fn tool_call_cancelled_error() -> McpError {
    McpError::internal_error("public MCP tool call cancelled", None)
}
