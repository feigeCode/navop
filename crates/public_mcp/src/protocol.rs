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
use std::{
    future,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

const SERVER_NAME: &str = "navop-public-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct PublicMcpServer {
    tool_registry: PublicMcpToolRegistry,
    permission_mode: SharedPermissionMode,
    approval_manager: PublicMcpApprovalManager,
}

#[derive(Clone)]
pub struct SharedPermissionMode {
    mode: Arc<AtomicU8>,
}

impl SharedPermissionMode {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode: Arc::new(AtomicU8::new(permission_mode_to_u8(mode))),
        }
    }

    pub fn get(&self) -> PermissionMode {
        permission_mode_from_u8(self.mode.load(Ordering::SeqCst))
    }

    pub fn set(&self, mode: PermissionMode) {
        self.mode
            .store(permission_mode_to_u8(mode), Ordering::SeqCst);
    }
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
        Self::with_shared_permission_and_approval(
            tool_registry,
            SharedPermissionMode::new(permission_mode),
            approval_manager,
        )
    }

    pub fn with_shared_permission(
        tool_registry: PublicMcpToolRegistry,
        permission_mode: SharedPermissionMode,
    ) -> Self {
        Self::with_shared_permission_and_approval(
            tool_registry,
            permission_mode,
            PublicMcpApprovalManager::default(),
        )
    }

    pub fn with_shared_permission_and_approval(
        tool_registry: PublicMcpToolRegistry,
        permission_mode: SharedPermissionMode,
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
        let permission_mode = self.permission_mode.get();
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(SERVER_NAME, SERVER_VERSION).with_title("Navop Public MCP"),
            )
            .with_instructions(server_instructions(permission_mode))
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
            permission_mode: self.permission_mode.get(),
            approver: self.approval_manager.clone(),
        };
        let name = request.name.to_string();
        let arguments = request.arguments;
        async move { tool_registry.call_tool(&name, arguments, context).await }
    }
}

fn server_instructions(mode: PermissionMode) -> String {
    let (id, approval) = match mode {
        PermissionMode::Deny => (
            "deny",
            "mutating tools are denied; enable confirmation or automatic mode in Navop when the user wants writes",
        ),
        PermissionMode::Ask => (
            "ask",
            "mutating tools require approval in Navop; preserve rejections and never bypass them",
        ),
        PermissionMode::Allow => (
            "allow",
            "mutating tools run automatically; confirm user intent before destructive operations",
        ),
    };
    format!(
        "Navop Public MCP permission_mode={id}: {approval}. Available tools also depend on Navop Tool Exposure settings and active connection sessions. If a tool is missing, ask the user to enable its tool group and open the relevant connection/session."
    )
}

fn permission_mode_to_u8(mode: PermissionMode) -> u8 {
    match mode {
        PermissionMode::Deny => 0,
        PermissionMode::Ask => 1,
        PermissionMode::Allow => 2,
    }
}

fn permission_mode_from_u8(value: u8) -> PermissionMode {
    match value {
        1 => PermissionMode::Ask,
        2 => PermissionMode::Allow,
        _ => PermissionMode::Deny,
    }
}
