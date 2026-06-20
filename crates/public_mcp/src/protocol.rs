use crate::permissions::{
    ApprovalDecision, PermissionMode, PublicMcpOperationKind, decide_permission,
};
use crate::registry::PublicMcpRegistry;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, Implementation, JsonObject, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
    },
    service::{MaybeSendFuture, RequestContext},
};
use serde_json::{Value, json};
use std::{borrow::Cow, future, future::Future, sync::Arc};

const SERVER_NAME: &str = "onetcli-public-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct PublicMcpServer {
    registry: PublicMcpRegistry,
    permission_mode: PermissionMode,
}

impl PublicMcpServer {
    pub fn new(registry: PublicMcpRegistry, permission_mode: PermissionMode) -> Self {
        Self {
            registry,
            permission_mode,
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
        future::ready(Ok(ListToolsResult {
            tools: tools(),
            next_cursor: None,
            meta: None,
        }))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|tool| tool.name == name)
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + MaybeSendFuture + '_ {
        let result = match request.name.as_ref() {
            "public_mcp.list_sessions" => Ok(CallToolResult::structured(json!({
                "sessions": self.registry.list_sessions()
            }))),
            "public_mcp.terminal_snapshot" => self.terminal_snapshot(request.arguments),
            "public_mcp.terminal_write" => self.terminal_write(request.arguments),
            name => Err(McpError::invalid_params(
                format!("unknown public MCP tool: {name}"),
                None,
            )),
        };
        future::ready(result)
    }
}

impl PublicMcpServer {
    fn terminal_snapshot(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, McpError> {
        let session_id = required_string(arguments.as_ref(), "session_id")?;
        let snapshot = self
            .registry
            .terminal_snapshot(session_id)
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(snapshot).map_err(internal_error)?,
        ))
    }

    fn terminal_write(&self, arguments: Option<JsonObject>) -> Result<CallToolResult, McpError> {
        match decide_permission(self.permission_mode, PublicMcpOperationKind::WriteTerminal) {
            ApprovalDecision::Allow => {}
            ApprovalDecision::Ask => {
                return Ok(CallToolResult::structured_error(json!({
                    "code": "approval_required",
                    "message": "terminal write requires approval"
                })));
            }
            ApprovalDecision::Deny => {
                return Ok(CallToolResult::structured_error(json!({
                    "code": "permission_denied",
                    "message": "terminal write denied by permission mode"
                })));
            }
        }

        let session_id = required_string(arguments.as_ref(), "session_id")?;
        let input = required_string(arguments.as_ref(), "input")?;
        self.registry
            .write_terminal(session_id, input)
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(json!({ "ok": true })))
    }
}

fn tools() -> Vec<Tool> {
    vec![
        Tool::new(
            "public_mcp.list_sessions",
            "List currently connected SSH terminal sessions exposed by OnetCli.",
            object_schema([]),
        )
        .with_annotations(read_only_annotations("List terminal sessions")),
        Tool::new(
            "public_mcp.terminal_snapshot",
            "Read the visible text of an exposed SSH terminal session.",
            object_schema([("session_id", string_schema())]),
        )
        .with_annotations(read_only_annotations("Read terminal snapshot")),
        Tool::new(
            "public_mcp.terminal_write",
            "Write raw input to an exposed SSH terminal session.",
            object_schema([("session_id", string_schema()), ("input", string_schema())]),
        )
        .with_annotations(
            ToolAnnotations::with_title("Write terminal input")
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        ),
    ]
}

fn read_only_annotations(title: &str) -> ToolAnnotations {
    ToolAnnotations::with_title(title)
        .read_only(true)
        .destructive(false)
        .idempotent(true)
        .open_world(false)
}

fn object_schema(properties: impl IntoIterator<Item = (&'static str, Value)>) -> Arc<JsonObject> {
    let required = properties
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect::<JsonObject>();
    let required_names = required.keys().cloned().map(Value::String).collect::<Vec<_>>();
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(required));
    if !required_names.is_empty() {
        schema.insert("required".to_string(), Value::Array(required_names));
    }
    Arc::new(schema)
}

fn string_schema() -> Value {
    json!({ "type": "string" })
}

fn required_string<'a>(
    arguments: Option<&'a JsonObject>,
    field: &'static str,
) -> Result<&'a str, McpError> {
    arguments
        .and_then(|args| args.get(field))
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params(format!("missing string argument: {field}"), None))
}

fn internal_error(error: impl std::fmt::Display) -> McpError {
    McpError::internal_error(Cow::Owned(error.to_string()), None)
}
