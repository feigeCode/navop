use ai_chat_view::AiAccessLevels;
use anyhow::Result;
use gpui::App;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, DatabaseType, GlobalStorageState, StoredConnection,
};
use public_mcp::registry::PublicMcpRegistry;
use public_mcp::tools::ResourcePoolProvider;
#[cfg(feature = "database")]
use redis_view::GlobalRedisState;

use serde_json::Value;
use std::sync::Arc;
use tool_runtime::{
    AiAccessLevel, ResourceCapability, ResourceKind, ResourceOrigin, ResourcePool, ResourceRef,
};

pub(super) fn app_resource_pool_provider(cx: &App) -> Option<ResourcePoolProvider> {
    let repo = connection_repository(cx);
    let terminal_registry = terminal_view::public_mcp::registry(cx);
    #[cfg(feature = "database")]
    let redis_state = cx.try_global::<GlobalRedisState>().cloned();
    #[cfg(not(feature = "database"))]
    let redis_state: Option<()> = None;
    if repo.is_none() && terminal_registry.is_none() && redis_state.is_none() {
        return None;
    }
    Some(Arc::new(move || {
        #[cfg(feature = "database")]
        let pool = resource_pool_from_sources(
            repo.as_ref(),
            terminal_registry.as_ref(),
            redis_state.as_ref(),
        );
        #[cfg(not(feature = "database"))]
        let pool = resource_pool_from_sources(repo.as_ref(), terminal_registry.as_ref());
        match pool {
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

#[cfg(feature = "database")]
fn resource_pool_from_sources(
    repo: Option<&Arc<ConnectionRepository>>,
    terminal_registry: Option<&PublicMcpRegistry>,
    redis_state: Option<&GlobalRedisState>,
) -> Result<ResourcePool> {
    // 一次读取连接级 AI 访问策略，随后对已保存连接、活动终端会话和活动 Redis
    // 连接统一收窄；公共 MCP 会话与内嵌 AI 共用同一份策略，避免出现绕过通道。
    let ai_access_levels = load_ai_access_levels(repo)?;
    let mut pool = match repo {
        Some(repo) => saved_connection_resource_pool(repo, &ai_access_levels)?,
        None => ResourcePool::new(),
    };
    if let Some(registry) = terminal_registry {
        pool = registry
            .list_sessions()
            .into_iter()
            .filter_map(|session| terminal_session_resource(session, &ai_access_levels))
            .fold(pool, ResourcePool::with_resource);
    }
    if let Some(state) = redis_state {
        pool = append_active_redis_resources(pool, state.connection_ids(), &ai_access_levels);
    }
    Ok(pool)
}

#[cfg(not(feature = "database"))]
fn resource_pool_from_sources(
    repo: Option<&Arc<ConnectionRepository>>,
    terminal_registry: Option<&PublicMcpRegistry>,
) -> Result<ResourcePool> {
    let ai_access_levels = load_ai_access_levels(repo)?;
    let mut pool = match repo {
        Some(repo) => saved_connection_resource_pool(repo, &ai_access_levels)?,
        None => ResourcePool::new(),
    };
    if let Some(registry) = terminal_registry {
        pool = registry
            .list_sessions()
            .into_iter()
            .filter_map(|session| terminal_session_resource(session, &ai_access_levels))
            .fold(pool, ResourcePool::with_resource);
    }
    Ok(pool)
}

/// 读取连接级 AI 访问策略；没有可用的连接仓库时视为全部允许。
fn load_ai_access_levels(repo: Option<&Arc<ConnectionRepository>>) -> Result<AiAccessLevels> {
    match repo {
        Some(repo) => repo.ai_access_levels(),
        None => Ok(AiAccessLevels::new()),
    }
}

fn ai_access_level(ai_access_levels: &AiAccessLevels, connection_id: i64) -> AiAccessLevel {
    ai_access_levels
        .get(&connection_id)
        .copied()
        .unwrap_or(AiAccessLevel::Allowed)
}

pub(super) fn saved_connection_resource_pool(
    repo: &ConnectionRepository,
    ai_access_levels: &AiAccessLevels,
) -> Result<ResourcePool> {
    let connections = repo.list()?;
    Ok(connections
        .into_iter()
        .filter_map(|connection| connection_resource(connection, ai_access_levels))
        .fold(ResourcePool::new(), ResourcePool::with_resource))
}

fn connection_resource(
    connection: StoredConnection,
    ai_access_levels: &AiAccessLevels,
) -> Option<ResourceRef> {
    let connection_id = connection.id?;
    let ai_access_level = ai_access_level(ai_access_levels, connection_id);
    if ai_access_level == AiAccessLevel::Denied {
        return None;
    }
    let id = connection_id.to_string();
    let label = if connection.name.is_empty() {
        format!("connection {id}")
    } else {
        connection.name.clone()
    };
    let mut resource = ResourceRef::new(id, connection_kind(&connection), label);
    for alias in connection_aliases(&connection) {
        resource = resource.with_alias(alias);
    }
    for capability in ai_access_level.narrow_capabilities(&connection_capabilities(&connection)) {
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

fn terminal_session_resource(
    session: public_mcp::registry::PublicMcpSessionInfo,
    ai_access_levels: &AiAccessLevels,
) -> Option<ResourceRef> {
    // 本地终端没有关联连接，不受连接级策略影响；关联连接被禁止时整个会话
    // 都不进入资源池，避免 AI 通过活动会话绕过限制。
    let ai_access_level = session
        .connection_id
        .map(|connection_id| ai_access_level(ai_access_levels, connection_id))
        .unwrap_or(AiAccessLevel::Allowed);
    // 与「禁止 AI 操作」的已保存连接保持一致：直接移出资源池，而不是保留资源
    // 再清空能力。保留「可见但零能力」的目标会让模型看到资源却永远调不通，
    // 容易诱发无效重试；直接不在池里则由路由层按未知目标拒绝，语义也更统一。
    if ai_access_level == AiAccessLevel::Denied {
        return None;
    }
    let label = if session.host_label.is_empty() {
        session.title.clone()
    } else {
        session.host_label.clone()
    };
    let mut resource = ResourceRef::new(session.session_id.clone(), ResourceKind::Terminal, label)
        .with_alias(session.session_id);
    resource.origin = ResourceOrigin::ActiveSession;
    for capability in ai_access_level.narrow_capabilities(&session.capabilities) {
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
    Some(resource)
}

#[cfg(feature = "database")]
fn append_active_redis_resources(
    pool: ResourcePool,
    connection_ids: Vec<String>,
    ai_access_levels: &AiAccessLevels,
) -> ResourcePool {
    connection_ids
        .into_iter()
        .filter(|connection_id| !connection_id.is_empty())
        .filter_map(|connection_id| active_redis_resource(connection_id, ai_access_levels))
        .fold(pool, insert_or_merge_resource)
}

fn active_redis_resource(
    connection_id: String,
    ai_access_levels: &AiAccessLevels,
) -> Option<ResourceRef> {
    let ai_access_level = connection_id
        .parse::<i64>()
        .ok()
        .map(|id| ai_access_level(ai_access_levels, id))
        .unwrap_or(AiAccessLevel::Allowed);
    if ai_access_level == AiAccessLevel::Denied {
        return None;
    }
    let mut resource = ResourceRef::new(
        connection_id.clone(),
        ResourceKind::Redis,
        format!("redis {connection_id}"),
    )
    .with_alias(connection_id);
    resource.origin = ResourceOrigin::ActiveSession;
    for capability in ai_access_level.narrow_capabilities(&[ResourceCapability::Execute]) {
        resource = resource.with_capability(capability);
    }
    Some(resource)
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
        ConnectionType::Ftp => ResourceKind::Other("ftp".into()),
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
        let pool = append_active_redis_resources(
            ResourcePool::new(),
            vec!["redis-a".to_string()],
            &no_levels(),
        );

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

        let pool = append_active_redis_resources(pool, vec!["21".to_string()], &no_levels());

        assert_eq!(1, pool.resources.len());
        let resource = pool.resolve_target("10.2.4.54").unwrap();
        assert_eq!("21", resource.id.as_str());
        assert!(resource.capabilities.contains(&ResourceCapability::Execute));
    }

    #[test]
    fn terminal_session_resource_preserves_registry_capabilities() {
        let resource = terminal_session_resource(
            public_mcp::registry::PublicMcpSessionInfo {
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
            },
            &no_levels(),
        )
        .expect("未被禁止会话应生成资源");

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
        let resource = connection_resource(
            stored_connection(
                42,
                "prod-db",
                ConnectionType::Database,
                r#"{"type":"mysql"}"#,
            ),
            &no_levels(),
        )
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
        let resource = connection_resource(
            stored_connection(
                42,
                "prod-db",
                ConnectionType::Database,
                r#"{"type":"mysql"}"#,
            ),
            &no_levels(),
        )
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
        let resource = connection_resource(
            stored_connection(
                42,
                "prod-a",
                ConnectionType::SshSftp,
                r#"{"host":"10.2.4.54"}"#,
            ),
            &no_levels(),
        )
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

    fn no_levels() -> AiAccessLevels {
        AiAccessLevels::new()
    }

    fn ai_levels(pairs: &[(i64, AiAccessLevel)]) -> AiAccessLevels {
        pairs.iter().copied().collect()
    }

    fn session_info(
        connection_id: Option<i64>,
        capabilities: Vec<ResourceCapability>,
    ) -> public_mcp::registry::PublicMcpSessionInfo {
        public_mcp::registry::PublicMcpSessionInfo {
            session_id: "terminal-7".to_string(),
            connection_id,
            title: "root@prod-a:~".to_string(),
            host_label: "prod-a".to_string(),
            cwd: Some("/root".to_string()),
            rows: 24,
            cols: 120,
            connection_kind: public_mcp::registry::TerminalConnectionKind::Ssh,
            connected: true,
            capabilities,
        }
    }

    fn connection_repo() -> Arc<ConnectionRepository> {
        let conn =
            one_core::storage::connection::SqliteConnection::open_with_pool_size(":memory:", 1)
                .expect("内存 SQLite 应能打开");
        conn.with_connection(|db| {
            one_core::storage::migration::run_migrations(db)?;
            Ok(())
        })
        .expect("迁移应成功");
        Arc::new(ConnectionRepository::new(conn))
    }

    fn insert_connection(repo: &ConnectionRepository, name: &str) -> i64 {
        let mut connection =
            stored_connection(0, name, ConnectionType::Database, r#"{"type":"mysql"}"#);
        connection.id = None;
        repo.insert(&mut connection).expect("连接应写入成功")
    }

    #[test]
    fn denied_saved_connection_is_absent_from_resource_pool() {
        let repo = connection_repo();
        let allowed = insert_connection(&repo, "allowed-db");
        let denied = insert_connection(&repo, "denied-db");
        repo.set_ai_access_level(denied, AiAccessLevel::Denied)
            .expect("策略应写入成功");
        let levels = repo.ai_access_levels().expect("策略应可读取");

        let pool = saved_connection_resource_pool(&repo, &levels).expect("资源池应构建成功");

        assert!(
            pool.resolve_target(&allowed.to_string()).is_ok(),
            "允许的连接应出现在资源池"
        );
        assert!(
            pool.resolve_target(&denied.to_string()).is_err(),
            "被禁止 AI 操作的连接不应出现在 Public MCP 资源池"
        );
    }

    #[test]
    fn read_only_saved_connection_keeps_only_read_only_capabilities() {
        let repo = connection_repo();
        let read_only = insert_connection(&repo, "readonly-db");
        repo.set_ai_access_level(read_only, AiAccessLevel::ReadOnly)
            .expect("策略应写入成功");
        let levels = repo.ai_access_levels().expect("策略应可读取");

        let pool = saved_connection_resource_pool(&repo, &levels).expect("资源池应构建成功");
        let resource = pool
            .resolve_target(&read_only.to_string())
            .expect("只读连接仍应出现在资源池");

        assert!(
            resource
                .capabilities
                .contains(&ResourceCapability::DatabaseQuery),
            "只读连接应保留查询能力: {:?}",
            resource.capabilities
        );
        assert!(
            !resource
                .capabilities
                .contains(&ResourceCapability::DatabaseExecute),
            "只读连接不应暴露写能力: {:?}",
            resource.capabilities
        );
        assert!(
            !resource
                .capabilities
                .contains(&ResourceCapability::ManageConnection)
        );
        assert!(
            !resource
                .capabilities
                .contains(&ResourceCapability::OpenSession)
        );
    }

    #[test]
    fn terminal_session_of_denied_connection_is_hidden() {
        let levels = ai_levels(&[(21, AiAccessLevel::Denied)]);

        let resource = terminal_session_resource(
            session_info(Some(21), vec![ResourceCapability::TerminalExec]),
            &levels,
        );

        assert!(
            resource.is_none(),
            "被禁止连接的终端会话不应进入资源池: {resource:?}"
        );
    }

    #[test]
    fn terminal_session_of_read_only_connection_loses_exec_capabilities() {
        let levels = ai_levels(&[(21, AiAccessLevel::ReadOnly)]);

        let resource = terminal_session_resource(
            session_info(
                Some(21),
                vec![
                    ResourceCapability::TerminalExec,
                    ResourceCapability::TerminalInput,
                ],
            ),
            &levels,
        )
        .expect("只读连接的会话仍应可见");

        assert!(
            !resource
                .capabilities
                .contains(&ResourceCapability::TerminalExec),
            "只读连接不应暴露终端执行能力: {:?}",
            resource.capabilities
        );
        assert!(
            !resource
                .capabilities
                .contains(&ResourceCapability::TerminalInput)
        );
    }

    #[test]
    fn local_terminal_session_without_connection_is_unaffected() {
        let levels = ai_levels(&[(21, AiAccessLevel::Denied)]);

        let resource = terminal_session_resource(
            session_info(None, vec![ResourceCapability::TerminalExec]),
            &levels,
        )
        .expect("本地终端没有连接策略，应保留");

        assert!(
            resource
                .capabilities
                .contains(&ResourceCapability::TerminalExec)
        );
    }

    #[test]
    fn active_redis_resources_apply_connection_policy() {
        let levels = ai_levels(&[(21, AiAccessLevel::Denied)]);

        let pool = append_active_redis_resources(
            ResourcePool::new(),
            vec!["21".to_string(), "redis-a".to_string()],
            &levels,
        );
        assert!(
            pool.resolve_target("21").is_err(),
            "被禁止连接的 Redis 活动资源不应出现"
        );
        assert!(pool.resolve_target("redis-a").is_ok());

        let levels = ai_levels(&[(21, AiAccessLevel::ReadOnly)]);
        let pool =
            append_active_redis_resources(ResourcePool::new(), vec!["21".to_string()], &levels);
        let resource = pool.resolve_target("21").expect("只读连接仍应可见");
        assert!(
            !resource.capabilities.contains(&ResourceCapability::Execute),
            "只读连接不应暴露 Redis 执行能力: {:?}",
            resource.capabilities
        );
    }

    fn stored_connection(
        id: i64,
        name: &str,
        connection_type: ConnectionType,
        params: &str,
    ) -> StoredConnection {
        StoredConnection {
            ai_access_level: Default::default(),
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
            preferred_open_mode: None,
        }
    }
}
