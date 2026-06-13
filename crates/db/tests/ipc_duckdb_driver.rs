use std::{collections::HashMap, path::PathBuf};

use base64::Engine;
use db::{
    ColumnDefinition, DatabasePlugin, DbConnection, ExecOptions, ExportConfig, ImportConfig,
    IndexDefinition, SqlResult, TableDesign,
    ipc::{
        EXTERNAL_DRIVER_ID_PARAM, ExternalDatabasePlugin, ExternalDbConnection, IpcDriverEntry,
        IpcDriverManifest, IpcDriverRegistry, IpcDriverTransport,
    },
};
use extension_protocol::method as wire_method;
use one_core::storage::{DatabaseType, DbConnectionConfig};

fn driver_binary() -> PathBuf {
    // current_exe: target/debug/deps/<test>-<hash>
    // parent:       target/debug/deps/
    // parent:       target/debug/
    let target_dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let name = if cfg!(windows) {
        "duckdb_driver.exe"
    } else {
        "duckdb_driver"
    };
    target_dir.join(name)
}

fn make_driver(binary: &std::path::Path, manifest_dir: &std::path::Path) -> IpcDriverManifest {
    IpcDriverManifest {
        id: "duckdb".into(),
        name: "DuckDB".into(),
        description: String::new(),
        version: String::new(),
        entry: IpcDriverEntry {
            command: binary.to_string_lossy().into_owned(),
            args: Vec::new(),
            working_dir: None,
        },
        dialect: Default::default(),
        capabilities: None,
        methods: vec![
            wire_method::CONN_TEST.into(),
            wire_method::SCHEMA_DATABASES.into(),
            wire_method::SCHEMA_SCHEMAS.into(),
            wire_method::SCHEMA_OBJECTS.into(),
            wire_method::SCHEMA_COLUMNS.into(),
            wire_method::SCHEMA_VIEWS.into(),
            wire_method::SCHEMA_INDEXES.into(),
            wire_method::SCHEMA_CHECKS.into(),
            wire_method::EXEC_BATCH.into(),
            wire_method::TX_BEGIN.into(),
            wire_method::TX_COMMIT.into(),
            wire_method::TX_ROLLBACK.into(),
            wire_method::DATA_EXPORT.into(),
            wire_method::DATA_IMPORT_BEGIN.into(),
            wire_method::DATA_IMPORT_CHUNK.into(),
            wire_method::DATA_IMPORT_COMMIT.into(),
            wire_method::DATA_IMPORT_ABORT.into(),
            wire_method::STREAM_READ.into(),
            wire_method::STREAM_CLOSE.into(),
            wire_method::DDL_BUILD_CREATE_TABLE.into(),
            wire_method::DDL_BUILD_ALTER_TABLE.into(),
            wire_method::DDL_BUILD_DROP.into(),
        ],
        ui: Default::default(),
        transport: IpcDriverTransport::local_socket("duckdb-driver.sock"),
        manifest_dir: manifest_dir.to_path_buf(),
    }
}

fn make_config(id: &str, db_path: &std::path::Path) -> DbConnectionConfig {
    DbConnectionConfig {
        id: id.into(),
        name: "DuckDB Test".into(),
        database_type: DatabaseType::External,
        host: db_path.to_string_lossy().into_owned(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: Some("main".into()),
        service_name: None,
        sid: None,
        workspace_id: None,
        extra_params: HashMap::new(),
    }
}

fn make_external_config(id: &str, db_path: &std::path::Path) -> DbConnectionConfig {
    let mut config = make_config(id, db_path);
    config
        .extra_params
        .insert(EXTERNAL_DRIVER_ID_PARAM.into(), "duckdb".into());
    config
}

fn skip_if_missing_binary(binary: &std::path::Path) -> bool {
    if binary.exists() {
        return false;
    }
    eprintln!(
        "SKIP: duckdb_driver binary not found at {:?}\n\
         Build it first: cargo build -p duckdb_driver",
        binary
    );
    true
}

#[tokio::test]
async fn duckdb_driver_ipc_full_integration() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("test.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_config("duckdb-test", &db_path);

    // ---- connect ----
    let mut conn = ExternalDbConnection::new(config.clone(), driver);
    conn.connect().await.expect("connect");
    let plugin = ExternalDatabasePlugin::new();

    // ---- ping ----
    conn.ping().await.expect("ping");

    // ---- current_database ----
    assert_eq!(conn.current_database().await.unwrap(), Some("main".into()));

    // ---- SELECT literal ----
    let result = conn.query("SELECT 1 AS val").await.unwrap();
    match result {
        SqlResult::Query(q) => {
            assert_eq!(q.columns, vec!["val"]);
            assert_eq!(q.rows[0][0].as_deref(), Some("1"));
        }
        other => panic!("expected query result, got {other:?}"),
    }

    // ---- DDL + DML ----
    conn.query("CREATE TABLE t (id INTEGER, name VARCHAR)")
        .await
        .unwrap();
    conn.query("INSERT INTO t VALUES (1, 'alice'), (2, 'bob')")
        .await
        .unwrap();

    let result = conn.query("SELECT * FROM t ORDER BY id").await.unwrap();
    match result {
        SqlResult::Query(q) => {
            assert_eq!(q.rows.len(), 2);
            assert_eq!(q.rows[0][0].as_deref(), Some("1"));
            assert_eq!(q.rows[0][1].as_deref(), Some("alice"));
            assert_eq!(q.rows[1][0].as_deref(), Some("2"));
            assert_eq!(q.rows[1][1].as_deref(), Some("bob"));
        }
        other => panic!("expected query result, got {other:?}"),
    }

    // ---- schema/objects (tables) ----
    let tables = plugin.list_tables(&conn, "main", None).await.unwrap();
    let names: Vec<&str> = tables.iter().map(|table| table.name.as_str()).collect();
    assert!(names.contains(&"t"), "expected 't' in tables: {names:?}");

    // ---- schema/columns ----
    let cols = plugin.list_columns(&conn, "main", None, "t").await.unwrap();
    assert_eq!(cols.len(), 2);
    assert_eq!(cols[0].name, "id");
    assert_eq!(cols[1].name, "name");

    // ---- disconnect ----
    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn external_plugin_create_connection_returns_connected_connection() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("plugin-create.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-plugin-create", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));

    let mut conn = plugin.create_connection(config).await.expect("create");

    conn.ping().await.expect("ping after plugin create");
    let result = conn.query("SELECT 42 AS val").await.unwrap();
    match result {
        SqlResult::Query(q) => {
            assert_eq!(q.columns, vec!["val"]);
            assert_eq!(q.rows[0][0].as_deref(), Some("42"));
        }
        other => panic!("expected query result, got {other:?}"),
    }

    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn external_execute_respects_max_rows_for_query_results() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("max-rows.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-max-rows", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));
    let mut conn = plugin.create_connection(config).await.expect("create");
    let options = ExecOptions {
        max_rows: Some(3),
        ..ExecOptions::default()
    };

    let results = conn
        .execute(
            &plugin,
            "SELECT range AS n FROM range(5) ORDER BY n",
            options,
        )
        .await
        .expect("execute query");

    assert_eq!(results.len(), 1);
    match &results[0] {
        SqlResult::Query(query) => {
            assert_eq!(query.rows.len(), 3);
            assert_eq!(query.rows[2][0].as_deref(), Some("2"));
        }
        other => panic!("expected query result, got {other:?}"),
    }

    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn external_execute_transactional_batch_rolls_back_on_error() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("transactional-batch.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-transactional-batch", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));
    let mut conn = plugin.create_connection(config).await.expect("create");
    let options = ExecOptions {
        transactional: true,
        stop_on_error: true,
        ..ExecOptions::default()
    };

    let results = conn
        .execute(
            &plugin,
            "
            CREATE TABLE batch_tx (id INTEGER PRIMARY KEY);
            INSERT INTO batch_tx VALUES (1);
            INSERT INTO batch_tx VALUES (1);
            ",
            options,
        )
        .await
        .expect("execute transactional batch");

    assert!(
        results.iter().any(SqlResult::is_error),
        "duplicate key should be reported as statement error: {results:?}"
    );
    let tables = plugin
        .list_tables(&*conn, "main", None)
        .await
        .expect("list tables after rollback");
    let names: Vec<&str> = tables.iter().map(|table| table.name.as_str()).collect();
    assert!(
        !names.contains(&"batch_tx"),
        "transactional batch should roll back table creation: {names:?}"
    );

    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn external_tx_methods_rollback_wire_transaction() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("wire-tx.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-wire-tx", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));
    let mut conn = plugin.create_connection(config).await.expect("create");

    let begin = conn
        .driver_request_value(wire_method::TX_BEGIN, serde_json::json!({}))
        .await
        .expect("tx/begin");
    let tx_id = begin["tx_id"].as_str().expect("tx_id").to_string();
    conn.driver_request_value(
        wire_method::EXEC_RUN,
        serde_json::json!({
            "tx_id": tx_id,
            "sql": "CREATE TABLE wire_tx_users (id INTEGER)"
        }),
    )
    .await
    .expect("exec/run in tx");
    conn.driver_request_value(
        wire_method::TX_ROLLBACK,
        serde_json::json!({ "tx_id": tx_id }),
    )
    .await
    .expect("tx/rollback");

    let tables = plugin
        .list_tables(&*conn, "main", None)
        .await
        .expect("list tables after wire rollback");
    let names: Vec<&str> = tables.iter().map(|table| table.name.as_str()).collect();
    assert!(
        !names.contains(&"wire_tx_users"),
        "wire tx rollback should discard table creation: {names:?}"
    );

    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn external_data_export_streams_via_driver_request() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("wire-export.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-wire-export", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));
    let mut conn = plugin.create_connection(config).await.expect("create");

    conn.query("CREATE TABLE wire_export_users (id INTEGER, name VARCHAR)")
        .await
        .expect("create export table");
    conn.query("INSERT INTO wire_export_users VALUES (1, 'Ada'), (2, 'Linus')")
        .await
        .expect("insert export rows");
    conn.driver_request_value(
        wire_method::DATA_EXPORT,
        serde_json::json!({
            "sql": "SELECT id, name FROM wire_export_users ORDER BY id",
            "format": "ndjson",
            "stream_id": "wire-export-1"
        }),
    )
    .await
    .expect("data/export");

    let mut bytes = Vec::new();
    loop {
        let chunk = conn
            .driver_request_value(
                wire_method::STREAM_READ,
                serde_json::json!({ "stream_id": "wire-export-1", "max_bytes": 7 }),
            )
            .await
            .expect("stream/read");
        let data = chunk["data"].as_str().expect("base64 data");
        bytes.extend(
            base64::engine::general_purpose::STANDARD
                .decode(data.as_bytes())
                .expect("valid base64"),
        );
        if chunk["done"].as_bool().unwrap_or(false) {
            break;
        }
    }

    let text = String::from_utf8(bytes).expect("utf-8 export");
    assert!(text.contains(r#""name":"Ada""#), "{text}");
    assert!(text.contains(r#""name":"Linus""#), "{text}");
    conn.driver_request_value(
        wire_method::STREAM_CLOSE,
        serde_json::json!({ "stream_id": "wire-export-1" }),
    )
    .await
    .expect("stream/close");

    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn external_data_import_writes_rows_via_driver_request() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("wire-import.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-wire-import", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));
    let mut conn = plugin.create_connection(config).await.expect("create");

    conn.query("CREATE TABLE wire_import_users (id INTEGER, name VARCHAR)")
        .await
        .expect("create import table");
    let begin = conn
        .driver_request_value(
            wire_method::DATA_IMPORT_BEGIN,
            serde_json::json!({
                "table": "wire_import_users",
                "format": "json",
                "columns": ["id", "name"]
            }),
        )
        .await
        .expect("data/import_begin");
    let import_id = begin["import_id"].as_str().expect("import_id").to_string();
    let chunk = conn
        .driver_request_value(
            wire_method::DATA_IMPORT_CHUNK,
            serde_json::json!({
                "import_id": import_id.clone(),
                "rows": [
                    [
                        { "type": "i64", "value": 1 },
                        { "type": "text", "value": "Ada" }
                    ],
                    [
                        { "type": "i64", "value": 2 },
                        { "type": "text", "value": "Linus" }
                    ]
                ]
            }),
        )
        .await
        .expect("data/import_chunk");
    assert_eq!(chunk["inserted"], 2);

    let commit = conn
        .driver_request_value(
            wire_method::DATA_IMPORT_COMMIT,
            serde_json::json!({ "import_id": import_id }),
        )
        .await
        .expect("data/import_commit");
    assert_eq!(commit["inserted"], 2);

    let result = conn
        .query("SELECT id, name FROM wire_import_users ORDER BY id")
        .await
        .expect("query imported rows");
    match result {
        SqlResult::Query(query) => {
            assert_eq!(query.rows.len(), 2);
            assert_eq!(query.rows[0][0].as_deref(), Some("1"));
            assert_eq!(query.rows[0][1].as_deref(), Some("Ada"));
            assert_eq!(query.rows[1][0].as_deref(), Some("2"));
            assert_eq!(query.rows[1][1].as_deref(), Some("Linus"));
        }
        other => panic!("expected query result, got {other:?}"),
    }

    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn external_plugin_import_data_uses_driver_data_import() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("plugin-import.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-plugin-import", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));
    let mut conn = plugin.create_connection(config).await.expect("create");

    conn.query("CREATE TABLE plugin_import_users (id INTEGER, name VARCHAR)")
        .await
        .expect("create import table");
    let import_config = ImportConfig {
        format: db::DataFormat::Json,
        database: "main".into(),
        table: Some("plugin_import_users".into()),
        ..ImportConfig::default()
    };
    let result = plugin
        .import_data(
            &*conn,
            &import_config,
            r#"[{"id":1,"name":"Ada"},{"id":2,"name":"Linus"}]"#,
        )
        .await
        .expect("external plugin import_data");

    assert!(result.success, "import should succeed: {result:?}");
    assert_eq!(result.rows_imported, 2);
    let rows = conn
        .query("SELECT id, name FROM plugin_import_users ORDER BY id")
        .await
        .expect("query imported rows");
    match rows {
        SqlResult::Query(query) => {
            assert_eq!(query.rows[0][0].as_deref(), Some("1"));
            assert_eq!(query.rows[0][1].as_deref(), Some("Ada"));
            assert_eq!(query.rows[1][0].as_deref(), Some("2"));
            assert_eq!(query.rows[1][1].as_deref(), Some("Linus"));
        }
        other => panic!("expected query result, got {other:?}"),
    }

    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn external_plugin_export_data_uses_driver_data_export() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("plugin-export.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-plugin-export", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));
    let mut conn = plugin.create_connection(config).await.expect("create");

    conn.query("CREATE TABLE plugin_export_users (id INTEGER, name VARCHAR)")
        .await
        .expect("create export table");
    conn.query("INSERT INTO plugin_export_users VALUES (1, 'Ada'), (2, 'Linus')")
        .await
        .expect("insert export rows");
    let export_config = ExportConfig {
        format: db::DataFormat::Json,
        database: "main".into(),
        tables: vec!["plugin_export_users".into()],
        ..ExportConfig::default()
    };
    let result = plugin
        .export_data(&*conn, &export_config)
        .await
        .expect("external plugin export_data");

    assert!(result.success, "export should succeed: {result:?}");
    assert_eq!(result.rows_exported, 2);
    let value: serde_json::Value =
        serde_json::from_str(&result.output).expect("JSON export output");
    assert_eq!(value[0]["name"], "Ada");
    assert_eq!(value[1]["name"], "Linus");

    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn external_conn_test_runs_without_injected_conn_id() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("main.db");
    let probe_path = temp.path().join("probe.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-conn-test", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));
    let mut conn = plugin.create_connection(config).await.expect("create");

    let value = conn
        .driver_request_value(
            wire_method::CONN_TEST,
            serde_json::json!({
                "driver_id": "duckdb",
                "config": { "host": probe_path.to_string_lossy() }
            }),
        )
        .await
        .expect("conn/test");

    assert_eq!(value["ok"], true);
    assert!(value.get("conn_id").is_none());

    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn external_plugin_test_connection_uses_driver_conn_test() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("plugin-test-connection.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-plugin-test-connection", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));

    plugin
        .test_connection(config)
        .await
        .expect("external plugin conn/test");
}

#[tokio::test]
async fn external_plugin_async_create_table_builder_uses_driver_ddl_method() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("plugin-ddl.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-plugin-ddl", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));
    let mut conn = plugin.create_connection(config).await.expect("create");
    let mut design = TableDesign::new("main", "events");
    design.add_column(
        ColumnDefinition::new("id")
            .data_type("INTEGER")
            .nullable(false)
            .primary_key(true),
    );
    design.add_column(ColumnDefinition::new("payload").data_type("VARCHAR"));
    design.add_index(IndexDefinition::new("idx_events_payload").columns(vec!["payload".into()]));

    let sql = plugin
        .build_create_table_sql_async(&*conn, &design)
        .await
        .expect("driver ddl build");

    assert!(sql.contains("CREATE TABLE \"events\""));
    assert!(sql.contains("CREATE INDEX \"idx_events_payload\" ON \"events\""));

    let results = conn
        .execute(&plugin, &sql, ExecOptions::default())
        .await
        .expect("execute generated ddl");
    assert!(
        results.iter().all(|result| !result.is_error()),
        "generated ddl should execute cleanly: {results:?}"
    );
    let tables = plugin.list_tables(&*conn, "main", None).await.unwrap();
    let names: Vec<&str> = tables.iter().map(|table| table.name.as_str()).collect();
    assert!(
        names.contains(&"events"),
        "expected events table: {names:?}"
    );

    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn duckdb_driver_lists_table_checks_via_schema_checks() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("checks.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-checks", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));
    let mut conn = plugin.create_connection(config).await.expect("create");

    conn.query(
        "CREATE TABLE guarded (
            id INTEGER,
            amount INTEGER,
            CONSTRAINT positive_amount CHECK (amount > 0)
        )",
    )
    .await
    .expect("create guarded table");

    let checks = plugin
        .list_table_checks(&*conn, "main", None, "guarded")
        .await
        .expect("list checks");

    assert_eq!(1, checks.len(), "expected one check constraint: {checks:?}");
    assert!(
        !checks[0].name.trim().is_empty(),
        "check constraint name should be populated: {checks:?}"
    );
    assert_eq!("guarded", checks[0].table_name);
    let definition = checks[0].definition.as_deref().unwrap_or_default();
    assert!(
        definition.contains("amount") && definition.contains("> 0"),
        "unexpected check definition: {definition}"
    );

    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn external_plugin_async_alter_builder_renames_column_through_driver() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("plugin-alter-rename.db");
    let driver = make_driver(&binary, temp.path());
    let config = make_external_config("duckdb-plugin-alter-rename", &db_path);
    let plugin =
        ExternalDatabasePlugin::with_registry(IpcDriverRegistry::from_drivers(vec![driver]));
    let mut conn = plugin.create_connection(config).await.expect("create");

    conn.query("CREATE TABLE events (id INTEGER, payload VARCHAR)")
        .await
        .expect("create events table");

    let mut original = TableDesign::new("main", "events");
    original.add_column(ColumnDefinition::new("id").data_type("INTEGER"));
    original.add_column(ColumnDefinition::new("payload").data_type("VARCHAR"));
    let mut current = TableDesign::new("main", "events");
    current.add_column(ColumnDefinition::new("id").data_type("INTEGER"));
    current.add_column(ColumnDefinition::new("body").data_type("VARCHAR"));

    let sql = plugin
        .build_alter_table_sql_with_renames_async(
            &*conn,
            &original,
            &current,
            &[("payload".to_string(), "body".to_string())],
        )
        .await
        .expect("driver alter build");

    assert!(sql.contains("RENAME COLUMN \"payload\" TO \"body\""));
    assert!(!sql.contains("ADD COLUMN \"body\""));
    assert!(!sql.contains("DROP COLUMN \"payload\""));

    let results = conn
        .execute(&plugin, &sql, ExecOptions::default())
        .await
        .expect("execute generated alter");
    assert!(
        results.iter().all(|result| !result.is_error()),
        "generated alter should execute cleanly: {results:?}"
    );
    let columns = plugin
        .list_columns(&*conn, "main", None, "events")
        .await
        .expect("list columns");
    let names: Vec<&str> = columns.iter().map(|column| column.name.as_str()).collect();
    assert!(names.contains(&"body"), "expected body column: {names:?}");
    assert!(
        !names.contains(&"payload"),
        "payload column should have been renamed: {names:?}"
    );

    conn.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn same_duckdb_driver_can_open_multiple_connections_concurrently() {
    let binary = driver_binary();
    if skip_if_missing_binary(&binary) {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let mut handles = Vec::new();
    for idx in 0..3 {
        let driver = make_driver(&binary, temp.path());
        let db_path = temp.path().join(format!("multi-{idx}.db"));
        let config = make_config(&format!("duckdb-multi-{idx}"), &db_path);
        handles.push(tokio::spawn(async move {
            let mut conn = ExternalDbConnection::new(config, driver);
            conn.connect().await.expect("connect");
            let result = conn.query(&format!("SELECT {idx} AS val")).await.unwrap();
            match result {
                SqlResult::Query(q) => {
                    let expected = idx.to_string();
                    assert_eq!(q.columns, vec!["val"]);
                    assert_eq!(q.rows[0][0].as_deref(), Some(expected.as_str()));
                }
                other => panic!("expected query result, got {other:?}"),
            }
            conn.disconnect().await.expect("disconnect");
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }
}
