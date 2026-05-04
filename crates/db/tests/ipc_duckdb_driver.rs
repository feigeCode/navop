use std::{collections::HashMap, path::PathBuf};

use db::{
    DbConnection, SqlResult,
    ipc::{ExternalDbConnection, IpcDriverEntry, IpcDriverManifest, IpcDriverTransport},
};
use one_core::storage::{DatabaseType, DbConnectionConfig};
use serde_json::json;

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

#[tokio::test]
async fn duckdb_driver_ipc_full_integration() {
    let binary = driver_binary();
    if !binary.exists() {
        eprintln!(
            "SKIP: duckdb_driver binary not found at {:?}\n\
             Build it first: cargo build -p duckdb_driver",
            binary
        );
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let socket = format!("onetcli-test-duckdb-{}.sock", uuid::Uuid::new_v4());
    let db_path = temp.path().join("test.db");

    let driver = IpcDriverManifest {
        id: "duckdb".into(),
        name: "DuckDB".into(),
        description: String::new(),
        version: String::new(),
        entry: IpcDriverEntry {
            command: binary.to_string_lossy().into_owned(),
            args: vec![socket.clone()],
            working_dir: None,
        },
        dialect: Default::default(),
        ui: Default::default(),
        transport: IpcDriverTransport::local_socket(socket),
        manifest_dir: temp.path().to_path_buf(),
    };

    let config = DbConnectionConfig {
        id: "duckdb-test".into(),
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
    };

    // ---- connect ----
    let mut conn = ExternalDbConnection::new(config.clone(), driver);
    conn.connect().await.expect("connect");

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
    conn.query("CREATE TABLE t (id INTEGER, name VARCHAR)").await.unwrap();
    conn.query("INSERT INTO t VALUES (1, 'alice'), (2, 'bob')").await.unwrap();

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

    // ---- metadata.list_tables ----
    let metadata = json!({"method":"metadata.list_tables","params":{}}).to_string();
    let result = conn
        .query(&format!("/*onetcli-ipc-metadata*/ {metadata}"))
        .await
        .unwrap();
    match result {
        SqlResult::Query(q) => {
            let cell = q.rows[0][0].as_deref().unwrap();
            let tables: Vec<serde_json::Value> = serde_json::from_str(cell).unwrap();
            let names: Vec<&str> = tables
                .iter()
                .map(|t| t["name"].as_str().unwrap())
                .collect();
            assert!(names.contains(&"t"), "expected 't' in tables: {names:?}");
        }
        other => panic!("expected query result, got {other:?}"),
    }

    // ---- metadata.list_columns ----
    let metadata = json!({"method":"metadata.list_columns","params":{"table":"t"}}).to_string();
    let result = conn
        .query(&format!("/*onetcli-ipc-metadata*/ {metadata}"))
        .await
        .unwrap();
    match result {
        SqlResult::Query(q) => {
            let cell = q.rows[0][0].as_deref().unwrap();
            let cols: Vec<serde_json::Value> = serde_json::from_str(cell).unwrap();
            assert_eq!(cols.len(), 2);
            assert_eq!(cols[0]["name"], "id");
            assert_eq!(cols[1]["name"], "name");
        }
        other => panic!("expected query result, got {other:?}"),
    }

    // ---- disconnect ----
    conn.disconnect().await.expect("disconnect");
}
