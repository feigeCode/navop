use agent_runtime::{ResourceKind, ResourceScope};
use one_core::storage::{ConnectionType, StoredConnection};

use crate::{
    build_agent_context_all, build_mentions_single, build_resource_context_all,
    build_resource_context_single,
};

fn stored_connection(
    id: i64,
    name: &str,
    connection_type: ConnectionType,
    params: &str,
) -> StoredConnection {
    StoredConnection {
        id: Some(id),
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

#[test]
fn single_connection_builds_context_with_one_resource() {
    let conn = stored_connection(
        42,
        "test-db",
        ConnectionType::Database,
        r#"{"type":"postgres"}"#,
    );

    let ctx = build_resource_context_single(&conn);

    assert_eq!(ctx.resources.len(), 1);
    assert_eq!(ctx.resources[0].label, "test-db");
    assert_eq!(ctx.resources[0].kind, ResourceKind::Postgres);
}

#[test]
fn all_connections_builds_context_with_multiple_resources() {
    let conns = vec![
        stored_connection(
            1,
            "mysql-1",
            ConnectionType::Database,
            r#"{"type":"mysql"}"#,
        ),
        stored_connection(2, "redis-1", ConnectionType::Redis, "{}"),
    ];

    let current = conns[0].clone();
    let ctx = build_resource_context_all(Some(&current), conns);

    assert_eq!(ctx.resources.len(), 2);
    assert!(ctx.current.is_some());
    assert_eq!(ctx.current().unwrap().label, "mysql-1");
}

#[test]
fn mentions_include_connection_id_label_kind_and_endpoint() {
    let conn = stored_connection(
        7,
        "cache",
        ConnectionType::Redis,
        r#"{"host":"127.0.0.1","port":6379}"#,
    );

    let mentions = build_mentions_single(&conn);

    assert_eq!(1, mentions.len());
    assert_eq!("7", mentions[0].id);
    assert_eq!("cache", mentions[0].label);
    assert_eq!("redis", mentions[0].kind);
    assert_eq!("redis | 127.0.0.1:6379", mentions[0].detail);
}

#[test]
fn database_connection_scopes_include_selected_database_and_schema() {
    let mut conn = stored_connection(
        9,
        "pg",
        ConnectionType::Database,
        r#"{"type":"postgres","schema":"public"}"#,
    );
    conn.selected_databases = Some(r#"["ai_app"]"#.to_string());

    let ctx = build_resource_context_single(&conn);

    assert_eq!(
        vec![
            ResourceScope::new("database", "Database", "ai_app"),
            ResourceScope::new("schema", "Schema", "public")
        ],
        ctx.resources[0].scopes
    );
}

#[test]
fn agent_context_all_pairs_resources_with_mentions() {
    let conns = vec![
        stored_connection(1, "mongo-1", ConnectionType::MongoDB, "{}"),
        stored_connection(2, "ssh-1", ConnectionType::SshSftp, "{}"),
    ];

    let (ctx, mentions) = build_agent_context_all(Some(&conns[1]), &conns);

    assert_eq!(2, ctx.resources.len());
    assert_eq!(2, mentions.len());
    assert_eq!(
        Some("ssh-1"),
        ctx.current().map(|resource| resource.label.as_str())
    );
    assert_eq!(ResourceKind::Mongo, ctx.resources[0].kind);
}
