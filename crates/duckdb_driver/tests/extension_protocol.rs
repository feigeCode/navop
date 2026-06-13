use std::time::Duration;

use base64::Engine;
use extension_host::{FramedTransport, JsonRpcClient, RequestOptions};
use extension_protocol::conn::{ConnOpenParams, ConnOpenResult};
use extension_protocol::lifecycle::{InitParams, InitResult};
use extension_protocol::method;
use extension_protocol::query::{CursorFetchParams, CursorFetchResult};
use interprocess::local_socket::{
    GenericNamespaced, ToNsName, tokio::Stream, traits::tokio::Stream as _,
};

async fn connect_socket(socket_name: &str) -> Stream {
    let name = socket_name
        .to_ns_name::<GenericNamespaced>()
        .expect("valid socket name");
    for _ in 0..50 {
        match Stream::connect(name.clone()).await {
            Ok(stream) => return stream,
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    panic!("server did not accept socket connection");
}

#[tokio::test]
async fn duckdb_driver_supports_extension_database_protocol() {
    let socket_name = format!("onetcli-duckdb-ext-{}.sock", uuid::Uuid::new_v4());
    let server_socket = socket_name.clone();
    let server =
        tokio::spawn(async move { duckdb_driver::server::run_as_listener(&server_socket).await });

    let stream = connect_socket(&socket_name).await;
    let (reader, writer) = tokio::io::split(stream);
    let client = JsonRpcClient::start(FramedTransport::new(reader, writer));
    let handle = client.handle();
    let timeout = RequestOptions::default().with_timeout(Duration::from_secs(2));

    let init: InitResult = handle
        .call(
            method::INIT,
            serde_json::to_value(InitParams::new("onetcli-test", "duckdb-test")).unwrap(),
            timeout.clone(),
        )
        .await
        .expect("init should succeed");
    assert!(init.declares_method(method::CONN_OPEN));
    assert!(init.declares_method(method::QUERY_START));
    assert!(init.declares_method(method::TX_BEGIN));
    assert!(init.declares_method(method::DATA_EXPORT));
    assert!(init.declares_method(method::DATA_IMPORT_BEGIN));
    assert!(init.declares_method(method::DATA_IMPORT_CHUNK));
    assert!(init.declares_method(method::DATA_IMPORT_COMMIT));
    assert!(init.declares_method(method::DATA_IMPORT_ABORT));
    assert!(init.declares_method(method::STREAM_READ));
    assert!(init.has_feature(extension_protocol::lifecycle::Capability::TRANSACTIONS));
    assert!(!init.has_feature(extension_protocol::lifecycle::Capability::NESTED_TRANSACTIONS));
    assert!(init.has_feature(extension_protocol::lifecycle::Capability::DATA_PIPE));

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("protocol.db");
    let open: ConnOpenResult = handle
        .call(
            method::CONN_OPEN,
            serde_json::to_value(ConnOpenParams::new(
                "duckdb",
                serde_json::json!({ "host": db_path.to_string_lossy() }),
            ))
            .unwrap(),
            timeout.clone(),
        )
        .await
        .expect("conn/open should succeed");

    let _: serde_json::Value = handle
        .call(
            method::EXEC_RUN,
            serde_json::json!({
                "conn_id": open.conn_id,
                "sql": "CREATE TABLE t(id INTEGER, name TEXT)"
            }),
            timeout.clone(),
        )
        .await
        .expect("exec/run create should succeed");
    let _: serde_json::Value = handle
        .call(
            method::EXEC_RUN,
            serde_json::json!({
                "conn_id": open.conn_id,
                "sql": "INSERT INTO t VALUES (?, ?)",
                "params": [
                    { "type": "i64", "value": 1 },
                    { "type": "text", "value": "Ada" }
                ]
            }),
            timeout.clone(),
        )
        .await
        .expect("exec/run bound insert should succeed");
    let _: serde_json::Value = handle
        .call(
            method::EXEC_RUN,
            serde_json::json!({
                "conn_id": open.conn_id,
                "sql": "INSERT INTO t VALUES (?, ?)",
                "params": [
                    { "type": "i64", "value": 2 },
                    { "type": "text", "value": "Linus" }
                ]
            }),
            timeout.clone(),
        )
        .await
        .expect("exec/run second bound insert should succeed");

    let started: extension_protocol::query::QueryStartResult = handle
        .call(
            method::QUERY_START,
            serde_json::json!({
                "conn_id": open.conn_id,
                "sql": "SELECT id, name FROM t WHERE id >= ? ORDER BY id",
                "params": [{ "type": "i64", "value": 1 }]
            }),
            timeout.clone(),
        )
        .await
        .expect("query/start with bound params should succeed");
    assert_eq!(started.columns.len(), 2);

    let fetched: CursorFetchResult = handle
        .call(
            method::CURSOR_FETCH,
            serde_json::to_value(CursorFetchParams {
                cursor_id: started.cursor_id.clone(),
                n: Some(10),
                next_token: None,
            })
            .unwrap(),
            timeout.clone(),
        )
        .await
        .expect("cursor/fetch should succeed");
    assert!(fetched.done);
    assert_eq!(fetched.rows.len(), 2);

    let _: serde_json::Value = handle
        .call(
            method::DATA_EXPORT,
            serde_json::json!({
                "conn_id": open.conn_id,
                "sql": "SELECT id, name FROM t ORDER BY id",
                "format": "ndjson",
                "stream_id": "protocol-export-1"
            }),
            timeout.clone(),
        )
        .await
        .expect("data/export should succeed");
    let exported: extension_protocol::data::StreamReadResult = handle
        .call(
            method::STREAM_READ,
            serde_json::json!({ "stream_id": "protocol-export-1", "max_bytes": 4096 }),
            timeout.clone(),
        )
        .await
        .expect("stream/read should route by stream_id without conn_id");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(exported.data.as_bytes())
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains(r#""name":"Ada""#));

    let _: serde_json::Value = handle
        .call(
            method::STREAM_CLOSE,
            serde_json::json!({ "stream_id": "protocol-export-1" }),
            timeout.clone(),
        )
        .await
        .expect("stream/close should succeed");

    let _: serde_json::Value = handle
        .call(
            method::EXEC_RUN,
            serde_json::json!({
                "conn_id": open.conn_id,
                "sql": "CREATE TABLE imported_users(id INTEGER, name TEXT)"
            }),
            timeout.clone(),
        )
        .await
        .expect("create import target should succeed");
    let import_begin: extension_protocol::data::ImportBeginResult = handle
        .call(
            method::DATA_IMPORT_BEGIN,
            serde_json::json!({
                "conn_id": open.conn_id,
                "table": "imported_users",
                "format": "json",
                "columns": ["id", "name"]
            }),
            timeout.clone(),
        )
        .await
        .expect("data/import_begin should succeed");
    let import_id = import_begin.import_id.clone();
    let import_chunk: extension_protocol::data::ImportChunkResult = handle
        .call(
            method::DATA_IMPORT_CHUNK,
            serde_json::json!({
                "import_id": import_id,
                "rows": [
                    [
                        { "type": "i64", "value": 10 },
                        { "type": "text", "value": "Grace" }
                    ]
                ]
            }),
            timeout.clone(),
        )
        .await
        .expect("data/import_chunk should route by import_id without conn_id");
    assert_eq!(import_chunk.inserted, 1);
    let import_commit: extension_protocol::data::ImportCommitResult = handle
        .call(
            method::DATA_IMPORT_COMMIT,
            serde_json::json!({ "import_id": import_id }),
            timeout.clone(),
        )
        .await
        .expect("data/import_commit should route by import_id without conn_id");
    assert_eq!(import_commit.inserted, 1);

    let imported: extension_protocol::query::QueryStartResult = handle
        .call(
            method::QUERY_START,
            serde_json::json!({
                "conn_id": open.conn_id,
                "sql": "SELECT id, name FROM imported_users ORDER BY id"
            }),
            timeout.clone(),
        )
        .await
        .expect("query imported rows should succeed");
    let imported_rows: CursorFetchResult = handle
        .call(
            method::CURSOR_FETCH,
            serde_json::to_value(CursorFetchParams {
                cursor_id: imported.cursor_id.clone(),
                n: Some(10),
                next_token: None,
            })
            .unwrap(),
            timeout.clone(),
        )
        .await
        .expect("fetch imported rows should succeed");
    assert_eq!(
        imported_rows.rows[0][0],
        extension_protocol::row::CellValue::I64 { value: 10 }
    );
    assert_eq!(
        imported_rows.rows[0][1],
        extension_protocol::row::CellValue::Text {
            value: "Grace".into()
        }
    );
    let _: serde_json::Value = handle
        .call(
            method::CURSOR_CLOSE,
            serde_json::json!({ "cursor_id": imported.cursor_id }),
            timeout.clone(),
        )
        .await
        .expect("cursor/close imported should succeed");

    let _: serde_json::Value = handle
        .call(
            method::CURSOR_CLOSE,
            serde_json::json!({ "cursor_id": started.cursor_id }),
            timeout.clone(),
        )
        .await
        .expect("cursor/close should succeed");
    let _: serde_json::Value = handle
        .call(
            method::CONN_CLOSE,
            serde_json::json!({ "conn_id": open.conn_id }),
            timeout,
        )
        .await
        .expect("conn/close should succeed");

    client.shutdown().await;
    server.abort();
}
