use super::{
    ConnectionSaveEvent, ConnectionSaveNotifier, ConnectionToolHooks, connection_tool_registry,
    connection_tool_registry_with_workspaces_and_hooks,
};
use db::ipc::{IpcDriverEntry, IpcDriverManifest, IpcDriverRegistry, IpcDriverTransport};
use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, DatabaseType};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tool_runtime::{ResourceCapability, RiskLevel, ToolAdapter, ToolContext};

mod create_extended;
mod management;

#[derive(Default)]
struct RecordingSaveNotifier {
    events: Mutex<Vec<ConnectionSaveEvent>>,
}

impl RecordingSaveNotifier {
    fn drain(&self) -> Vec<ConnectionSaveEvent> {
        self.events.lock().expect("events lock").drain(..).collect()
    }
}

impl ConnectionSaveNotifier for RecordingSaveNotifier {
    fn notify_save(&self, event: ConnectionSaveEvent) -> super::ConnectionSaveNotifyFuture {
        self.events.lock().expect("events lock").push(event);
        Box::pin(async { Ok(()) })
    }
}

#[test]
fn connection_registry_lists_save_tools() {
    let registry = connection_tool_registry(repo());
    let tools = registry.list(ToolAdapter::Mcp);
    let tool_ids = tools.iter().map(|tool| tool.id.clone()).collect::<Vec<_>>();
    let save = tools
        .iter()
        .find(|tool| tool.id == "connections.save")
        .expect("save tool should be registered");
    let open_session = tools
        .iter()
        .find(|tool| tool.id == "connections.open_session")
        .expect("open_session tool should be registered");

    assert_eq!(
        json!([
            { "required": ["kind", "values"] },
            { "required": ["id", "patch"] }
        ]),
        save.input_schema["oneOf"]
    );
    assert!(
        save.description
            .contains("Call connections.get_schema first")
    );
    assert!(
        open_session
            .description
            .contains("open the session in the background")
    );
    assert!(open_session.description.contains("session.activated"));
    assert_eq!(
        json!("integer"),
        save.input_schema["properties"]["id"]["type"]
    );

    assert!(tool_ids.contains(&"connections.list_kinds".to_string()));
    assert!(tool_ids.contains(&"connections.get_schema".to_string()));
    assert!(tool_ids.contains(&"connections.validate".to_string()));
    assert!(tool_ids.contains(&"connections.save".to_string()));
    assert!(tool_ids.contains(&"connections.open_session".to_string()));
    assert!(!tool_ids.contains(&"connections.create".to_string()));
    assert!(!tool_ids.contains(&"connections.update".to_string()));
    assert!(!tool_ids.contains(&"connections.move_workspace".to_string()));
    assert!(!tool_ids.contains(&"connections.set_sync_enabled".to_string()));
    assert!(!tool_ids.iter().any(|id| id.starts_with("onetcli.")));
}

#[test]
fn connection_show_descriptor_identifies_connection_reference() {
    let registry = connection_tool_registry(repo());
    let tool = registry
        .list(ToolAdapter::Mcp)
        .into_iter()
        .find(|tool| tool.id == "connections.show")
        .expect("show tool should be registered");

    assert_eq!(json!(["connection"]), tool.input_schema["required"]);
    assert!(
        tool.input_schema["properties"]["connection"]["description"]
            .as_str()
            .unwrap_or_default()
            .contains("numeric id")
    );
}

#[test]
fn connection_registry_exposes_save_tools_to_function_calling() {
    let registry = connection_tool_registry(repo());
    let tool_ids = registry
        .list(ToolAdapter::FunctionCalling)
        .into_iter()
        .map(|tool| tool.id)
        .collect::<Vec<_>>();

    assert!(tool_ids.contains(&"connections.list".to_string()));
    assert!(tool_ids.contains(&"connections.show".to_string()));
    assert!(tool_ids.contains(&"connections.list_kinds".to_string()));
    assert!(tool_ids.contains(&"connections.save".to_string()));
    assert!(tool_ids.contains(&"connections.open_session".to_string()));
}

#[test]
fn save_is_mutating_and_non_destructive() {
    let registry = connection_tool_registry(repo());
    let tool = registry
        .get("connections.save", ToolAdapter::FunctionCalling)
        .expect("save tool should be exposed");

    assert!(!tool.annotations.read_only);
    assert!(!tool.annotations.destructive);
    assert_eq!(RiskLevel::Medium, tool.annotations.risk);
}

#[test]
fn connection_reference_tools_target_saved_connection_resources() {
    let registry = connection_tool_registry(repo());

    for tool_id in ["connections.show", "connections.test"] {
        let tool = registry
            .get_runtime(tool_id, ToolAdapter::FunctionCalling)
            .expect("connection reference tool should be registered");
        assert!(tool.target.required, "{tool_id} should require target");
        assert_eq!(
            vec![ResourceCapability::ManageConnection],
            tool.target.required_capabilities,
            "{tool_id} should target saved connection resources"
        );
    }

    let open_session = registry
        .get_runtime("connections.open_session", ToolAdapter::FunctionCalling)
        .expect("open_session tool should be registered");
    assert!(open_session.target.required);
    assert_eq!(
        vec![ResourceCapability::OpenSession],
        open_session.target.required_capabilities
    );
}

#[test]
fn save_notifies_created_connection_after_create() {
    let repo = repo();
    let notifier = Arc::new(RecordingSaveNotifier::default());
    let registry = connection_tool_registry_with_workspaces_and_hooks(
        repo,
        None,
        ConnectionToolHooks::default()
            .with_save_notifier(Some(notifier.clone() as Arc<dyn ConnectionSaveNotifier>)),
    );

    let id = create_connection(
        &registry,
        json!({
            "kind": "database",
            "database_type": "MySQL",
            "values": {
                "name": "created mysql",
                "host": "10.0.1.20",
                "username": "app"
            }
        }),
    );

    let events = notifier.drain();
    assert_eq!(1, events.len());
    match &events[0] {
        ConnectionSaveEvent::Created(connection) => {
            assert_eq!(Some(id), connection.id);
            assert_eq!("created mysql", connection.name);
        }
        other => panic!("unexpected save event: {other:?}"),
    }
}

#[test]
fn save_notifies_updated_connection_after_update() {
    let repo = repo();
    let notifier = Arc::new(RecordingSaveNotifier::default());
    let registry = connection_tool_registry_with_workspaces_and_hooks(
        repo,
        None,
        ConnectionToolHooks::default()
            .with_save_notifier(Some(notifier.clone() as Arc<dyn ConnectionSaveNotifier>)),
    );
    let id = create_connection(
        &registry,
        json!({
            "kind": "database",
            "database_type": "MySQL",
            "values": {
                "name": "prod mysql",
                "host": "10.0.1.20",
                "username": "app"
            }
        }),
    );
    notifier.drain();

    let result = futures::executor::block_on(registry.call(
        "connections.save",
        json!({
            "id": id,
            "patch": { "name": "prod mysql renamed" }
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("save update should run");

    assert_eq!(json!(true), result.structured_content["ok"]);
    let events = notifier.drain();
    assert_eq!(1, events.len());
    match &events[0] {
        ConnectionSaveEvent::Updated(connection) => {
            assert_eq!(Some(id), connection.id);
            assert_eq!("prod mysql renamed", connection.name);
        }
        other => panic!("unexpected save event: {other:?}"),
    }
}

#[test]
fn list_saved_connections_returns_redacted_summaries() {
    let repo = repo();
    let registry = connection_tool_registry(repo);
    create_connection(
        &registry,
        json!({
            "kind": "database",
            "database_type": "MySQL",
            "values": {
                "name": "prod mysql",
                "host": "10.0.1.20",
                "username": "app",
                "password": "secret"
            }
        }),
    );

    let result = futures::executor::block_on(registry.call(
        "connections.list",
        json!({ "include_summary": true }),
        ToolContext::for_adapter(ToolAdapter::FunctionCalling),
    ))
    .expect("list saved connections should run");

    assert_eq!(
        "prod mysql",
        result.structured_content["connections"][0]["name"]
    );
    assert_eq!(
        "<redacted>",
        result.structured_content["connections"][0]["summary"]["password"]
    );
}

#[test]
fn show_saved_connection_supports_id_and_name() {
    let repo = repo();
    let registry = connection_tool_registry(repo);
    let id = create_connection(
        &registry,
        json!({
            "kind": "ssh_sftp",
            "values": {
                "name": "prod ssh",
                "host": "10.0.1.30",
                "username": "deploy"
            }
        }),
    );

    let by_id = futures::executor::block_on(registry.call(
        "connections.show",
        json!({ "connection": id.to_string() }),
        ToolContext::for_adapter(ToolAdapter::FunctionCalling),
    ))
    .expect("show by id should run");
    let by_name = futures::executor::block_on(registry.call(
        "connections.show",
        json!({ "connection": "prod ssh" }),
        ToolContext::for_adapter(ToolAdapter::FunctionCalling),
    ))
    .expect("show by name should run");

    assert_eq!(id, by_id.structured_content["connection"]["id"]);
    assert_eq!(by_id.structured_content, by_name.structured_content);
}

#[test]
fn list_kinds_includes_all_creatable_connection_types() {
    let registry = connection_tool_registry(repo());

    let result = futures::executor::block_on(registry.call(
        "connections.list_kinds",
        json!({}),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("list kinds should run");

    let kinds = result.structured_content["kinds"]
        .as_array()
        .expect("kinds should be an array")
        .iter()
        .filter_map(|kind| kind["kind"].as_str())
        .collect::<Vec<_>>();

    assert_eq!(creatable_kinds(), kinds);
}

#[test]
fn list_kinds_includes_ipc_database_types_from_registry() {
    let registry = IpcDriverRegistry::from_drivers(vec![driver_manifest("demo", "Demo")]);

    let output = super::schema::list_kinds_with_registry(&registry);
    let database_types = output["kinds"][0]["database_types"]
        .as_array()
        .expect("database types should be an array")
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();

    assert!(database_types.contains(&"MySQL"));
    assert!(database_types.contains(&"External:demo"));
}

#[test]
fn get_schema_supports_all_creatable_connection_types() {
    let registry = connection_tool_registry(repo());

    for kind in creatable_kinds() {
        let result = futures::executor::block_on(registry.call(
            "connections.get_schema",
            json!({ "kind": kind }),
            ToolContext::for_adapter(ToolAdapter::Mcp),
        ))
        .expect("schema tool should run");

        assert_eq!(kind, result.structured_content["kind"]);
        assert_eq!(json!(1), result.structured_content["schema_version"]);
        assert!(
            result.structured_content["fields"]
                .as_array()
                .is_some_and(|fields| !fields.is_empty())
        );
    }
}

#[test]
fn database_schema_uses_database_specific_connection_form() {
    let registry = connection_tool_registry(repo());

    let result = futures::executor::block_on(registry.call(
        "connections.get_schema",
        json!({ "kind": "database", "database_type": "PostgreSQL" }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("schema tool should run");

    let fields = result.structured_content["fields"]
        .as_array()
        .expect("fields should be an array");
    let field_names = fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect::<Vec<_>>();

    assert!(field_names.contains(&"connect_timeout"));
    assert!(field_names.contains(&"ssl_mode"));
    assert_eq!(json!(5432), field_by_name(fields, "port")["default"]);
    assert_eq!(
        json!(["disable", "prefer", "require"]),
        field_by_name(fields, "ssl_mode")["enum"]
    );
}

#[test]
fn ipc_database_schema_uses_driver_connection_form() {
    let mut driver = driver_manifest("demo", "Demo");
    driver.ui.form = Some(
        serde_json::from_value(json!({
            "schema_version": 1,
            "forms": [{
                "kind": "Connection",
                "title_i18n_key": "connection.title",
                "submit_i18n_key": "save",
                "tabs": [{
                    "id": "general",
                    "label_i18n_key": "general",
                    "fields": [{
                        "id": "workspace",
                        "label_i18n_key": "workspace",
                        "field_type": "Select",
                        "required": false,
                        "default_value": "main",
                        "placeholder_i18n_key": null,
                        "help_i18n_key": null,
                        "options": [
                            { "value": "main", "label_i18n_key": "Main" },
                            { "value": "analytics", "label_i18n_key": "Analytics" }
                        ],
                        "options_source": null,
                        "visible_when": [],
                        "default_when": [],
                        "disabled_when_editing": false,
                        "rows": null,
                        "min": null,
                        "max": null
                    }]
                }]
            }]
        }))
        .expect("driver form should parse"),
    );
    let registry = IpcDriverRegistry::from_drivers(vec![driver]);

    let result = super::schema::schema_for_with_registry(
        json!({ "kind": "database", "database_type": "External:demo" }),
        &registry,
    )
    .expect("schema should build");
    let fields = result["fields"]
        .as_array()
        .expect("fields should be an array");

    assert_eq!("workspace", field_by_name(fields, "workspace")["name"]);
    assert_eq!(json!("main"), field_by_name(fields, "workspace")["default"]);
    assert_eq!(
        json!(["main", "analytics"]),
        field_by_name(fields, "workspace")["enum"]
    );
}

#[cfg(not(feature = "builtin-duckdb"))]
#[test]
fn duckdb_database_schema_uses_ipc_driver_connection_form_when_builtin_disabled() {
    let mut driver = driver_manifest("duckdb", "DuckDB IPC");
    driver.ui.form = Some(
        serde_json::from_value(json!({
            "schema_version": 1,
            "forms": [{
                "kind": "Connection",
                "title_i18n_key": "connection.title",
                "submit_i18n_key": "save",
                "tabs": [{
                    "id": "general",
                    "label_i18n_key": "general",
                    "fields": [{
                        "id": "path",
                        "label_i18n_key": "path",
                        "field_type": "FilePath",
                        "required": true,
                        "default_value": "/tmp/app.duckdb",
                        "placeholder_i18n_key": null,
                        "help_i18n_key": null,
                        "options": [],
                        "options_source": null,
                        "visible_when": [],
                        "default_when": [],
                        "disabled_when_editing": false,
                        "rows": null,
                        "min": null,
                        "max": null
                    }]
                }]
            }]
        }))
        .expect("driver form should parse"),
    );
    let registry = IpcDriverRegistry::from_drivers(vec![driver]);

    let result = super::schema::schema_for_with_registry(
        json!({ "kind": "database", "database_type": "DuckDB" }),
        &registry,
    )
    .expect("schema should build");
    let fields = result["fields"]
        .as_array()
        .expect("fields should be an array");

    assert_eq!(
        json!("/tmp/app.duckdb"),
        field_by_name(fields, "path")["default"]
    );
}

#[test]
fn ipc_database_schema_fills_empty_general_tab_from_default_form() {
    let mut driver = driver_manifest("demo", "Demo Driver");
    driver.ui.default_port = Some(7654);
    driver.ui.form = Some(
        serde_json::from_value(json!({
            "schema_version": 1,
            "forms": [{
                "kind": "Connection",
                "title_i18n_key": "connection.title",
                "submit_i18n_key": "save",
                "tabs": [{
                    "id": "general",
                    "label_i18n_key": "general",
                    "fields": []
                }]
            }]
        }))
        .expect("driver form should parse"),
    );
    let registry = IpcDriverRegistry::from_drivers(vec![driver]);

    let result = super::schema::schema_for_with_registry(
        json!({ "kind": "database", "database_type": "External:demo" }),
        &registry,
    )
    .expect("schema should build");
    let fields = result["fields"]
        .as_array()
        .expect("fields should be an array");

    assert_eq!(
        json!("Demo Driver"),
        field_by_name(fields, "name")["default"]
    );
    assert_eq!(json!(7654), field_by_name(fields, "port")["default"]);
}

#[test]
fn ipc_database_schema_uses_default_form_when_driver_has_no_form() {
    let mut driver = driver_manifest("demo", "Demo Driver");
    driver.ui.default_port = Some(7654);
    let registry = IpcDriverRegistry::from_drivers(vec![driver]);

    let result = super::schema::schema_for_with_registry(
        json!({ "kind": "database", "database_type": "External:demo" }),
        &registry,
    )
    .expect("schema should build");
    let fields = result["fields"]
        .as_array()
        .expect("fields should be an array");

    assert_eq!(
        json!("Demo Driver"),
        field_by_name(fields, "name")["default"]
    );
    assert_eq!(json!(7654), field_by_name(fields, "port")["default"]);
    assert_eq!(json!(true), field_by_name(fields, "password")["secret"]);
}

#[test]
fn create_database_connection_persists_mysql_config() {
    let repo = repo();
    let registry = connection_tool_registry(repo.clone());

    let result = futures::executor::block_on(registry.call(
        "connections.save",
        json!({
            "kind": "database",
            "database_type": "MySQL",
            "values": {
                "name": "prod mysql",
                "host": "10.0.1.20",
                "port": 3306,
                "username": "app",
                "password": "secret",
                "database": "ai_app"
            }
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("save tool should run");

    assert_eq!(json!(true), result.structured_content["ok"]);
    assert_eq!("database", result.structured_content["connection"]["kind"]);
    let id = result.structured_content["connection"]["id"]
        .as_i64()
        .expect("created id should be returned");
    let stored = repo
        .get(id)
        .expect("connection should be readable")
        .expect("connection should exist");
    let db = stored.to_db_connection().expect("params should parse");

    assert_eq!("prod mysql", stored.name);
    assert_eq!(DatabaseType::MySQL, db.database_type);
    assert_eq!("10.0.1.20", db.host);
    assert_eq!(3306, db.port);
    assert_eq!("app", db.username);
    assert_eq!("secret", db.password);
    assert_eq!(Some("ai_app"), db.database.as_deref());
    assert_eq!(
        "<redacted>",
        result.structured_content["connection"]["summary"]["password"]
    );
}

#[test]
fn validate_reports_missing_required_fields_without_writing() {
    let repo = repo();
    let registry = connection_tool_registry(repo.clone());

    let result = futures::executor::block_on(registry.call(
        "connections.validate",
        json!({
            "kind": "database",
            "database_type": "MySQL",
            "values": {
                "host": "10.0.1.20"
            }
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("validate tool should run");

    assert_eq!(json!(false), result.structured_content["ok"]);
    assert_eq!(json!(false), result.structured_content["can_apply"]);
    assert_eq!(json!(0), repo.count().expect("count should run"));
    assert_eq!(
        json!(["name", "username"]),
        result.structured_content["missing_required"]
    );
}

#[test]
fn validate_rejects_invalid_numeric_fields_without_writing() {
    let repo = repo();
    let registry = connection_tool_registry(repo.clone());

    let result = futures::executor::block_on(registry.call(
        "connections.save",
        json!({
            "kind": "database",
            "database_type": "MySQL",
            "values": {
                "name": "bad mysql",
                "host": "10.0.1.20",
                "port": 70000,
                "username": "app"
            }
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("save tool should return validation output");

    assert_eq!(json!(false), result.structured_content["ok"]);
    assert_eq!(json!(false), result.structured_content["can_apply"]);
    assert_eq!(json!(0), repo.count().expect("count should run"));
    assert_eq!(
        json!([{
            "field": "port",
            "message": "must be an integer between 0 and 65535"
        }]),
        result.structured_content["invalid_fields"]
    );
}

pub(super) fn create_connection(
    registry: &tool_runtime::ToolRegistry,
    input: serde_json::Value,
) -> i64 {
    let result = futures::executor::block_on(registry.call(
        "connections.save",
        input,
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("save tool should run");

    assert_eq!(json!(true), result.structured_content["ok"]);
    result.structured_content["connection"]["id"]
        .as_i64()
        .expect("created id should be returned")
}

fn creatable_kinds() -> Vec<&'static str> {
    vec![
        "database",
        "ssh_sftp",
        "redis",
        "mongodb",
        "serial",
        "port_forwarding",
        "rdp",
        "vnc",
    ]
}

fn field_by_name<'a>(fields: &'a [serde_json::Value], name: &str) -> &'a serde_json::Value {
    fields
        .iter()
        .find(|field| field["name"] == name)
        .expect("field should exist")
}

fn driver_manifest(id: &str, name: &str) -> IpcDriverManifest {
    IpcDriverManifest {
        id: id.to_string(),
        name: name.to_string(),
        api: "database".into(),
        category: None,
        description: String::new(),
        version: String::new(),
        compatibility: serde_json::Value::Null,
        entry: IpcDriverEntry {
            command: "./driver".to_string(),
            commands: Default::default(),
            args: Vec::new(),
            working_dir: None,
            env_from_config: Default::default(),
        },
        transport: IpcDriverTransport::local_socket(format!("{id}.sock")),
        dialect: Default::default(),
        capabilities: None,
        connection: Default::default(),
        methods: Vec::new(),
        ui: Default::default(),
        manifest_dir: PathBuf::from("."),
    }
}

pub(super) fn repo() -> Arc<ConnectionRepository> {
    let conn = SqliteConnection::open_with_pool_size(":memory:", 1).expect("sqlite should open");
    conn.with_connection(|db| {
        run_migrations(db)?;
        Ok(())
    })
    .expect("migrations should run");
    Arc::new(ConnectionRepository::new(conn))
}
