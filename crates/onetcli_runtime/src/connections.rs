#[cfg(test)]
mod tests;

mod build;
mod extended_build;
mod input;
mod management;
mod schema;
mod validation;

use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, StoredConnection, WorkspaceRepository};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tool_runtime::{
    ResourceCapability, RiskLevel, ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor,
    ToolError, ToolFuture, ToolHandler, ToolMode, ToolRegistry, ToolResult, ToolTargetSpec,
};

#[derive(Clone, Copy)]
enum ConnectionTool {
    List,
    Show,
    ListKinds,
    GetSchema,
    Validate,
    Save,
    Find,
    Delete,
    Test,
    OpenSession,
}

pub trait ConnectionSessionOpener: Send + Sync + 'static {
    fn open_session(&self, connection: StoredConnection) -> ToolFuture;
}

#[derive(Debug, Clone)]
pub enum ConnectionSaveEvent {
    Created(StoredConnection),
    Updated(StoredConnection),
}

pub type ConnectionSaveNotifyFuture = Pin<Box<dyn Future<Output = Result<(), ToolError>> + Send>>;

pub trait ConnectionSaveNotifier: Send + Sync + 'static {
    fn notify_save(&self, event: ConnectionSaveEvent) -> ConnectionSaveNotifyFuture;
}

#[derive(Clone, Default)]
pub struct ConnectionToolHooks {
    session_opener: Option<Arc<dyn ConnectionSessionOpener>>,
    save_notifier: Option<Arc<dyn ConnectionSaveNotifier>>,
}

impl ConnectionToolHooks {
    pub fn with_session_opener(
        mut self,
        session_opener: Option<Arc<dyn ConnectionSessionOpener>>,
    ) -> Self {
        self.session_opener = session_opener;
        self
    }

    pub fn with_save_notifier(
        mut self,
        save_notifier: Option<Arc<dyn ConnectionSaveNotifier>>,
    ) -> Self {
        self.save_notifier = save_notifier;
        self
    }
}

#[derive(Clone)]
struct ConnectionToolHandler {
    repo: Arc<ConnectionRepository>,
    workspaces: Option<Arc<WorkspaceRepository>>,
    session_opener: Option<Arc<dyn ConnectionSessionOpener>>,
    save_notifier: Option<Arc<dyn ConnectionSaveNotifier>>,
    tool: ConnectionTool,
}

pub fn connection_tool_registry(repo: Arc<ConnectionRepository>) -> ToolRegistry {
    connection_tool_registry_with_workspaces(repo, None)
}

pub fn connection_tool_registry_with_workspaces(
    repo: Arc<ConnectionRepository>,
    workspaces: Option<Arc<WorkspaceRepository>>,
) -> ToolRegistry {
    connection_tool_registry_with_workspaces_and_hooks(
        repo,
        workspaces,
        ConnectionToolHooks::default(),
    )
}

pub fn connection_tool_registry_with_workspaces_and_session_opener(
    repo: Arc<ConnectionRepository>,
    workspaces: Option<Arc<WorkspaceRepository>>,
    session_opener: Option<Arc<dyn ConnectionSessionOpener>>,
) -> ToolRegistry {
    connection_tool_registry_with_workspaces_and_hooks(
        repo,
        workspaces,
        ConnectionToolHooks::default().with_session_opener(session_opener),
    )
}

pub fn connection_tool_registry_with_workspaces_and_hooks(
    repo: Arc<ConnectionRepository>,
    workspaces: Option<Arc<WorkspaceRepository>>,
    hooks: ConnectionToolHooks,
) -> ToolRegistry {
    ToolRegistry::new(vec![
        Arc::new(
            ConnectionToolHandler::new(repo.clone(), workspaces.clone(), ConnectionTool::List)
                .with_hooks(hooks.clone()),
        ),
        Arc::new(
            ConnectionToolHandler::new(repo.clone(), workspaces.clone(), ConnectionTool::Show)
                .with_hooks(hooks.clone()),
        ),
        Arc::new(
            ConnectionToolHandler::new(repo.clone(), workspaces.clone(), ConnectionTool::ListKinds)
                .with_hooks(hooks.clone()),
        ),
        Arc::new(
            ConnectionToolHandler::new(repo.clone(), workspaces.clone(), ConnectionTool::GetSchema)
                .with_hooks(hooks.clone()),
        ),
        Arc::new(
            ConnectionToolHandler::new(repo.clone(), workspaces.clone(), ConnectionTool::Validate)
                .with_hooks(hooks.clone()),
        ),
        Arc::new(
            ConnectionToolHandler::new(repo.clone(), workspaces.clone(), ConnectionTool::Save)
                .with_hooks(hooks.clone()),
        ),
        Arc::new(
            ConnectionToolHandler::new(repo.clone(), workspaces.clone(), ConnectionTool::Find)
                .with_hooks(hooks.clone()),
        ),
        Arc::new(
            ConnectionToolHandler::new(repo.clone(), workspaces.clone(), ConnectionTool::Delete)
                .with_hooks(hooks.clone()),
        ),
        Arc::new(
            ConnectionToolHandler::new(repo.clone(), workspaces.clone(), ConnectionTool::Test)
                .with_hooks(hooks.clone()),
        ),
        Arc::new(
            ConnectionToolHandler::new(repo, workspaces, ConnectionTool::OpenSession)
                .with_hooks(hooks),
        ),
    ])
}

impl ConnectionToolHandler {
    fn new(
        repo: Arc<ConnectionRepository>,
        workspaces: Option<Arc<WorkspaceRepository>>,
        tool: ConnectionTool,
    ) -> Self {
        Self {
            repo,
            workspaces,
            session_opener: None,
            save_notifier: None,
            tool,
        }
    }

    fn with_hooks(mut self, hooks: ConnectionToolHooks) -> Self {
        self.session_opener = hooks.session_opener;
        self.save_notifier = hooks.save_notifier;
        self
    }

    async fn call_tool(&self, input: Value, context: ToolContext) -> Result<ToolResult, ToolError> {
        match self.tool {
            ConnectionTool::List => {
                management::list_saved(&self.repo, self.workspaces.as_ref(), input)
            }
            ConnectionTool::Show => management::show(&self.repo, self.workspaces.as_ref(), input),
            ConnectionTool::ListKinds => Ok(ToolResult::structured(schema::list_kinds())),
            ConnectionTool::GetSchema => Ok(ToolResult::structured(schema::schema_for(input)?)),
            ConnectionTool::Validate => Ok(ToolResult::structured(validation::validate(input))),
            ConnectionTool::Save => self.save(input).await,
            ConnectionTool::Find => management::find(&self.repo, self.workspaces.as_ref(), input),
            ConnectionTool::Delete => {
                management::delete(&self.repo, self.workspaces.as_ref(), input)
            }
            ConnectionTool::Test => {
                management::test_connection(&self.repo, self.workspaces.as_ref(), input).await
            }
            ConnectionTool::OpenSession => self.open_session(input, context).await,
        }
    }

    async fn save(&self, input: Value) -> Result<ToolResult, ToolError> {
        if input.get("id").is_some() {
            let id = input::optional_i64(&input, "id").ok_or_else(|| ToolError::Failed {
                message: "missing integer field: id".to_string(),
            })?;
            let result = management::update(&self.repo, self.workspaces.as_ref(), input)?;
            let connection = management::find_unique_connection(&self.repo, &id.to_string())?;
            self.notify_save(ConnectionSaveEvent::Updated(connection))
                .await?;
            return Ok(result);
        }
        self.create_and_notify(input).await
    }

    async fn create_and_notify(&self, input: Value) -> Result<ToolResult, ToolError> {
        let validation = validation::validate(input.clone());
        if !validation["can_apply"].as_bool().unwrap_or(false) {
            return Ok(ToolResult::structured(validation));
        }
        let mut connection = build::build_connection(&input)?;
        self.repo
            .insert(&mut connection)
            .map_err(input::tool_error)?;
        self.notify_save(ConnectionSaveEvent::Created(connection.clone()))
            .await?;
        Ok(ToolResult::structured(json!({
            "ok": true,
            "connection": management::summarize(&connection, self.workspaces.as_ref(), true)?
        })))
    }

    async fn notify_save(&self, event: ConnectionSaveEvent) -> Result<(), ToolError> {
        if let Some(notifier) = &self.save_notifier {
            notifier.notify_save(event).await?;
        }
        Ok(())
    }

    async fn open_session(
        &self,
        input: Value,
        context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let reference = input::required_str(&input, "connection")?;
        let connection = management::find_unique_connection(&self.repo, reference)?;
        let summary = management::summarize(&connection, self.workspaces.as_ref(), true)?;
        let adapter = adapter_name(context.adapter);

        let Some(opener) = self.session_opener.clone() else {
            return Ok(ToolResult::structured(json!({
                "ok": true,
                "opened": false,
                "adapter": adapter,
                "connection": summary,
                "message": "No UI session opener is available in this runtime; the connection was resolved only."
            })));
        };

        let opened = opener.open_session(connection).await?;
        Ok(ToolResult::structured(json!({
            "ok": true,
            "opened": true,
            "adapter": adapter,
            "connection": summary,
            "session": opened.structured_content
        })))
    }
}

impl ToolHandler for ConnectionToolHandler {
    fn descriptor(&self) -> ToolDescriptor {
        let (id, title, description, read_only) = match self.tool {
            ConnectionTool::List => (
                "connections.list",
                "List saved connections",
                "List saved OnetCli connection profiles with filters and pagination. Supports kind, database_type, workspace_id, name_contains, host, limit, cursor, and include_summary. By default it returns compact metadata without connection params; set include_summary=true when redacted fields are needed.",
                true,
            ),
            ConnectionTool::Show => (
                "connections.show",
                "Show saved connection",
                "Show one saved OnetCli connection profile by numeric id or exact name. If the name is duplicated, the call fails and asks for an id; use connections.find to list candidates. Returned details redact secrets.",
                true,
            ),
            ConnectionTool::ListKinds => (
                "connections.list_kinds",
                "List connection kinds",
                "List connection kinds that can be created through OnetCli, including kind ids such as ssh_sftp, database, redis, mongodb, serial, port_forwarding, rdp, and vnc. Use before connections.get_schema when you do not know the exact kind string.",
                true,
            ),
            ConnectionTool::GetSchema => (
                "connections.get_schema",
                "Get connection schema",
                "Return the required fields, optional fields, defaults, and enum values for creating a specific connection kind. Use this before connections.validate or connections.save so arguments match the selected kind.",
                true,
            ),
            ConnectionTool::Validate => (
                "connections.validate",
                "Validate connection",
                "Validate a proposed connection creation request without saving it. Use the same creation arguments as connections.save, including kind and values, to check missing fields and type errors before mutating saved connections.",
                true,
            ),
            ConnectionTool::Save => (
                "connections.save",
                "Save connection",
                "Create or update an OnetCli connection profile. Omit id and pass kind plus values to create; pass id plus patch to update top-level fields or connection params. Call connections.get_schema first for the selected kind, and call connections.validate first when unsure. Use top-level remark, not values.remark. Password-like values are redacted in responses but may still appear in MCP tool-call arguments/logs depending on the client.",
                false,
            ),
            ConnectionTool::Find => (
                "connections.find",
                "Find saved connections",
                "Find saved connections using filters such as exact name, name_contains, kind, database_type, workspace_id, and host. Returns an array and never chooses among duplicate names; automation should prefer ids from this result.",
                true,
            ),
            ConnectionTool::Delete => (
                "connections.delete",
                "Delete saved connection",
                "Delete a saved connection by numeric id. Use connections.find or connections.show first if the id is not known.",
                false,
            ),
            ConnectionTool::Test => (
                "connections.test",
                "Test saved connection",
                "Test whether a saved database connection can actually connect and ping. This is different from connections.validate, which only validates request shape before saving. Non-database connection kinds return a structured unsupported_kind result.",
                false,
            ),
            ConnectionTool::OpenSession => (
                "connections.open_session",
                "Open connection session",
                "Open a saved OnetCli connection in the running app UI by numeric id or exact name. Use this when active session lists are empty and automation needs OnetCli to open a connection first. In CLI-only runtimes this resolves the connection and reports opened=false because no UI opener is available.",
                false,
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
            adapters: vec![
                ToolAdapter::Mcp,
                ToolAdapter::FunctionCalling,
                ToolAdapter::Cli,
            ],
            annotations: match self.tool {
                ConnectionTool::Save => save_annotations(title),
                _ => annotations(title, read_only),
            },
        }
    }

    fn call(&self, input: Value, context: ToolContext) -> tool_runtime::ToolFuture {
        let handler = self.clone();
        Box::pin(async move { handler.call_tool(input, context).await })
    }

    fn target_spec(&self) -> ToolTargetSpec {
        match self.tool {
            ConnectionTool::Show | ConnectionTool::Test => {
                ToolTargetSpec::required_with_capabilities(
                    Vec::new(),
                    vec![ResourceCapability::ManageConnection],
                )
            }
            ConnectionTool::OpenSession => ToolTargetSpec::required_with_capabilities(
                Vec::new(),
                vec![ResourceCapability::OpenSession],
            ),
            _ => ToolTargetSpec::none(),
        }
    }
}

fn annotations(title: &str, read_only: bool) -> ToolAnnotations {
    if read_only {
        ToolAnnotations::read_only(title)
    } else {
        ToolAnnotations::mutating(title)
    }
}

fn save_annotations(title: &str) -> ToolAnnotations {
    ToolAnnotations {
        title: title.to_string(),
        read_only: false,
        destructive: false,
        idempotent: false,
        open_world: false,
        supports_parallel: false,
        risk: RiskLevel::Medium,
    }
}

fn input_schema(tool: ConnectionTool) -> Value {
    match tool {
        ConnectionTool::List | ConnectionTool::Find => list_schema(),
        ConnectionTool::ListKinds => json!({
            "type": "object",
            "properties": {}
        }),
        ConnectionTool::Show => json!({
            "type": "object",
            "properties": { "connection": connection_ref_schema() },
            "required": ["connection"]
        }),
        ConnectionTool::GetSchema => kind_schema(true),
        ConnectionTool::Validate => create_schema(),
        ConnectionTool::Save => save_schema(),
        ConnectionTool::Delete => id_schema(),
        ConnectionTool::Test | ConnectionTool::OpenSession => json!({
            "type": "object",
            "properties": { "connection": connection_ref_schema() },
            "required": ["connection"]
        }),
    }
}

fn adapter_name(adapter: ToolAdapter) -> &'static str {
    match adapter {
        ToolAdapter::Cli => "cli",
        ToolAdapter::FunctionCalling => "function_calling",
        ToolAdapter::Mcp => "mcp",
        ToolAdapter::Gui => "gui",
    }
}

fn list_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": kind_value_schema(),
            "database_type": database_type_schema(),
            "workspace_id": { "type": ["integer", "null"] },
            "name": { "type": "string", "description": "Exact saved connection name." },
            "name_contains": { "type": "string" },
            "host": { "type": "string" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
            "cursor": { "type": "integer", "minimum": 0 },
            "include_summary": { "type": "boolean" }
        }
    })
}

fn kind_schema(include_database_type: bool) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert("kind".to_string(), kind_value_schema());
    if include_database_type {
        properties.insert("database_type".to_string(), database_type_schema());
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": ["kind"]
    })
}

fn create_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": kind_value_schema(),
            "database_type": database_type_schema(),
            "values": {
                "type": "object",
                "description": "Connection fields for the selected kind. Get exact fields from connections.get_schema."
            },
            "workspace_id": {
                "type": ["integer", "null"],
                "description": "Optional workspace id to associate with the connection."
            },
            "remark": {
                "type": ["string", "null"],
                "description": "Optional human-readable note for the saved connection. Pass remark here at the top level; values.remark is not used."
            },
            "sync_enabled": {
                "type": "boolean",
                "description": "Whether this saved connection should participate in sync."
            },
            "team_id": {
                "type": ["string", "null"],
                "description": "Optional team id for team-scoped connections."
            }
        },
        "required": ["kind", "values"]
    })
}

fn save_schema() -> Value {
    let mut schema = create_schema();
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        properties.insert("id".to_string(), json!({ "type": "integer" }));
        properties.insert("patch".to_string(), update_patch_schema());
    }
    if let Some(object) = schema.as_object_mut() {
        object.remove("required");
        object.insert(
            "oneOf".to_string(),
            json!([
                { "required": ["kind", "values"] },
                { "required": ["id", "patch"] }
            ]),
        );
    }
    schema
}

fn update_patch_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "remark": { "type": ["string", "null"] },
            "workspace_id": { "type": ["integer", "null"] },
            "sync_enabled": { "type": "boolean" },
            "database_type": database_type_schema(),
            "values": { "type": "object" }
        }
    })
}

fn id_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "id": { "type": "integer" } },
        "required": ["id"]
    })
}

fn connection_ref_schema() -> Value {
    json!({
        "type": "string",
        "description": "Saved connection numeric id as a string, or the exact saved connection name."
    })
}

fn kind_value_schema() -> Value {
    json!({
        "type": "string",
        "description": "Connection kind id. Use connections.list_kinds for supported values.",
        "enum": ["database", "ssh_sftp", "redis", "mongodb", "serial", "port_forwarding", "rdp", "vnc"]
    })
}

fn database_type_schema() -> Value {
    json!({
        "type": ["string", "null"],
        "description": "Database engine when kind is database, for example MySQL, PostgreSQL, SQLite, DuckDB, MSSQL, Oracle, or ClickHouse."
    })
}
