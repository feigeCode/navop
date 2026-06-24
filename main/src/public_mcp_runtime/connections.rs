#[cfg(test)]
mod tests;

mod build;
mod extended_build;
mod input;
mod schema;
mod validation;

use one_core::storage::ConnectionRepository;
use one_core::storage::traits::Repository;
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolHandler, ToolMode,
    ToolRegistry, ToolResult,
};

#[derive(Clone, Copy)]
enum ConnectionTool {
    ListKinds,
    GetSchema,
    Validate,
    Create,
}

#[derive(Clone)]
struct ConnectionToolHandler {
    repo: Arc<ConnectionRepository>,
    tool: ConnectionTool,
}

pub(super) fn connection_tool_registry(repo: Arc<ConnectionRepository>) -> ToolRegistry {
    ToolRegistry::new(vec![
        Arc::new(ConnectionToolHandler::new(
            repo.clone(),
            ConnectionTool::ListKinds,
        )),
        Arc::new(ConnectionToolHandler::new(
            repo.clone(),
            ConnectionTool::GetSchema,
        )),
        Arc::new(ConnectionToolHandler::new(
            repo.clone(),
            ConnectionTool::Validate,
        )),
        Arc::new(ConnectionToolHandler::new(repo, ConnectionTool::Create)),
    ])
}

impl ConnectionToolHandler {
    fn new(repo: Arc<ConnectionRepository>, tool: ConnectionTool) -> Self {
        Self { repo, tool }
    }

    fn call_tool(&self, input: Value) -> Result<ToolResult, ToolError> {
        match self.tool {
            ConnectionTool::ListKinds => Ok(ToolResult::structured(schema::list_kinds())),
            ConnectionTool::GetSchema => Ok(ToolResult::structured(schema::schema_for(input)?)),
            ConnectionTool::Validate => Ok(ToolResult::structured(validation::validate(input))),
            ConnectionTool::Create => self.create(input),
        }
    }

    fn create(&self, input: Value) -> Result<ToolResult, ToolError> {
        let validation = validation::validate(input.clone());
        if !validation["can_apply"].as_bool().unwrap_or(false) {
            return Ok(ToolResult::structured(validation));
        }
        let mut connection = build::build_connection(&input)?;
        self.repo
            .insert(&mut connection)
            .map_err(input::tool_error)?;
        Ok(ToolResult::structured(json!({
            "ok": true,
            "connection": build::connection_summary(&connection)?
        })))
    }
}

impl ToolHandler for ConnectionToolHandler {
    fn descriptor(&self) -> ToolDescriptor {
        let (id, title, description, read_only) = match self.tool {
            ConnectionTool::ListKinds => (
                "public_mcp.connections.list_kinds",
                "List connection kinds",
                "List connection kinds that can be created through OnetCli.",
                true,
            ),
            ConnectionTool::GetSchema => (
                "public_mcp.connections.get_schema",
                "Get connection schema",
                "Return field schema and defaults for a connection kind.",
                true,
            ),
            ConnectionTool::Validate => (
                "public_mcp.connections.validate",
                "Validate connection",
                "Validate a connection creation request without writing it.",
                true,
            ),
            ConnectionTool::Create => (
                "public_mcp.connections.create",
                "Create connection",
                "Create a saved OnetCli connection from structured fields.",
                false,
            ),
        };
        ToolDescriptor {
            id: id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: annotations(title, read_only),
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let handler = self.clone();
        Box::pin(async move { handler.call_tool(input) })
    }
}

fn annotations(title: &str, read_only: bool) -> ToolAnnotations {
    if read_only {
        ToolAnnotations::read_only(title)
    } else {
        ToolAnnotations::mutating(title)
    }
}
