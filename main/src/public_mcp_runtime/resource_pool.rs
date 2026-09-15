use anyhow::Result;
use gpui::App;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, DatabaseType, GlobalStorageState, StoredConnection,
};
use public_mcp::registry::PublicMcpRegistry;
use public_mcp::tools::ResourcePoolProvider;
use redis_view::GlobalRedisState;
use serde_json::Value;
use std::sync::Arc;
use tool_runtime::{ResourceCapability, ResourceKind, ResourceOrigin, ResourcePool, ResourceRef};

pub(super) fn app_resource_pool_provider(cx: &App) -> Option<ResourcePoolProvider> {
    let repo = connection_repository(cx);
    let terminal_registry = terminal_view::public_mcp::registry(cx);
    let redis_state = cx.try_global::<GlobalRedisState>().cloned();
    if repo.is_none() && terminal_registry.is_none() && redis_state.is_none() {
        return None;
    }
    Some(Arc::new(move || {
        match resource_pool_from_sources(
            repo.as_ref(),
            terminal_registry.as_ref(),
            redis_state.as_ref(),
        ) {
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
    redis_state: Option<&GlobalRedisState>,
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
    if let Some(state) = redis_state {
        pool = append_active_redis_resources(pool, state.connection_ids());
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
    for capability in connection_capabilities(&connection) {
        resource = resource.with_capability(capability);
    }
    Some(resource)
}

fn connection_capabilities(connection: &StoredConnection) -> Vec<ResourceCapability> {
    let mut capabilities = vec![
        ResourceCapability::ManageConnection,
        ResourceCapability::OpenSession,
    ];
    capabilities.extend(match connection.connection_type {
        ConnectionType::Database => vec![
            ResourceCapability::DatabaseQuery,
            ResourceCapability::DatabaseExecute,
        ],
        ConnectionType::SshSftp => vec![
            ResourceCapability::List,
            ResourceCapability::ReadFile,
            ResourceCapability::WriteFile,
        ],
        ConnectionType::Redis => vec![ResourceCapability::Execute],
        ConnectionType::MongoDB => vec![ResourceCapability::Query, ResourceCapability::Execute],
        ConnectionType::Serial => Vec::new(),
        _ => Vec::new(),
    });
    capabilities
}

fn terminal_session_resource(session: public_mcp::registry::PublicMcpSessionInfo) -> ResourceRef {
    let label = if session.host_label.is_empty() {
        session.title.clone()
    } else {
        session.host_label.clone()
    };
    let mut resource = ResourceRef::new(session.session_id.clone(), ResourceKind::Terminal, label)
        .with_alias(session.session_id);
    resource.origin = ResourceOrigin::ActiveSession;
    for capability in session.capabilities {
        resource = resource.with_capability(capability);
    }
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

fn append_active_redis_resources(pool: ResourcePool, connection_ids: Vec<String>) -> ResourcePool {
    connection_ids
        .into_iter()
        .filter(|connection_id| !connection_id.is_empty())
        .map(active_redis_resource)
        .fold(pool, insert_or_merge_resource)
}

fn active_redis_resource(connection_id: String) -> ResourceRef {
    let mut resource = ResourceRef::new(
        connection_id.clone(),
        ResourceKind::Redis,
        format!("redis {connection_id}"),
    )
    .with_alias(connection_id)
    .with_capability(ResourceCapability::Execute);
    resource.origin = ResourceOrigin::ActiveSession;
    resource
}

fn insert_or_merge_resource(mut pool: ResourcePool, resource: ResourceRef) -> ResourcePool {
    let Some(existing) = pool
        .resources
        .iter_mut()
        .find(|item| item.id == resource.id && item.kind == resource.kind)
    else {
        return pool.with_resource(resource);
    };
    for alias in resource.aliases {
        if !existing.aliases.contains(&alias) {
            existing.aliases.push(alias);
        }
    }
    for capability in resource.capabilities {
        if !existing.capabilities.contains(&capability) {
            existing.capabilities.push(capability);
        }
    }
    pool
}

fn connection_kind(connection: &StoredConnection) -> ResourceKind {
    match connection.connection_type {
        ConnectionType::Database => database_kind(connection),
        ConnectionType::SshSftp => ResourceKind::Ssh,
        ConnectionType::Redis => ResourceKind::Redis,
        ConnectionType::MongoDB => ResourceKind::Mongo,
        ConnectionType::Mqtt => ResourceKind::Other("mqtt".into()),
        ConnectionType::Serial => ResourceKind::Terminal,
        ConnectionType::Telnet => ResourceKind::Terminal,
        ConnectionType::PortForwarding => ResourceKind::Other("port-forwarding".into()),
        ConnectionType::Rdp => ResourceKind::Other("rdp".into()),
        ConnectionType::Vnc => ResourceKind::Other("vnc".into()),
        ConnectionType::Extension => connection
            .to_extension_params()
            .map(|params| {
                ResourceKind::Other(format!(
                    "extension:{}:{}",
                    params.extension_id, params.contribution_id
                ))
            })
            .unwrap_or_else(|_| ResourceKind::Other("extension".into())),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_redis_resources_are_added_to_pool() {
        let pool = append_active_redis_resources(ResourcePool::new(), vec!["redis-a".to_string()]);

        let resource = pool.resolve_target("redis-a").unwrap();
        assert_eq!(ResourceKind::Redis, resource.kind);
        assert_eq!(ResourceOrigin::ActiveSession, resource.origin);
        assert!(resource.capabilities.contains(&ResourceCapability::Execute));
    }

    #[test]
    fn active_redis_resource_merges_with_saved_redis_resource() {
        let pool = ResourcePool::new().with_resource(
            ResourceRef::new("21", ResourceKind::Redis, "cache-prod").with_alias("10.2.4.54"),
        );

        let pool = append_active_redis_resources(pool, vec!["21".to_string()]);

        assert_eq!(1, pool.resources.len());
        let resource = pool.resolve_target("10.2.4.54").unwrap();
        assert_eq!("21", resource.id.as_str());
        assert!(resource.capabilities.contains(&ResourceCapability::Execute));
    }

    #[test]
    fn terminal_session_resource_preserves_registry_capabilities() {
        let resource = terminal_session_resource(public_mcp::registry::PublicMcpSessionInfo {
            session_id: "terminal-1".to_string(),
            connection_id: Some(21),
            title: "root@zn-54:~".to_string(),
            host_label: "prod-a".to_string(),
            cwd: Some("/root".to_string()),
            rows: 24,
            cols: 120,
            connection_kind: public_mcp::registry::TerminalConnectionKind::Ssh,
            connected: true,
            capabilities: vec![ResourceCapability::TerminalExec],
        });

        assert!(
            resource
                .capabilities
                .contains(&ResourceCapability::TerminalExec)
        );
        assert!(
            !resource
                .capabilities
                .contains(&ResourceCapability::RemoteExec)
        );
        assert_eq!(ResourceOrigin::ActiveSession, resource.origin);
    }

    #[test]
    fn saved_database_resource_has_database_capabilities() {
        let resource = connection_resource(stored_connection(
            42,
            "prod-db",
            ConnectionType::Database,
            r#"{"type":"mysql"}"#,
        ))
        .expect("database connection should become a resource");

        assert!(
            resource
                .capabilities
                .contains(&ResourceCapability::DatabaseQuery)
        );
        assert!(
            resource
                .capabilities
                .contains(&ResourceCapability::DatabaseExecute)
        );
    }

    #[test]
    fn saved_resource_has_management_and_open_session_capabilities() {
        let resource = connection_resource(stored_connection(
            42,
            "prod-db",
            ConnectionType::Database,
            r#"{"type":"mysql"}"#,
        ))
        .expect("saved connection should become a resource");

        assert!(
            resource
                .capabilities
                .contains(&ResourceCapability::ManageConnection)
        );
        assert!(
            resource
                .capabilities
                .contains(&ResourceCapability::OpenSession)
        );
    }

    #[test]
    fn saved_ssh_sftp_resource_has_file_capabilities() {
        let resource = connection_resource(stored_connection(
            42,
            "prod-a",
            ConnectionType::SshSftp,
            r#"{"host":"10.2.4.54"}"#,
        ))
        .expect("ssh/sftp connection should become a resource");

        assert!(resource.capabilities.contains(&ResourceCapability::List));
        assert!(
            resource
                .capabilities
                .contains(&ResourceCapability::ReadFile)
        );
        assert!(
            resource
                .capabilities
                .contains(&ResourceCapability::WriteFile)
        );
    }

    fn stored_connection(
        id: i64,
        name: &str,
        connection_type: ConnectionType,
        params: &str,
    ) -> StoredConnection {
        StoredConnection {
            id: Some(id),
            credential_revision: None,
            name: name.to_string(),
            connection_type,
            params: params.to_string(),
            workspace_id: None,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }
}
