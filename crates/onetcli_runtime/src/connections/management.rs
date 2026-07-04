use super::{build, input};
use db::manager::DbManager;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, DatabaseType, DbConnectionConfig, StoredConnection,
    WorkspaceRepository,
};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tool_runtime::{ToolError, ToolResult};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;

pub(super) fn list_saved(
    repo: &ConnectionRepository,
    workspace_repo: Option<&Arc<WorkspaceRepository>>,
    input: Value,
) -> Result<ToolResult, ToolError> {
    let query = ConnectionQuery::from_input(&input, false);
    let listed = filtered_connections(repo, &query)?;
    let workspace_names = workspace_names(workspace_repo)?;
    Ok(ToolResult::structured(page_response(
        listed,
        &query,
        &workspace_names,
    )?))
}

pub(super) fn find(
    repo: &ConnectionRepository,
    workspace_repo: Option<&Arc<WorkspaceRepository>>,
    input: Value,
) -> Result<ToolResult, ToolError> {
    let query = ConnectionQuery::from_input(&input, false);
    let listed = filtered_connections(repo, &query)?;
    let workspace_names = workspace_names(workspace_repo)?;
    Ok(ToolResult::structured(page_response(
        listed,
        &query,
        &workspace_names,
    )?))
}

pub(super) fn show(
    repo: &ConnectionRepository,
    workspace_repo: Option<&Arc<WorkspaceRepository>>,
    input: Value,
) -> Result<ToolResult, ToolError> {
    let reference = input::required_str(&input, "connection")?;
    let connection = find_unique_connection(repo, reference)?;
    Ok(ToolResult::structured(json!({
        "connection": summarize(&connection, workspace_repo, true)?
    })))
}

pub(super) fn update(
    repo: &ConnectionRepository,
    workspace_repo: Option<&Arc<WorkspaceRepository>>,
    input: Value,
) -> Result<ToolResult, ToolError> {
    let id = required_i64(&input, "id")?;
    let patch = input::required_object(&input, "patch")?;
    let mut connection = load_connection(repo, id)?;
    apply_patch(&mut connection, patch, workspace_repo)?;
    repo.update(&connection).map_err(input::tool_error)?;
    Ok(ToolResult::structured(json!({
        "ok": true,
        "connection": summarize(&connection, workspace_repo, true)?
    })))
}

pub(super) fn delete(
    repo: &ConnectionRepository,
    workspace_repo: Option<&Arc<WorkspaceRepository>>,
    input: Value,
) -> Result<ToolResult, ToolError> {
    let id = required_i64(&input, "id")?;
    let connection = load_connection(repo, id)?;
    repo.delete(id).map_err(input::tool_error)?;
    Ok(ToolResult::structured(json!({
        "ok": true,
        "deleted": summarize(&connection, workspace_repo, false)?
    })))
}

pub(super) async fn test_connection(
    repo: &ConnectionRepository,
    workspace_repo: Option<&Arc<WorkspaceRepository>>,
    input: Value,
) -> Result<ToolResult, ToolError> {
    let reference = input::required_str(&input, "connection")?;
    let connection = find_unique_connection(repo, reference)?;
    if connection.connection_type != ConnectionType::Database {
        return Ok(ToolResult::structured(test_failure(
            &connection,
            workspace_repo,
            "unsupported_kind",
            "connections.test currently supports database connections only",
        )?));
    }
    test_database_connection(connection, workspace_repo).await
}

async fn test_database_connection(
    connection: StoredConnection,
    workspace_repo: Option<&Arc<WorkspaceRepository>>,
) -> Result<ToolResult, ToolError> {
    let config = connection.to_db_connection().map_err(input::tool_error)?;
    let database_type = config.database_type.storage_key();
    let plugin = DbManager::new()
        .get_plugin(&config.database_type)
        .map_err(input::tool_error)?;
    let result = plugin.test_connection(config).await;
    let connection_summary = summarize(&connection, workspace_repo, false)?;
    Ok(ToolResult::structured(match result {
        Ok(()) => json!({
            "ok": true,
            "kind": "database",
            "database_type": database_type,
            "connection": connection_summary
        }),
        Err(error) => json!({
            "ok": false,
            "code": classify_db_error(&error.to_string()),
            "message": error.to_string(),
            "kind": "database",
            "database_type": database_type,
            "connection": connection_summary
        }),
    }))
}

fn filtered_connections(
    repo: &ConnectionRepository,
    query: &ConnectionQuery,
) -> Result<Vec<StoredConnection>, ToolError> {
    Ok(repo
        .list()
        .map_err(input::tool_error)?
        .into_iter()
        .filter(|connection| query.matches(connection))
        .collect())
}

fn page_response(
    connections: Vec<StoredConnection>,
    query: &ConnectionQuery,
    workspace_names: &HashMap<i64, String>,
) -> Result<Value, ToolError> {
    let total = connections.len();
    let page = connections
        .into_iter()
        .skip(query.cursor)
        .take(query.limit)
        .map(|connection| {
            let workspace_name = connection
                .workspace_id
                .and_then(|id| workspace_names.get(&id).map(String::as_str));
            build::connection_summary_with_options(
                &connection,
                workspace_name,
                query.include_summary,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = (query.cursor + page.len() < total).then_some(query.cursor + page.len());
    Ok(json!({
        "connections": page,
        "total_matched": total,
        "cursor": query.cursor,
        "limit": query.limit,
        "next_cursor": next_cursor
    }))
}

pub(super) fn find_unique_connection(
    repo: &ConnectionRepository,
    reference: &str,
) -> Result<StoredConnection, ToolError> {
    if let Ok(id) = reference.parse::<i64>() {
        return load_connection(repo, id);
    }
    let matches = repo
        .list()
        .map_err(input::tool_error)?
        .into_iter()
        .filter(|stored| stored.name == reference)
        .collect::<Vec<_>>();
    match matches.len() {
        0 => Err(ToolError::Failed {
            message: format!("unknown connection: {reference}"),
        }),
        1 => Ok(matches.into_iter().next().expect("one match should exist")),
        _ => Err(ToolError::Failed {
            message: format!(
                "multiple connections named `{reference}`; use a numeric id or connections.find"
            ),
        }),
    }
}

fn apply_patch(
    connection: &mut StoredConnection,
    patch: &Value,
    workspace_repo: Option<&Arc<WorkspaceRepository>>,
) -> Result<(), ToolError> {
    if let Some(name) = patch.get("name").and_then(Value::as_str) {
        connection.name = name.to_string();
    }
    if patch.get("remark").is_some() {
        connection.remark = patch
            .get("remark")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if let Some(sync_enabled) = patch.get("sync_enabled").and_then(Value::as_bool) {
        connection.sync_enabled = sync_enabled;
    }
    if patch.get("workspace_id").is_some() {
        let workspace_id = optional_i64_or_null(patch, "workspace_id")?;
        ensure_workspace_exists(workspace_repo, workspace_id)?;
        connection.workspace_id = workspace_id;
    }
    apply_params_patch(connection, patch)?;
    Ok(())
}

fn apply_params_patch(connection: &mut StoredConnection, patch: &Value) -> Result<(), ToolError> {
    let mut params = params_object(connection)?;
    if let Some(database_type) = patch.get("database_type").and_then(Value::as_str) {
        if connection.connection_type != ConnectionType::Database {
            return Err(ToolError::Failed {
                message: "database_type can only be updated on database connections".to_string(),
            });
        }
        let database_type =
            DatabaseType::from_storage_key(database_type).ok_or_else(|| ToolError::Failed {
                message: format!("unknown database type: {database_type}"),
            })?;
        params.insert(
            "database_type".to_string(),
            json!(database_type.storage_key()),
        );
    }
    if let Some(values) = patch.get("values").and_then(Value::as_object) {
        merge_values(connection.connection_type, &mut params, values);
        if let Some(name) = values.get("name").and_then(Value::as_str) {
            connection.name = name.to_string();
        }
    }
    connection.params = serde_json::to_string(&Value::Object(params)).map_err(input::tool_error)?;
    Ok(())
}

fn merge_values(
    connection_type: ConnectionType,
    params: &mut Map<String, Value>,
    values: &Map<String, Value>,
) {
    for (key, value) in values {
        if key == "name" {
            continue;
        }
        if connection_type == ConnectionType::Database && !build::database_core_field(key) {
            let extra_params = params
                .entry("extra_params".to_string())
                .or_insert_with(|| json!({}));
            if !extra_params.is_object() {
                *extra_params = json!({});
            }
            if let Some(extra_params) = extra_params.as_object_mut() {
                extra_params.insert(key.clone(), json!(build::extra_param_string(value)));
            }
        } else {
            params.insert(key.clone(), value.clone());
        }
    }
}

pub(super) fn summarize(
    connection: &StoredConnection,
    workspace_repo: Option<&Arc<WorkspaceRepository>>,
    include_summary: bool,
) -> Result<Value, ToolError> {
    let workspace_name = workspace_name(workspace_repo, connection.workspace_id)?;
    build::connection_summary_with_options(connection, workspace_name.as_deref(), include_summary)
}

fn workspace_names(
    repo: Option<&Arc<WorkspaceRepository>>,
) -> Result<HashMap<i64, String>, ToolError> {
    let Some(repo) = repo else {
        return Ok(HashMap::new());
    };
    Ok(repo
        .list()
        .map_err(input::tool_error)?
        .into_iter()
        .filter_map(|workspace| workspace.id.map(|id| (id, workspace.name)))
        .collect())
}

fn workspace_name(
    repo: Option<&Arc<WorkspaceRepository>>,
    id: Option<i64>,
) -> Result<Option<String>, ToolError> {
    let (Some(repo), Some(id)) = (repo, id) else {
        return Ok(None);
    };
    Ok(repo
        .get(id)
        .map_err(input::tool_error)?
        .map(|item| item.name))
}

fn ensure_workspace_exists(
    repo: Option<&Arc<WorkspaceRepository>>,
    workspace_id: Option<i64>,
) -> Result<(), ToolError> {
    let (Some(repo), Some(workspace_id)) = (repo, workspace_id) else {
        return Ok(());
    };
    if repo.exists(workspace_id).map_err(input::tool_error)? {
        Ok(())
    } else {
        Err(ToolError::Failed {
            message: format!("unknown workspace: {workspace_id}"),
        })
    }
}

fn load_connection(repo: &ConnectionRepository, id: i64) -> Result<StoredConnection, ToolError> {
    repo.get(id)
        .map_err(input::tool_error)?
        .ok_or_else(|| ToolError::Failed {
            message: format!("unknown connection: {id}"),
        })
}

fn params_object(connection: &StoredConnection) -> Result<Map<String, Value>, ToolError> {
    serde_json::from_str::<Value>(&connection.params)
        .map_err(input::tool_error)?
        .as_object()
        .cloned()
        .ok_or_else(|| ToolError::Failed {
            message: "connection params must be a JSON object".to_string(),
        })
}

fn required_i64(input: &Value, field: &str) -> Result<i64, ToolError> {
    input
        .get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| ToolError::Failed {
            message: format!("missing integer field: {field}"),
        })
}

fn optional_i64_or_null(input: &Value, field: &str) -> Result<Option<i64>, ToolError> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_i64().map(Some).ok_or_else(|| ToolError::Failed {
            message: format!("field `{field}` must be an integer or null"),
        }),
    }
}

fn test_failure(
    connection: &StoredConnection,
    workspace_repo: Option<&Arc<WorkspaceRepository>>,
    code: &str,
    message: &str,
) -> Result<Value, ToolError> {
    Ok(json!({
        "ok": false,
        "code": code,
        "message": message,
        "kind": build::mcp_kind(connection.connection_type),
        "connection": summarize(connection, workspace_repo, false)?
    }))
}

fn classify_db_error(message: &str) -> &'static str {
    let message = message.to_ascii_lowercase();
    if message.contains("timeout") {
        "timeout"
    } else if message.contains("auth") || message.contains("login") || message.contains("password")
    {
        "authentication_failed"
    } else if message.contains("tls") || message.contains("ssl") || message.contains("cert") {
        "tls_error"
    } else {
        "connection_failed"
    }
}

#[derive(Default)]
struct ConnectionQuery {
    kind: Option<String>,
    database_type: Option<String>,
    workspace_id: Option<Option<i64>>,
    name: Option<String>,
    name_contains: Option<String>,
    host: Option<String>,
    limit: usize,
    cursor: usize,
    include_summary: bool,
}

impl ConnectionQuery {
    fn from_input(input: &Value, default_include_summary: bool) -> Self {
        Self {
            kind: str_field(input, "kind"),
            database_type: str_field(input, "database_type"),
            workspace_id: input.get("workspace_id").map(|value| value.as_i64()),
            name: str_field(input, "name"),
            name_contains: str_field(input, "name_contains").map(|value| value.to_lowercase()),
            host: str_field(input, "host"),
            limit: limit(input),
            cursor: input.get("cursor").and_then(Value::as_u64).unwrap_or(0) as usize,
            include_summary: input
                .get("include_summary")
                .and_then(Value::as_bool)
                .unwrap_or(default_include_summary),
        }
    }

    fn matches(&self, connection: &StoredConnection) -> bool {
        self.matches_kind(connection)
            && self.matches_workspace(connection)
            && self.matches_name(connection)
            && self.matches_params(connection)
    }

    fn matches_kind(&self, connection: &StoredConnection) -> bool {
        self.kind
            .as_deref()
            .is_none_or(|kind| build::mcp_kind(connection.connection_type) == kind)
    }

    fn matches_workspace(&self, connection: &StoredConnection) -> bool {
        self.workspace_id
            .as_ref()
            .is_none_or(|workspace_id| &connection.workspace_id == workspace_id)
    }

    fn matches_name(&self, connection: &StoredConnection) -> bool {
        let exact = self
            .name
            .as_deref()
            .is_none_or(|name| connection.name == name);
        let contains = self
            .name_contains
            .as_ref()
            .is_none_or(|needle| connection.name.to_lowercase().contains(needle.as_str()));
        exact && contains
    }

    fn matches_params(&self, connection: &StoredConnection) -> bool {
        let Ok(params) = serde_json::from_str::<Value>(&connection.params) else {
            return false;
        };
        self.matches_database_type(&params) && self.matches_host(&params)
    }

    fn matches_database_type(&self, params: &Value) -> bool {
        self.database_type.as_deref().is_none_or(|expected| {
            serde_json::from_value::<DbConnectionConfig>(params.clone())
                .map(|config| config.database_type.storage_key() == expected)
                .unwrap_or(false)
        })
    }

    fn matches_host(&self, params: &Value) -> bool {
        self.host
            .as_deref()
            .is_none_or(|host| params.get("host").and_then(Value::as_str) == Some(host))
    }
}

fn str_field(input: &Value, field: &str) -> Option<String> {
    input.get(field).and_then(Value::as_str).map(str::to_string)
}

fn limit(input: &Value) -> usize {
    input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|limit| limit as usize)
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_LIMIT)
}
