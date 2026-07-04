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
    assert!(names.contains(&"db_tables".to_string()));
    assert!(names.contains(&"db_describe_table".to_string()));
    assert!(names.contains(&"db_sample_rows".to_string()));
    assert!(names.contains(&"db_exec".to_string()));
    assert!(!names.contains(&"db_execute_sql".to_string()));
    assert!(!names.contains(&"db_list_tables".to_string()));

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
    assert_agent_target_schema(&registry, "db.query", serde_json::json!(["target", "sql"]));
    assert_tool_risk(&registry, "db.tables", RiskLevel::Read);
    assert_tool_risk(&registry, "db.describe_table", RiskLevel::Read);
    assert_tool_risk(&registry, "db.sample_rows", RiskLevel::Read);
    assert_tool_risk(&registry, "db.exec", RiskLevel::High);
}

#[gpui::test]
fn agent_runtime_tool_registry_ignores_public_mcp_toolset_exposure(cx: &mut TestAppContext) {
    let registry = cx.update(|cx| {
        register_connection_repository(cx);
        let mut settings = AppSettings::default();
        settings.mcp.toolsets = McpToolsetSettings {
            terminal: false,
            connections: false,
            sftp: false,
            database: false,
            redis: false,
            internal_functions: false,
        };
        cx.set_global(settings);

        agent_runtime_tool_registry(cx).expect("agent registry should build")
    });
    let names = registry
        .names()
        .into_iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();

    assert!(names.contains(&"connections_set_sync_enabled".to_string()));
    assert!(names.contains(&"internal_functions_call".to_string()));
    assert!(names.contains(&"db_query".to_string()));
    assert!(names.contains(&"redis_get".to_string()));
    assert!(names.contains(&"sftp_read".to_string()));
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

    assert!(names.contains(&"redis_command".to_string()));
    assert!(names.contains(&"redis_keys".to_string()));
    assert!(names.contains(&"redis_get".to_string()));
    assert!(names.contains(&"redis_set".to_string()));
    assert!(!names.contains(&"redis_execute_command".to_string()));
    assert!(!names.contains(&"redis.execute_command".to_string()));
    assert_agent_target_schema(
        &registry,
        "redis.command",
        serde_json::json!(["target", "command"]),
    );
    assert_tool_risk(&registry, "redis.command", RiskLevel::High);
    assert_tool_risk(&registry, "redis.keys", RiskLevel::Medium);
    assert_tool_risk(&registry, "redis.get", RiskLevel::Low);
    assert_tool_risk(&registry, "redis.set", RiskLevel::High);
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

    assert!(names.contains(&"sftp_list".to_string()));
    assert!(names.contains(&"sftp_read".to_string()));
    assert!(names.contains(&"sftp_write".to_string()));
    assert!(names.contains(&"sftp_stat".to_string()));
    assert!(names.contains(&"sftp_upload".to_string()));
    assert!(names.contains(&"sftp_download".to_string()));
    assert!(!names.contains(&"ssh_list_dir".to_string()));
    assert!(!names.contains(&"ssh_read_file".to_string()));
    assert!(!names.contains(&"ssh_file_stat".to_string()));
    assert!(!names.contains(&"ssh_write_file".to_string()));
    assert!(!names.contains(&"sftp.write".to_string()));
    assert_agent_target_schema(&registry, "sftp.list", serde_json::json!(["target"]));
    assert_tool_risk(&registry, "sftp.list", RiskLevel::Read);
    assert_tool_risk(&registry, "sftp.read", RiskLevel::Read);
    assert_tool_risk(&registry, "sftp.write", RiskLevel::High);
    assert_tool_risk(&registry, "sftp.stat", RiskLevel::Read);
    assert_tool_risk(&registry, "sftp.upload", RiskLevel::High);
    assert_tool_risk(&registry, "sftp.download", RiskLevel::High);
}

fn register_connection_repository(cx: &mut gpui::App) {
    let storage = StorageManager::new_with_connection(test_connection());
    storage.register(ConnectionRepository::new(storage.connection()));
    storage.register(WorkspaceRepository::new(storage.connection()));
    cx.set_global(GlobalStorageState { storage });
}

fn assert_tool_risk(registry: &agent_runtime::ToolRegistry, name: &str, risk: RiskLevel) {
    let tool = registry.get(&ToolName::new(name)).expect(name);
    assert_eq!(risk, tool.spec(&ResourceContext::new()).risk, "{name} risk");
}

fn assert_agent_target_schema(
    registry: &agent_runtime::ToolRegistry,
    name: &str,
    required: serde_json::Value,
) {
    let tool = registry.get(&ToolName::new(name)).expect(name);
    let spec = tool.spec(&ResourceContext::new());
    let properties = spec.parameters["properties"].as_object().unwrap();
    assert_eq!(required, spec.parameters["required"], "{name} required");
    assert!(properties.contains_key("target"), "{name} target");
    assert!(!properties.contains_key("connection"), "{name} connection");
    assert!(
        !properties.contains_key("connection_id"),
        "{name} connection_id"
    );
    assert!(!properties.contains_key("session_id"), "{name} session_id");
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
