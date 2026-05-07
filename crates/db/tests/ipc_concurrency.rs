//! P0-1 并发请求路由的 TDD 测试。
//!
//! 旧实现:`Mutex<Option<JsonRpcClient>>` + `request_value(&mut self)` 把整个
//! stream 串行化,多 caller 必须排队。当 server 设计为「收齐 N 个再乱序回复」
//! 的 rendezvous 场景时,串行 client 永远凑不齐 N 个 → 死锁直至 30s 超时。
//!
//! 新实现拆出 reader task + pending oneshot 路由表后,3 个 caller 应能并发
//! 把 request 推到 server,server 乱序回复也能精确路由回各自的 caller。

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use db::{
    DbConnection, SqlResult,
    ipc::{ExternalDbConnection, IpcDriverEntry, IpcDriverManifest, IpcDriverTransport},
};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions,
    tokio::{Stream, prelude::*},
};
use ipc::{
    IpcRequest, IpcResponse,
    framing::{recv_msg_async, send_msg_async},
};
use one_core::storage::{DatabaseType, DbConnectionConfig};
use serde_json::json;
use tokio::sync::oneshot;

// ───────────────────────── helpers ─────────────────────────

fn make_manifest(socket_name: String) -> IpcDriverManifest {
    IpcDriverManifest {
        id: "concurrency-mock".into(),
        name: "Concurrency Mock".into(),
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
    }
}

fn make_config() -> DbConnectionConfig {
    DbConnectionConfig {
        id: "concurrency-mock".into(),
        name: "Concurrency Mock".into(),
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
    }
}

fn unique_socket(tag: &str) -> String {
    format!("onetcli-conc-{tag}-{}.sock", uuid::Uuid::new_v4())
}

fn make_query_response(id: u64, sql: &str) -> IpcResponse {
    IpcResponse::result(
        id,
        json!({
            "type": "Query",
            "sql": sql,
            "columns": ["echo"],
            "column_meta": [{
                "name": "echo",
                "db_type": "VARCHAR",
                "field_type": "Text",
                "nullable": true
            }],
            "rows": [[sql]],
            "elapsed_ms": 0
        }),
    )
}

fn extract_query_sql(result: SqlResult) -> String {
    match result {
        SqlResult::Query(q) => q.sql,
        other => panic!("unexpected result variant: {other:?}"),
    }
}

async fn handshake(stream: &mut Stream) -> std::io::Result<()> {
    // initialize + connect:client 端会顺序发,这里也顺序回应。
    for _ in 0..2 {
        let req: IpcRequest = recv_msg_async(&mut *stream).await?;
        send_msg_async(
            &mut *stream,
            &IpcResponse::result(req.request_id, json!({})),
        )
        .await?;
    }
    Ok(())
}

// ───────────────────────── Test:乱序回复路由 ─────────────────────────

#[tokio::test]
async fn out_of_order_replies_are_routed_to_correct_caller() {
    let socket_name = unique_socket("reorder");
    let (ready_tx, ready_rx) = oneshot::channel();
    let server_socket = socket_name.clone();
    let server = tokio::spawn(async move { run_reorder_server(&server_socket, ready_tx).await });
    ready_rx.await.unwrap();

    let driver = make_manifest(socket_name);
    let mut conn = ExternalDbConnection::new(make_config(), driver);
    conn.connect().await.expect("connect");
    let conn = Arc::new(conn);

    // 3 个并发 query,等 server 收齐再乱序回复。
    let h1 = {
        let c = conn.clone();
        tokio::spawn(async move { c.query("REQ-A").await })
    };
    let h2 = {
        let c = conn.clone();
        tokio::spawn(async move { c.query("REQ-B").await })
    };
    let h3 = {
        let c = conn.clone();
        tokio::spawn(async move { c.query("REQ-C").await })
    };

    // 5s outer timeout 防止旧实现死锁拖到 30s。
    let test_run = async {
        let r1 = h1.await.unwrap().expect("REQ-A should succeed");
        let r2 = h2.await.unwrap().expect("REQ-B should succeed");
        let r3 = h3.await.unwrap().expect("REQ-C should succeed");
        (
            extract_query_sql(r1),
            extract_query_sql(r2),
            extract_query_sql(r3),
        )
    };
    let (s1, s2, s3) = tokio::time::timeout(Duration::from_secs(5), test_run)
        .await
        .expect("concurrent queries must complete within 5s — old client serializes through Mutex<JsonRpcClient> and deadlocks the rendezvous");

    // 每个 caller 必须拿到自己发的 sql,而不是别人的回复。
    assert_eq!(s1, "REQ-A");
    assert_eq!(s2, "REQ-B");
    assert_eq!(s3, "REQ-C");

    let mut conn = Arc::try_unwrap(conn)
        .ok()
        .expect("no other strong refs after join");
    conn.disconnect().await.expect("disconnect");
    server.await.unwrap().expect("server task ok");
}

async fn run_reorder_server(
    socket_name: &str,
    ready_tx: oneshot::Sender<()>,
) -> std::io::Result<()> {
    let name = socket_name.to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_tokio()?;
    let _ = ready_tx.send(());
    let mut conn = listener.accept().await?;

    handshake(&mut conn).await?;

    // 关键:**收齐 3 个 query 后再回**,逼客户端必须并发发送(rendezvous)。
    let q1: IpcRequest = recv_msg_async(&mut conn).await?;
    let q2: IpcRequest = recv_msg_async(&mut conn).await?;
    let q3: IpcRequest = recv_msg_async(&mut conn).await?;

    let sql_q1 = q1.params["sql"].as_str().unwrap().to_owned();
    let sql_q2 = q2.params["sql"].as_str().unwrap().to_owned();
    let sql_q3 = q3.params["sql"].as_str().unwrap().to_owned();

    // 关键:乱序回复 — q3 → q1 → q2。
    send_msg_async(&mut conn, &make_query_response(q3.request_id, &sql_q3)).await?;
    send_msg_async(&mut conn, &make_query_response(q1.request_id, &sql_q1)).await?;
    send_msg_async(&mut conn, &make_query_response(q2.request_id, &sql_q2)).await?;

    // disconnect。
    let dc: IpcRequest = recv_msg_async(&mut conn).await?;
    send_msg_async(&mut conn, &IpcResponse::result(dc.request_id, json!({}))).await?;
    Ok(())
}

// ───────────────────────── Test:fatal 后清理,下次 NotConnected ─────────────────────────

#[tokio::test]
async fn fatal_transport_error_evicts_client_so_next_query_returns_not_connected() {
    let socket_name = unique_socket("evict");
    let (ready_tx, ready_rx) = oneshot::channel();
    let server_socket = socket_name.clone();
    let server =
        tokio::spawn(
            async move { run_drop_after_handshake_server(&server_socket, ready_tx).await },
        );
    ready_rx.await.unwrap();

    let driver = make_manifest(socket_name);
    let mut conn = ExternalDbConnection::new(make_config(), driver);
    conn.connect()
        .await
        .expect("connect should succeed before driver drops the stream");

    // 首次 query:driver 已关 stream → reader 检测到 EOF → CloseGuard 标记 closed,
    // caller 收到 disconnected 错误(具体类型依赖时序,但必须是错误)。
    let first = tokio::time::timeout(Duration::from_secs(3), conn.query("first"))
        .await
        .expect("first query must fail within 3s after disconnect");
    assert!(
        first.is_err(),
        "first query should fail because driver disconnected"
    );

    // 关键 assertion:第二次 query 必须立刻返回 NotConnected,
    // 而不是再次走 transport(说明上一次失败已经 evict 了 broken client)。
    let second = conn.query("second").await;
    let err = second.expect_err("second query should fail with NotConnected");
    assert!(
        matches!(err, db::DbError::NotConnected),
        "second query should return NotConnected after fatal transport error; got: {err:?}"
    );

    // server task 自然结束。
    let _ = tokio::time::timeout(Duration::from_secs(1), server).await;
}

async fn run_drop_after_handshake_server(
    socket_name: &str,
    ready_tx: oneshot::Sender<()>,
) -> std::io::Result<()> {
    let name = socket_name.to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_tokio()?;
    let _ = ready_tx.send(());
    let mut conn = listener.accept().await?;

    handshake(&mut conn).await?;

    // 完成 initialize/connect 后立刻关 stream,模拟 driver 异常退出。
    drop(conn);
    Ok(())
}
