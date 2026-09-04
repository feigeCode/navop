//! 从应用连接数据构建 AgentChatView 的 ResourceContext。

use agent_runtime::{
    AgentResourceScope, DefaultTargetReason, ResourceCapability, ResourceCatalog, ResourceContext,
    ResourceId, ResourceKind, ResourceRef, ResourceScope,
};
use one_core::storage::{ConnectionType, StoredConnection};
use serde_json::Value;

use crate::input::MentionItem;
use crate::resource_display::{first_visible_alias, visible_alias};

/// 从单个连接构建 ResourceContext（用于侧边栏模式）。
pub fn build_resource_context_single(connection: &StoredConnection) -> ResourceContext {
    let resource = connection_to_resource_ref(connection);
    ResourceContext::new().with_resource(resource)
}

/// 从单个连接构建 Agent 视图所需的资源上下文与 `@` 提及项。
pub fn build_agent_context_single(
    connection: &StoredConnection,
) -> (ResourceContext, Vec<MentionItem>) {
    (
        build_resource_context_single(connection),
        build_mentions_single(connection),
    )
}

/// 从连接列表构建可添加到资源池的资源 catalog。
pub fn build_resource_catalog(connections: &[StoredConnection]) -> Vec<ResourceRef> {
    connections.iter().map(connection_to_resource_ref).collect()
}

/// 从单个连接构建侧边栏默认资源池,并附带全部连接 catalog。
pub fn build_agent_context_single_with_catalog(
    connection: &StoredConnection,
    connections: &[StoredConnection],
) -> (ResourceContext, Vec<MentionItem>, Vec<ResourceRef>) {
    let (pool, mentions) = build_agent_context_single(connection);
    let mentions = if connections.is_empty() {
        mentions
    } else {
        build_mentions_from_connections(connections)
    };
    (pool, mentions, build_resource_catalog(connections))
}

/// 从所有连接构建 ResourceContext，并设置默认目标（用于非侧边栏模式）。
pub fn build_resource_context_all(
    current_connection: Option<&StoredConnection>,
    all_connections: Vec<StoredConnection>,
) -> ResourceContext {
    let mut ctx = ResourceContext::new();
    let mut current_id: Option<ResourceId> = None;

    for conn in all_connections {
        let resource = connection_to_resource_ref(&conn);
        if let (Some(current), Some(conn_id)) = (current_connection, &conn.id) {
            if current.id == Some(*conn_id) {
                current_id = Some(resource.id.clone());
            }
        }
        ctx = ctx.with_resource(resource);
    }

    if let Some(id) = current_id {
        ctx.current = Some(id);
    }

    ctx
}

/// 从所有连接构建 Agent 视图所需的资源池与 `@` 提及项。
pub fn build_agent_context_all(
    current_connection: Option<&StoredConnection>,
    connections: &[StoredConnection],
) -> (ResourceContext, Vec<MentionItem>) {
    (
        build_resource_context_all(current_connection, connections.to_vec()),
        build_mentions_from_connections(connections),
    )
}

/// 从所有连接构建工作台模式所需的资源池、`@` 提及项与可添加资源 catalog。
pub fn build_workbench_agent_context(
    connections: &[StoredConnection],
) -> (ResourceContext, Vec<MentionItem>, Vec<ResourceRef>) {
    let (resources, mentions) = build_agent_context_all(connections.first(), connections);
    let catalog = build_resource_catalog(connections);
    (resources, mentions, catalog)
}

/// 从所有连接构建工作台资源状态:catalog 全量可见,scope 初始为空。
pub fn build_workbench_resource_state(
    connections: &[StoredConnection],
) -> (AgentResourceScope, ResourceCatalog, Vec<MentionItem>) {
    let catalog = ResourceCatalog::new(build_resource_catalog(connections));
    let mentions = build_mentions_from_connections(connections);
    (AgentResourceScope::empty(), catalog, mentions)
}

/// 从当前连接和全量连接构建侧边栏资源状态。
pub fn build_sidebar_resource_state(
    current_connection: &StoredConnection,
    connections: &[StoredConnection],
    reason: DefaultTargetReason,
) -> (AgentResourceScope, ResourceCatalog, Vec<MentionItem>) {
    let catalog_connections = if connections.is_empty() {
        vec![current_connection.clone()]
    } else {
        connections.to_vec()
    };
    let catalog = ResourceCatalog::new(build_resource_catalog(&catalog_connections));
    let current_resource = connection_to_resource_ref(current_connection);
    let scope = AgentResourceScope::single_default(current_resource, reason);
    let mentions = build_mentions_from_connections(&catalog_connections);
    (scope, catalog, mentions)
}

/// 从单个连接构建 `@` 提及项。
pub fn build_mentions_single(connection: &StoredConnection) -> Vec<MentionItem> {
    connection_to_mention(connection).into_iter().collect()
}

/// 从连接列表构建 `@` 提及项。
pub fn build_mentions_from_connections(connections: &[StoredConnection]) -> Vec<MentionItem> {
    connections
        .iter()
        .filter_map(connection_to_mention)
        .collect()
}

/// 将 StoredConnection 转换为 ResourceRef。
fn connection_to_resource_ref(connection: &StoredConnection) -> ResourceRef {
    let kind = connection_type_to_resource_kind(&connection.connection_type, &connection.params);
    let label = if connection.name.is_empty() {
        format!("连接 {:?}", connection.id)
    } else {
        connection.name.clone()
    };

    let mut resource = ResourceRef::new(
        connection
            .id
            .map_or_else(|| "unknown".to_string(), |id| id.to_string()),
        kind,
        label,
    );
    for alias in connection_aliases(connection) {
        resource = resource.with_alias(alias);
    }
    for scope in connection_scopes(connection) {
        resource.set_scope(scope);
    }
    for capability in connection_capabilities(connection) {
        resource = resource.with_capability(capability);
    }
    resource
}

fn connection_to_mention(connection: &StoredConnection) -> Option<MentionItem> {
    let resource = connection_to_resource_ref(connection);
    let id = connection.id?.to_string();
    let label = resource.label.clone();
    let display_label = mention_display_label(&resource);
    let detail = mention_detail(&resource);
    let kind = resource.kind.as_str().to_string();
    Some(MentionItem::new(id, label, detail, kind).with_display_label(display_label))
}

fn mention_display_label(resource: &ResourceRef) -> String {
    first_visible_alias(&resource.aliases)
        .filter(|alias| alias != &resource.label)
        .map(|alias| format!("{} · {}", resource.label, alias))
        .unwrap_or_else(|| resource.label.clone())
}

fn mention_detail(resource: &ResourceRef) -> String {
    let mut parts = vec![resource.kind.as_str().to_string()];
    parts.extend(
        resource
            .aliases
            .iter()
            .filter(|alias| visible_alias(alias))
            .cloned(),
    );
    parts.extend(
        resource
            .scopes
            .iter()
            .map(|scope| format!("{}: {}", scope.label, scope.value)),
    );
    parts.join(" · ")
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
        .filter_map(|key| string_field(&map, &[key]))
        .filter(|value| !value.is_empty())
        .collect()
}

fn string_field(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match map.get(*key) {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    })
}

fn connection_scopes(connection: &StoredConnection) -> Vec<ResourceScope> {
    if connection.connection_type != ConnectionType::Database {
        return Vec::new();
    }
    let mut scopes = Vec::new();
    if let Some(database) = selected_database_scope(connection) {
        scopes.push(ResourceScope::new("database", "Database", database));
    }
    if let Some(schema) = params_scope_field(&connection.params, "schema") {
        scopes.push(ResourceScope::new("schema", "Schema", schema));
    }
    scopes
}

fn selected_database_scope(connection: &StoredConnection) -> Option<String> {
    connection
        .selected_databases
        .as_ref()
        .and_then(|items| single_selected_database(items))
        .or_else(|| params_scope_field(&connection.params, "database"))
}

fn single_selected_database(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('[') {
        let parsed = serde_json::from_str::<Vec<String>>(trimmed).ok()?;
        return (parsed.len() == 1).then(|| parsed[0].clone());
    }
    (!trimmed.contains(',')).then(|| trimmed.to_string())
}

fn params_scope_field(params: &str, key: &str) -> Option<String> {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(params) else {
        return None;
    };
    string_field(&map, &[key]).filter(|value| !value.is_empty())
}

/// 将 ConnectionType 转换为 ResourceKind。
fn connection_type_to_resource_kind(conn_type: &ConnectionType, params: &str) -> ResourceKind {
    match conn_type {
        ConnectionType::Database => {
            // 从 params 解析数据库类型
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(params) {
                if let Some(Value::String(db_type)) = map.get("type") {
                    return match db_type.as_str() {
                        "mysql" => ResourceKind::Mysql,
                        "postgres" | "postgresql" => ResourceKind::Postgres,
                        "sqlite" => ResourceKind::Sqlite,
                        _ => ResourceKind::Other(db_type.clone()),
                    };
                }
            }
            ResourceKind::Other("database".into())
        }
        ConnectionType::Redis => ResourceKind::Redis,
        ConnectionType::MongoDB => ResourceKind::Mongo,
        ConnectionType::Mqtt => ResourceKind::Other("mqtt".into()),
        ConnectionType::SshSftp => ResourceKind::Ssh,
        ConnectionType::Serial => ResourceKind::Terminal,
        ConnectionType::Telnet => ResourceKind::Terminal,
        ConnectionType::PortForwarding => ResourceKind::Other("port-forwarding".into()),
        ConnectionType::Rdp => ResourceKind::Other("rdp".into()),
        ConnectionType::Vnc => ResourceKind::Other("vnc".into()),
        ConnectionType::Extension => {
            serde_json::from_str::<one_core::storage::ExtensionConnectionParams>(params)
                .map(|params| {
                    ResourceKind::Other(format!(
                        "extension:{}:{}",
                        params.extension_id, params.contribution_id
                    ))
                })
                .unwrap_or_else(|_| ResourceKind::Other("extension".into()))
        }
        ConnectionType::All => ResourceKind::Other("all".into()),
    }
}
