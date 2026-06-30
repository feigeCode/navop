use agent_runtime::{ResourceContext, ResourceKind, ToolError, tools::ToolInvocation};
use db::DatabasePlugin;
use one_core::storage::DbConnectionConfig;
use std::sync::Arc;

pub(super) struct AgentDbTarget {
    pub(super) connection_id: String,
    pub(super) database: Option<String>,
    pub(super) schema: Option<String>,
}

pub(super) struct OpenedDb {
    pub(super) connection_id: String,
    pub(super) config: DbConnectionConfig,
    pub(super) plugin: Arc<dyn DatabasePlugin>,
    pub(super) connection: Box<dyn db::DbConnection + Send + Sync>,
}

impl OpenedDb {
    pub(super) fn config_schema(&self) -> Option<String> {
        self.config.extra_params.get("schema").cloned()
    }
}

pub(super) fn resolve_db_target(invocation: &ToolInvocation) -> Result<AgentDbTarget, ToolError> {
    let resource = invocation.target_resource();
    if let Some(resource) = resource
        && !is_database_kind(&resource.kind)
    {
        return Err(ToolError::MissingResource(format!(
            "current Agent resource is not a database connection: {}",
            resource.id
        )));
    }
    let connection_id = invocation
        .arg_str("connection")
        .map(ToString::to_string)
        .or_else(|| resource.map(|item| item.id.to_string()))
        .ok_or_else(|| {
            ToolError::MissingResource(
                "please select a database connection in the database sidebar first".into(),
            )
        })?;
    let database = invocation
        .arg_str("database")
        .map(ToString::to_string)
        .or_else(|| scope_value(resource, "database"));
    let schema = invocation
        .arg_str("schema")
        .map(ToString::to_string)
        .or_else(|| scope_value(resource, "schema"));
    Ok(AgentDbTarget {
        connection_id,
        database,
        schema,
    })
}

pub(super) fn current_database_context(resources: &ResourceContext) -> Option<AgentDbTarget> {
    let resource = resources.current()?;
    is_database_kind(&resource.kind).then(|| AgentDbTarget {
        connection_id: resource.id.to_string(),
        database: scope_value(Some(resource), "database"),
        schema: scope_value(Some(resource), "schema"),
    })
}

pub(super) fn require_database(opened: &OpenedDb) -> Result<String, ToolError> {
    opened.config.database.clone().ok_or_else(|| {
        ToolError::MissingResource(
            "database is required; select a database in the database sidebar or pass database"
                .into(),
        )
    })
}

fn scope_value(resource: Option<&agent_runtime::ResourceRef>, key: &str) -> Option<String> {
    resource.and_then(|resource| {
        resource
            .scopes
            .iter()
            .find(|scope| scope.key == key)
            .map(|scope| scope.value.clone())
    })
}

fn is_database_kind(kind: &ResourceKind) -> bool {
    match kind {
        ResourceKind::Mysql | ResourceKind::Postgres | ResourceKind::Sqlite => true,
        ResourceKind::Other(value) => matches!(
            value.as_str(),
            "database" | "clickhouse" | "mssql" | "oracle" | "duckdb"
        ),
        _ => false,
    }
}
