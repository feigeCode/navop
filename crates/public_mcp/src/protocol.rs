use crate::approval::PublicMcpApprovalManager;
use crate::permissions::PermissionMode;
use crate::registry::PublicMcpRegistry;
use crate::tools::{PublicMcpToolContext, PublicMcpToolRegistry};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::{MaybeSendFuture, RequestContext},
};
use std::{future, future::Future};

const SERVER_NAME: &str = "onetcli-public-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct PublicMcpServer {
    tool_registry: PublicMcpToolRegistry,
    permission_mode: PermissionMode,
    approval_manager: PublicMcpApprovalManager,
}

impl PublicMcpServer {
    pub fn new(registry: PublicMcpRegistry, permission_mode: PermissionMode) -> Self {
        Self::with_tool_registry(PublicMcpToolRegistry::terminal(registry), permission_mode)
    }

    pub fn with_tool_registry(
        tool_registry: PublicMcpToolRegistry,
        permission_mode: PermissionMode,
    ) -> Self {
        Self::with_tool_registry_and_approval(
            tool_registry,
            permission_mode,
            PublicMcpApprovalManager::default(),
        )
    }

    pub fn with_tool_registry_and_approval(
        tool_registry: PublicMcpToolRegistry,
        permission_mode: PermissionMode,
        approval_manager: PublicMcpApprovalManager,
    ) -> Self {
        Self {
            tool_registry,
            permission_mode,
            approval_manager,
        }
    }
}

impl ServerHandler for PublicMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(SERVER_NAME, SERVER_VERSION).with_title("OnetCli Public MCP"),
            )
            .with_instructions(
                "Expose only currently connected OnetCli SSH terminal sessions.".to_string(),
            )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + MaybeSendFuture + '_ {
        let tools = self.tool_registry.tools();
        future::ready(Ok(ListToolsResult {
            tools,
            next_cursor: None,
            meta: None,
        }))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_registry.get_tool(name)
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + MaybeSendFuture + '_ {
        let tool_registry = self.tool_registry.clone();
        let context = PublicMcpToolContext {
            permission_mode: self.permission_mode,
            approver: self.approval_manager.clone(),
        };
        let name = request.name.to_string();
        let arguments = request.arguments;
        async move { tool_registry.call_tool(&name, arguments, context).await }
    }
}
