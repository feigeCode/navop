use one_core::storage::traits::Repository;
use one_core::storage::{Workspace, WorkspaceRepository};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolHandler, ToolMode,
    ToolRegistry, ToolResult,
};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;

#[derive(Clone, Copy)]
enum WorkspaceTool {
    List,
    Show,
}

#[derive(Clone)]
struct WorkspaceToolHandler {
    repo: Arc<WorkspaceRepository>,
    tool: WorkspaceTool,
}

pub fn workspace_tool_registry(repo: Arc<WorkspaceRepository>) -> ToolRegistry {
    ToolRegistry::new(vec![
        Arc::new(WorkspaceToolHandler::new(repo.clone(), WorkspaceTool::List)),
        Arc::new(WorkspaceToolHandler::new(repo, WorkspaceTool::Show)),
    ])
}

impl WorkspaceToolHandler {
    fn new(repo: Arc<WorkspaceRepository>, tool: WorkspaceTool) -> Self {
        Self { repo, tool }
    }

    fn call_tool(&self, input: Value) -> Result<ToolResult, ToolError> {
        match self.tool {
            WorkspaceTool::List => self.list(input),
            WorkspaceTool::Show => self.show(input),
        }
    }

    fn list(&self, input: Value) -> Result<ToolResult, ToolError> {
        let query = WorkspaceQuery::from_input(&input);
        let matched = self
            .repo
            .list()
            .map_err(tool_error)?
            .into_iter()
            .filter(|workspace| query.matches(workspace))
            .collect::<Vec<_>>();
        let total = matched.len();
        let page = matched
            .into_iter()
            .skip(query.cursor)
            .take(query.limit)
            .map(workspace_summary)
            .collect::<Vec<_>>();
        let next_cursor = (query.cursor + page.len() < total).then_some(query.cursor + page.len());
        Ok(ToolResult::structured(json!({
            "workspaces": page,
            "total_matched": total,
            "cursor": query.cursor,
            "limit": query.limit,
            "next_cursor": next_cursor
        })))
    }

    fn show(&self, input: Value) -> Result<ToolResult, ToolError> {
        let reference = required_str(&input, "workspace")?;
        let workspace = find_workspace(&self.repo, reference)?;
        Ok(ToolResult::structured(json!({
            "workspace": workspace_summary(workspace)
        })))
    }
}

impl ToolHandler for WorkspaceToolHandler {
    fn descriptor(&self) -> ToolDescriptor {
        let (id, title, description) = match self.tool {
            WorkspaceTool::List => (
                "workspaces.list",
                "List workspaces",
                "List saved Navop workspaces with ids and names. Supports name_contains, limit, and cursor so automation can resolve workspace ids before creating or moving connections.",
            ),
            WorkspaceTool::Show => (
                "workspaces.show",
                "Show workspace",
                "Show one saved Navop workspace by numeric id or exact name. If a name is duplicated, the call fails and asks for an id.",
            ),
        };
        ToolDescriptor {
            id: id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            input_schema: input_schema(self.tool),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations::read_only(title),
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let handler = self.clone();
        Box::pin(async move { handler.call_tool(input) })
    }
}

fn find_workspace(repo: &WorkspaceRepository, reference: &str) -> Result<Workspace, ToolError> {
    if let Ok(id) = reference.parse::<i64>() {
        return repo
            .get(id)
            .map_err(tool_error)?
            .ok_or_else(|| ToolError::Failed {
                message: format!("unknown workspace: {id}"),
            });
    }
    let matches = repo
        .list()
        .map_err(tool_error)?
        .into_iter()
        .filter(|workspace| workspace.name == reference)
        .collect::<Vec<_>>();
    match matches.len() {
        0 => Err(ToolError::Failed {
            message: format!("unknown workspace: {reference}"),
        }),
        1 => Ok(matches.into_iter().next().expect("one match should exist")),
        _ => Err(ToolError::Failed {
            message: format!("multiple workspaces named `{reference}`; use a numeric id"),
        }),
    }
}

fn workspace_summary(workspace: Workspace) -> Value {
    json!({
        "id": workspace.id,
        "name": workspace.name,
        "color": workspace.color,
        "icon": workspace.icon,
        "cloud_id": workspace.cloud_id,
        "created_at": workspace.created_at,
        "updated_at": workspace.updated_at
    })
}

fn input_schema(tool: WorkspaceTool) -> Value {
    match tool {
        WorkspaceTool::List => json!({
            "type": "object",
            "properties": {
                "name_contains": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                "cursor": { "type": "integer", "minimum": 0 }
            }
        }),
        WorkspaceTool::Show => json!({
            "type": "object",
            "properties": {
                "workspace": {
                    "type": "string",
                    "description": "Workspace numeric id as a string, or exact workspace name."
                }
            },
            "required": ["workspace"]
        }),
    }
}

fn required_str<'a>(input: &'a Value, field: &str) -> Result<&'a str, ToolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Failed {
            message: format!("missing string field: {field}"),
        })
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}

struct WorkspaceQuery {
    name_contains: Option<String>,
    limit: usize,
    cursor: usize,
}

impl WorkspaceQuery {
    fn from_input(input: &Value) -> Self {
        Self {
            name_contains: input
                .get("name_contains")
                .and_then(Value::as_str)
                .map(|value| value.to_lowercase()),
            limit: limit(input),
            cursor: input.get("cursor").and_then(Value::as_u64).unwrap_or(0) as usize,
        }
    }

    fn matches(&self, workspace: &Workspace) -> bool {
        self.name_contains
            .as_ref()
            .is_none_or(|needle| workspace.name.to_lowercase().contains(needle.as_str()))
    }
}

fn limit(input: &Value) -> usize {
    input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|limit| limit as usize)
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT)
}
