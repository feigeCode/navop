//! 从应用连接数据构建 AgentChatView 的 ResourceContext。

use agent_runtime::{ResourceContext, ResourceId, ResourceKind, ResourceRef, ResourceScope};
use one_core::storage::{ConnectionType, StoredConnection};
use serde_json::Value;

use crate::input::MentionItem;

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

/// 从所有连接构建 ResourceContext，并设置当前连接（用于非侧边栏模式）。
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

/// 从所有连接构建 Agent 视图所需的资源上下文与 `@` 提及项。
pub fn build_agent_context_all(
    current_connection: Option<&StoredConnection>,
    connections: &[StoredConnection],
) -> (ResourceContext, Vec<MentionItem>) {
    (
        build_resource_context_all(current_connection, connections.to_vec()),
        build_mentions_from_connections(connections),
    )
}

/// 从单个连接构建 `@` 提及项。
///
/// 连接本身通过顶部上下文选择器切换,不进入输入框 `@` 补全。
pub fn build_mentions_single(_connection: &StoredConnection) -> Vec<MentionItem> {
    Vec::new()
}

/// 从连接列表构建 `@` 提及项。
///
/// 连接本身通过顶部上下文选择器切换,不进入输入框 `@` 补全。
pub fn build_mentions_from_connections(_connections: &[StoredConnection]) -> Vec<MentionItem> {
    Vec::new()
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
    for scope in connection_scopes(connection) {
        resource.set_scope(scope);
    }
    resource
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
        ConnectionType::SshSftp => ResourceKind::Ssh,
        ConnectionType::Serial => ResourceKind::Terminal,
        ConnectionType::PortForwarding => ResourceKind::Other("port-forwarding".into()),
        ConnectionType::Rdp => ResourceKind::Other("rdp".into()),
        ConnectionType::Vnc => ResourceKind::Other("vnc".into()),
        ConnectionType::All => ResourceKind::Other("all".into()),
    }
}
