use anyhow::Result;
use gpui::App;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, DatabaseType, GlobalStorageState, StoredConnection,
};
use serde_json::Value;
use tool_runtime::{ResourceKind, ResourcePool, ResourceRef};

pub(super) fn app_resource_pool(cx: &App) -> Result<Option<ResourcePool>> {
    let Some(storage) = cx.try_global::<GlobalStorageState>() else {
        return Ok(None);
    };
    let Some(repo) = storage.storage.get::<ConnectionRepository>() else {
        return Ok(None);
    };
    saved_connection_resource_pool(&repo).map(Some)
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
