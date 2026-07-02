use super::agent_runtime_tool_registry;
use agent_runtime::{ResourceContext, RiskLevel, ToolName};
use gpui::TestAppContext;
use one_core::settings::{AppSettings, McpToolsetSettings};
use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::{
    ConnectionRepository, GlobalStorageState, StorageManager, WorkspaceRepository,
};

#[gpui::test]
fn agent_runtime_tool_registry_uses_native_database_tools(cx: &mut TestAppContext) {
    let registry = cx.update(|cx| {
        register_connection_repository(cx);
        let mut settings = AppSettings::default();
        settings.mcp.toolsets = McpToolsetSettings {
            terminal: false,
            connections: false,
            database: true,
            ..Default::default()
        };
        cx.set_global(settings);

        agent_runtime_tool_registry(cx).expect("agent registry should build")
    });
    let names = registry
        .names()
        .into_iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();

    assert!(names.contains(&"db_query".to_string()));
    assert!(names.contains(&"db_schema".to_string()));
    assert!(names.contains(&"db_execute_sql".to_string()));
    assert!(names.contains(&"db_list_tables".to_string()));
    assert!(
        !names.contains(&"db_exec".to_string()),
        "Agent registry must not expose write-capable db.exec through runtime bridge in Phase 3a"
    );

    let db_query = registry
        .get(&ToolName::new("db.query"))
        .expect("runtime-backed db.query should be registered");
    let spec = db_query.spec(&ResourceContext::new());
    assert_eq!(RiskLevel::Read, spec.risk);
    assert!(
        spec.description
            .contains("Run read-only SQL through a saved database connection"),
        "db_query should be backed by tool_runtime db.query descriptor"
    );
    assert_eq!(
        serde_json::json!(["connection", "sql"]),
        spec.parameters["required"]
    );
}

#[gpui::test]
fn agent_runtime_tool_registry_uses_native_redis_tools(cx: &mut TestAppContext) {
    let registry = cx.update(|cx| {
        register_connection_repository(cx);
        let mut settings = AppSettings::default();
        settings.mcp.toolsets = McpToolsetSettings {
            terminal: false,
            connections: false,
            redis: true,
            ..Default::default()
        };
        cx.set_global(settings);

        agent_runtime_tool_registry(cx).expect("agent registry should build")
    });
    let names = registry
        .names()
        .into_iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();

    assert!(names.contains(&"redis_execute_command".to_string()));
    assert!(
        !names.contains(&"redis.execute_command".to_string()),
        "Agent registry should not expose the old MCP redis.execute_command adapter"
    );
    let exec = registry
        .get(&ToolName::new("redis_execute_command"))
        .expect("redis execute tool");
    assert_eq!(
        RiskLevel::High,
        exec.spec(&ResourceContext::new()).risk,
        "Redis command execution must require approval through high risk"
    );
}

#[gpui::test]
fn agent_runtime_tool_registry_uses_native_ssh_sftp_tools(cx: &mut TestAppContext) {
    let registry = cx.update(|cx| {
        register_connection_repository(cx);
        let mut settings = AppSettings::default();
        settings.mcp.toolsets = McpToolsetSettings {
            terminal: false,
            connections: false,
            sftp: true,
            ..Default::default()
        };
        cx.set_global(settings);

        agent_runtime_tool_registry(cx).expect("agent registry should build")
    });
    let names = registry
        .names()
        .into_iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();

    assert!(names.contains(&"ssh_list_dir".to_string()));
    assert!(names.contains(&"ssh_read_file".to_string()));
    assert!(names.contains(&"ssh_file_stat".to_string()));
    assert!(names.contains(&"ssh_write_file".to_string()));
    assert!(
        !names.contains(&"sftp.write".to_string()),
        "Agent registry should not expose the old MCP sftp.write adapter"
    );
    let write = registry
        .get(&ToolName::new("ssh_write_file"))
        .expect("ssh write tool");
    assert_eq!(
        RiskLevel::High,
        write.spec(&ResourceContext::new()).risk,
        "SSH/SFTP writes must require approval through high risk"
    );
}

fn register_connection_repository(cx: &mut gpui::App) {
    let storage = StorageManager::new_with_connection(test_connection());
    storage.register(ConnectionRepository::new(storage.connection()));
    storage.register(WorkspaceRepository::new(storage.connection()));
    cx.set_global(GlobalStorageState { storage });
}

fn test_connection() -> SqliteConnection {
    let conn = SqliteConnection::open_with_pool_size(":memory:", 1)
        .expect("sqlite connection should open");
    conn.with_connection(|db| {
        run_migrations(db)?;
        Ok(())
    })
    .expect("migrations should run");
    conn
}
