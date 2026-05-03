use std::{collections::HashMap, path::PathBuf};

use db::{
    ipc::{ExternalDbConnection, IpcDriverEntry, IpcDriverManifest, IpcDriverTransport},
    DbConnection, SqlResult,
};
use interprocess::local_socket::{
    tokio::{prelude::*, Stream},
    GenericNamespaced, ListenerOptions,
};
use one_core::storage::{DatabaseType, DbConnectionConfig};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::oneshot,
};

#[tokio::test]
async fn external_connection_uses_mock_local_socket_driver() {
    let socket_name = format!("onetcli-ipc-test-{}.sock", uuid::Uuid::new_v4());
    let (ready_tx, ready_rx) = oneshot::channel();
    let server_name = socket_name.clone();
    let server = tokio::spawn(async move { run_mock_driver(&server_name, ready_tx).await });

    ready_rx.await.unwrap();

    let driver = IpcDriverManifest {
        id: "mock".into(),
        name: "Mock".into(),
        description: String::new(),
        version: String::new(),
        entry: IpcDriverEntry {
            command: String::new(),
            args: Vec::new(),
            working_dir: None,
        },
        dialect: Default::default(),
        ui: Default::default(),
        transport: IpcDriverTransport::local_socket(socket_name),
        manifest_dir: PathBuf::new(),
    };
    let config = DbConnectionConfig {
        id: "mock".into(),
        name: "Mock".into(),
        database_type: DatabaseType::External,
        host: String::new(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: Some("mockdb".into()),
        service_name: None,
        sid: None,
        workspace_id: None,
        extra_params: HashMap::new(),
    };
    let mut connection = ExternalDbConnection::new(config, driver);

    connection.connect().await.unwrap();
    connection.ping().await.unwrap();
    assert_eq!(
        connection.current_database().await.unwrap(),
        Some("mockdb".into())
    );

    let result = connection.query("select 1").await.unwrap();
    match result {
        SqlResult::Query(query) => assert_eq!(query.rows[0][0].as_deref(), Some("1")),
        other => panic!("unexpected query result: {other:?}"),
    }

    let metadata = json!({"method":"metadata.list_databases","params":{}}).to_string();
    let result = connection
        .query(&format!("/*onetcli-ipc-metadata*/ {metadata}"))
        .await
        .unwrap();
    match result {
        SqlResult::Query(query) => assert_eq!(query.rows[0][0].as_deref(), Some("[\"mockdb\"]")),
        other => panic!("unexpected metadata result: {other:?}"),
    }

    connection.disconnect().await.unwrap();
    server.await.unwrap().unwrap();
}

async fn run_mock_driver(socket_name: &str, ready_tx: oneshot::Sender<()>) -> std::io::Result<()> {
    let name = socket_name.to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_tokio()?;
    let _ = ready_tx.send(());
    let conn = listener.accept().await?;
    handle_conn(conn).await
}

async fn handle_conn(conn: Stream) -> std::io::Result<()> {
    let mut reader = BufReader::new(&conn);
    let mut writer = &conn;

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }

        let request: Value = serde_json::from_str(line.trim_end())?;
        let id = request["id"].as_u64().unwrap();
        let method = request["method"].as_str().unwrap();
        let result = match method {
            "initialize" | "connect" | "ping" | "disconnect" => json!({}),
            "current_database" => json!("mockdb"),
            "query" => json!({
                "type": "Query",
                "sql": request["params"]["sql"].as_str().unwrap(),
                "columns": ["value"],
                "column_meta": [{
                    "name": "value",
                    "db_type": "INT",
                    "field_type": "Integer",
                    "nullable": true
                }],
                "rows": [["1"]],
                "elapsed_ms": 1
            }),
            "metadata.list_databases" => json!(["mockdb"]),
            other => json!({"unknown_method": other}),
        };
        let response = json!({"jsonrpc":"2.0","id":id,"result":result});
        writer.write_all(response.to_string().as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        if method == "disconnect" {
            break;
        }
    }

    Ok(())
}
