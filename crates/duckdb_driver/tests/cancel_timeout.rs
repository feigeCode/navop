use std::time::Duration;

use extension_host::{FramedTransport, HostError, JsonRpcClient, RequestOptions};
use extension_protocol::conn::{ConnOpenParams, ConnOpenResult};
use extension_protocol::lifecycle::{InitParams, InitResult};
use extension_protocol::method;
use extension_protocol::query::{CursorFetchParams, CursorFetchResult, QueryStartResult};
use extension_protocol::row::CellValue;
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
async fn timed_out_fetch_interrupts_duckdb_and_connection_recovers() {
    let socket_name = format!("onetcli-duckdb-cancel-{}.sock", uuid::Uuid::new_v4());
    let server_socket = socket_name.clone();
    let server =
        tokio::spawn(async move { duckdb_driver::server::run_as_listener(&server_socket).await });

    let stream = connect_socket(&socket_name).await;
    let (reader, writer) = tokio::io::split(stream);
    let client = JsonRpcClient::start(FramedTransport::new(reader, writer));
    let handle = client.handle();
    let default_timeout = RequestOptions::default().with_timeout(Duration::from_secs(2));

    let _: InitResult = handle
        .call(
            method::INIT,
            serde_json::to_value(InitParams::new("onetcli-test", "duckdb-cancel-test")).unwrap(),
            default_timeout.clone(),
        )
        .await
        .expect("init should succeed");

    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("cancel.db");
    let open: ConnOpenResult = handle
        .call(
            method::CONN_OPEN,
            serde_json::to_value(ConnOpenParams::new(
                "duckdb",
                serde_json::json!({ "host": db_path.to_string_lossy() }),
            ))
            .unwrap(),
            default_timeout.clone(),
        )
        .await
        .expect("conn/open should succeed");

    let slow: QueryStartResult = handle
        .call(
            method::QUERY_START,
            serde_json::json!({
                "conn_id": open.conn_id,
                "sql": "SELECT sum(random()) AS v FROM range(1000000000)"
            }),
            default_timeout.clone(),
        )
        .await
        .expect("slow cursor should open");

    let err = handle
        .call_raw(
            method::CURSOR_FETCH,
            serde_json::to_value(CursorFetchParams {
                cursor_id: slow.cursor_id,
                n: Some(1),
                next_token: None,
            })
            .unwrap(),
            RequestOptions::default().with_timeout(Duration::from_millis(50)),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, HostError::Timeout { ref method, .. } if method == method::CURSOR_FETCH));

    let quick: QueryStartResult = handle
        .call(
            method::QUERY_START,
            serde_json::json!({ "conn_id": open.conn_id, "sql": "SELECT 42 AS answer" }),
            default_timeout.clone(),
        )
        .await
        .expect("connection should recover after interrupted fetch");
    let fetched: CursorFetchResult = handle
        .call(
            method::CURSOR_FETCH,
            serde_json::to_value(CursorFetchParams {
                cursor_id: quick.cursor_id,
                n: Some(10),
                next_token: None,
            })
            .unwrap(),
            default_timeout,
        )
        .await
        .expect("quick fetch should succeed after timeout cancellation");

    assert_eq!(fetched.rows[0][0], CellValue::I64 { value: 42 });
    client.shutdown().await;
    server.abort();
}
