use anyhow::Result;
use gpui::App;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, DatabaseType, GlobalStorageState, StoredConnection,
};
use public_mcp::registry::PublicMcpRegistry;
use public_mcp::tools::ResourcePoolProvider;
use serde_json::Value;
use std::sync::Arc;
use tool_runtime::{ResourceCapability, ResourceKind, ResourceOrigin, ResourcePool, ResourceRef};

pub(super) fn app_resource_pool_provider(cx: &App) -> Option<ResourcePoolProvider> {
    let repo = connection_repository(cx);
    let terminal_registry = terminal_view::public_mcp::registry(cx);
    if repo.is_none() && terminal_registry.is_none() {
        return None;
    }
    Some(Arc::new(move || {
        match resource_pool_from_sources(repo.as_ref(), terminal_registry.as_ref()) {
            Ok(pool) => Some(pool),
            Err(error) => {
                tracing::warn!(error = %error, "Failed to build Public MCP resource pool");
                Some(ResourcePool::new())
            }
        }
    }))
}

fn connection_repository(cx: &App) -> Option<Arc<ConnectionRepository>> {
    cx.try_global::<GlobalStorageState>()?
        .storage
        .get::<ConnectionRepository>()
}

fn resource_pool_from_sources(
    repo: Option<&Arc<ConnectionRepository>>,
    terminal_registry: Option<&PublicMcpRegistry>,
) -> Result<ResourcePool> {
    let mut pool = match repo {
        Some(repo) => saved_connection_resource_pool(repo)?,
        None => ResourcePool::new(),
    };
    if let Some(registry) = terminal_registry {
        pool = registry
            .list_sessions()
            .into_iter()
            .map(terminal_session_resource)
            .fold(pool, ResourcePool::with_resource);
    }
    Ok(pool)
}

pub(super) fn saved_connection_resource_pool(repo: &ConnectionRepository) -> Result<ResourcePool> {
    let connections = repo.list()?;
    Ok(connections
        .into_iter()
        .filter_map(|connection| connection_resource(connection))
        .fold(ResourcePool::new(), ResourcePool::with_resource))
}

fn connection_resource(connection: StoredConnection) -> Option<ResourceRef> {
    let id = connection.id?.to_string();
    let label = if connection.name.is_empty() {
        format!("connection {id}")
    } else {
        connection.name.clone()
    };
    let mut resource = ResourceRef::new(id, connection_kind(&connection), label);
    for alias in connection_aliases(&connection) {
        resource = resource.with_alias(alias);
    }
    Some(resource)
}

fn terminal_session_resource(session: public_mcp::registry::PublicMcpSessionInfo) -> ResourceRef {
    let label = if session.host_label.is_empty() {
        session.title.clone()
    } else {
        session.host_label.clone()
    };
    let mut resource = ResourceRef::new(session.session_id.clone(), ResourceKind::Terminal, label)
        .with_alias(session.session_id)
        .with_capability(ResourceCapability::ExecCommand);
    resource.origin = ResourceOrigin::ActiveSession;
    if let Some(connection_id) = session.connection_id {
        resource = resource.with_alias(connection_id.to_string());
    }
    for alias in [session.title, session.host_label] {
        if !alias.is_empty() {
            resource = resource.with_alias(alias);
        }
    }
    resource
}

fn connection_kind(connection: &StoredConnection) -> ResourceKind {
    match connection.connection_type {
        ConnectionType::Database => database_kind(connection),
        ConnectionType::SshSftp => ResourceKind::Ssh,
        ConnectionType::Redis => ResourceKind::Redis,
        ConnectionType::MongoDB => ResourceKind::Mongo,
        ConnectionType::Serial => ResourceKind::Terminal,
        ConnectionType::PortForwarding => ResourceKind::Other("port-forwarding".into()),
        ConnectionType::Rdp => ResourceKind::Other("rdp".into()),
        ConnectionType::Vnc => ResourceKind::Other("vnc".into()),
        ConnectionType::All => ResourceKind::Other("all".into()),
    }
}

fn database_kind(connection: &StoredConnection) -> ResourceKind {
    match connection
        .to_db_connection()
        .map(|config| config.database_type)
    {
        Ok(DatabaseType::MySQL) => ResourceKind::Mysql,
        Ok(DatabaseType::PostgreSQL) => ResourceKind::Postgres,
        Ok(DatabaseType::SQLite) => ResourceKind::Sqlite,
        Ok(DatabaseType::External { driver_id }) => ResourceKind::Other(driver_id),
        Ok(other) => ResourceKind::Other(format!("{other:?}").to_ascii_lowercase()),
        Err(_) => ResourceKind::Other("database".into()),
    }
}

fn connection_aliases(connection: &StoredConnection) -> Vec<String> {
    let mut aliases = Vec::new();
    if let Some(cloud_id) = connection
        .cloud_id
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        aliases.push(cloud_id.clone());
    }
    aliases.extend(params_aliases(&connection.params));
    aliases
}

fn params_aliases(params: &str) -> Vec<String> {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(params) else {
        return Vec::new();
    };
    ["host", "hostname", "path"]
        .into_iter()
        .filter_map(|key| string_field(&map, key))
        .collect()
}

fn string_field(map: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    let value = match map.get(key)? {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => return None,
    };
    (!value.is_empty()).then_some(value)
}
