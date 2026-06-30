use super::agent_runtime_tool_registry;
use gpui::TestAppContext;
use one_core::settings::{AppSettings, McpToolsetSettings};
use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::{
    ConnectionRepository, GlobalStorageState, StorageManager, WorkspaceRepository,
};

#[gpui::test]
fn agent_runtime_tool_registry_uses_native_database_tools(cx: &mut TestAppContext) {
    let names = cx.update(|cx| {
        register_connection_repository(cx);
        let mut settings = AppSettings::default();
        settings.mcp.toolsets = McpToolsetSettings {
            terminal: false,
            connections: false,
            database: true,
            ..Default::default()
        };
        cx.set_global(settings);

        agent_runtime_tool_registry(cx)
            .expect("agent registry should build")
            .names()
            .into_iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
    });

    assert!(names.contains(&"db_query".to_string()));
    assert!(names.contains(&"db_execute_sql".to_string()));
    assert!(names.contains(&"db_list_tables".to_string()));
    assert!(
        !names.contains(&"db_exec".to_string()),
        "Agent registry should not expose the old MCP db.exec adapter"
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
