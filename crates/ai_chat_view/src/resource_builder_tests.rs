use agent_runtime::{
    DefaultTargetReason, ResourceCapability, ResourceId, ResourceKind, ResourceScope,
};
use one_core::storage::{ConnectionType, StoredConnection};

use crate::{
    build_agent_context_all, build_agent_context_single_with_catalog,
    build_mentions_from_connections, build_mentions_single, build_resource_catalog,
    build_resource_context_all, build_resource_context_single, build_sidebar_resource_state,
    build_workbench_agent_context, build_workbench_resource_state,
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
fn database_connection_resource_has_database_capabilities() {
    let conn = stored_connection(
        42,
        "test-db",
        ConnectionType::Database,
        r#"{"type":"postgres"}"#,
    );

    let ctx = build_resource_context_single(&conn);

    assert!(
        ctx.resources[0]
            .capabilities
            .contains(&ResourceCapability::DatabaseQuery)
    );
    assert!(
        ctx.resources[0]
            .capabilities
            .contains(&ResourceCapability::DatabaseExecute)
    );
}

#[test]
fn saved_connection_resource_has_management_and_open_session_capabilities() {
    let conn = stored_connection(
        42,
        "test-db",
        ConnectionType::Database,
        r#"{"type":"postgres"}"#,
    );

    let ctx = build_resource_context_single(&conn);

    assert!(
        ctx.resources[0]
            .capabilities
            .contains(&ResourceCapability::ManageConnection)
    );
    assert!(
        ctx.resources[0]
            .capabilities
            .contains(&ResourceCapability::OpenSession)
    );
}

#[test]
fn ssh_sftp_connection_resource_has_file_capabilities() {
    let conn = stored_connection(42, "prod-a", ConnectionType::SshSftp, "{}");

    let ctx = build_resource_context_single(&conn);

    assert!(
        ctx.resources[0]
            .capabilities
            .contains(&ResourceCapability::ReadFile)
    );
    assert!(
        ctx.resources[0]
            .capabilities
            .contains(&ResourceCapability::WriteFile)
    );
    assert!(
        ctx.resources[0]
            .capabilities
            .contains(&ResourceCapability::List)
    );
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
fn single_connection_sets_connection_as_default_target() {
    let conn = stored_connection(42, "prod-a", ConnectionType::SshSftp, "{}");

    let ctx = build_resource_context_single(&conn);

    assert_eq!(1, ctx.resources.len());
    assert_eq!(
        Some("prod-a"),
        ctx.current().map(|resource| resource.label.as_str())
    );
}

#[test]
fn connection_host_is_resource_alias() {
    let conn = stored_connection(
        42,
        "prod-a",
        ConnectionType::SshSftp,
        r#"{"host":"10.2.4.54"}"#,
    );

    let ctx = build_resource_context_single(&conn);

    assert_eq!(vec!["10.2.4.54".to_string()], ctx.resources[0].aliases);
    assert_eq!(
        "prod-a",
        ctx.to_runtime_resource_pool()
            .resolve_target("10.2.4.54")
            .unwrap()
            .label
    );
}

#[test]
fn mention_detail_hides_cloud_uuid_but_keeps_host_context() {
    let mut conn = stored_connection(
        42,
        "10.1.131.181",
        ConnectionType::Database,
        r#"{"type":"mysql","host":"10.1.131.181","database":"ai_app2"}"#,
    );
    conn.cloud_id = Some("abfcee0a-2827-4588-9f6-587a7a95d1e9".to_string());

    let mut ctx = build_resource_context_single(&conn);
    let resource = ctx.resources.remove(0);
    let mention = build_mentions_single(&conn).remove(0);

    assert!(
        resource
            .aliases
            .iter()
            .any(|alias| alias == "abfcee0a-2827-4588-9f6-587a7a95d1e9")
    );
    assert_eq!("mysql · 10.1.131.181 · Database: ai_app2", mention.detail);
}

#[test]
fn all_connections_keep_all_resources_when_default_is_selected() {
    let conns = vec![
        stored_connection(1, "prod-a", ConnectionType::SshSftp, "{}"),
        stored_connection(2, "prod-b", ConnectionType::SshSftp, "{}"),
        stored_connection(
            3,
            "prod-db",
            ConnectionType::Database,
            r#"{"type":"mysql"}"#,
        ),
    ];

    let current = conns[1].clone();
    let ctx = build_resource_context_all(Some(&current), conns);

    assert_eq!(3, ctx.resources.len());
    assert_eq!(
        Some("prod-b"),
        ctx.current().map(|resource| resource.label.as_str())
    );
    assert!(
        ctx.resources
            .iter()
            .any(|resource| resource.label == "prod-a")
    );
    assert!(
        ctx.resources
            .iter()
            .any(|resource| resource.label == "prod-db")
    );
}

#[test]
fn connection_catalog_contains_all_saved_resources() {
    let conns = vec![
        stored_connection(1, "prod-a", ConnectionType::SshSftp, "{}"),
        stored_connection(2, "prod-b", ConnectionType::SshSftp, "{}"),
    ];

    let catalog = build_resource_catalog(&conns);

    assert_eq!(2, catalog.len());
    assert_eq!("prod-a", catalog[0].label);
    assert_eq!("prod-b", catalog[1].label);
}

#[test]
fn agent_context_single_can_receive_all_resources_as_catalog() {
    let conns = vec![
        stored_connection(1, "prod-a", ConnectionType::SshSftp, "{}"),
        stored_connection(2, "prod-b", ConnectionType::SshSftp, "{}"),
    ];
    let current = conns[0].clone();

    let (pool, mentions, catalog) = build_agent_context_single_with_catalog(&current, &conns);

    assert_eq!(1, pool.resources.len());
    assert_eq!(2, catalog.len());
    assert_eq!(2, mentions.len());
    assert_eq!("prod-a", mentions[0].label);
    assert_eq!("prod-b", mentions[1].label);
}

#[test]
fn connection_mentions_are_suggested_in_input() {
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
    assert!(mentions[0].detail.contains("127.0.0.1"));
}

#[test]
fn connection_list_mentions_are_suggested_in_input() {
    let conns = vec![
        stored_connection(
            1,
            "mysql-1",
            ConnectionType::Database,
            r#"{"type":"mysql"}"#,
        ),
        stored_connection(2, "redis-1", ConnectionType::Redis, "{}"),
    ];

    let mentions = build_mentions_from_connections(&conns);

    assert_eq!(2, mentions.len());
    assert_eq!("1", mentions[0].id);
    assert_eq!("mysql-1", mentions[0].label);
    assert_eq!("mysql", mentions[0].kind);
    assert_eq!("2", mentions[1].id);
    assert_eq!("redis-1", mentions[1].label);
    assert_eq!("redis", mentions[1].kind);
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

#[test]
fn workbench_agent_context_uses_all_connections_for_mentions_and_catalog() {
    let conns = vec![
        stored_connection(
            1,
            "mysql-1",
            ConnectionType::Database,
            r#"{"type":"mysql"}"#,
        ),
        stored_connection(2, "prod-ssh", ConnectionType::SshSftp, "{}"),
        stored_connection(3, "cache", ConnectionType::Redis, "{}"),
    ];

    let (resources, mentions, catalog) = build_workbench_agent_context(&conns);

    assert_eq!(3, resources.resources.len());
    assert_eq!(
        Some("mysql-1"),
        resources.current().map(|resource| resource.label.as_str())
    );
    assert_eq!(
        vec!["mysql-1", "prod-ssh", "cache"],
        mentions
            .iter()
            .map(|mention| mention.label.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        vec!["mysql-1", "prod-ssh", "cache"],
        catalog
            .iter()
            .map(|resource| resource.label.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn workbench_resource_state_has_catalog_but_empty_scope() {
    let conns = vec![
        stored_connection(1, "prod-a", ConnectionType::SshSftp, "{}"),
        stored_connection(
            2,
            "prod-db",
            ConnectionType::Database,
            r#"{"type":"mysql"}"#,
        ),
    ];

    let (scope, catalog, mentions) = build_workbench_resource_state(&conns);

    assert!(scope.selected.is_empty());
    assert!(scope.default_target.is_none());
    assert_eq!(2, catalog.resources.len());
    assert_eq!(
        vec!["prod-a", "prod-db"],
        mentions
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn sidebar_resource_state_keeps_current_connection_as_default_scope() {
    let conns = vec![
        stored_connection(1, "prod-a", ConnectionType::SshSftp, "{}"),
        stored_connection(
            2,
            "prod-db",
            ConnectionType::Database,
            r#"{"type":"mysql"}"#,
        ),
    ];

    let (scope, catalog, mentions) =
        build_sidebar_resource_state(&conns[1], &conns, DefaultTargetReason::CurrentDatabase);

    assert_eq!(1, scope.selected.len());
    assert_eq!("prod-db", scope.selected[0].label);
    assert_eq!(
        Some(&ResourceId::new("2")),
        scope
            .default_target
            .as_ref()
            .map(|target| &target.resource_id)
    );
    assert_eq!(2, catalog.resources.len());
    assert_eq!(2, mentions.len());
}
