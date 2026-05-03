use crate::duckdb_session::DuckDbSession;
use crate::protocol::{connect_config, string_param};
use anyhow::{Context, Result};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions,
    tokio::{Stream, prelude::*},
};
use ipc::{
    IpcErrorCode, IpcRequest, IpcResponse,
    framing::{recv_msg_async, send_msg_async},
};
use serde_json::{Value, json};

pub async fn run(socket_name: &str) -> Result<()> {
    let name = socket_name
        .to_ns_name::<GenericNamespaced>()
        .context("invalid local socket name")?;
    let listener = ListenerOptions::new()
        .name(name)
        .create_tokio()
        .context("failed to create local socket listener")?;

    loop {
        let stream = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream).await {
                tracing::warn!("DuckDB IPC connection failed: {error:#}");
            }
        });
    }
}

async fn handle_connection(mut stream: Stream) -> Result<()> {
    let mut session = DuckDbSession::new();

    loop {
        let request: IpcRequest = match recv_msg_async(&mut stream).await {
            Ok(req) => req,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        let request_id = request.request_id;
        let should_disconnect = request.method == "disconnect";

        let response = match handle_request(&mut session, &request) {
            Ok(result) => IpcResponse::result(request_id, result),
            Err(error) => IpcResponse::error(request_id, IpcErrorCode::Internal, error.to_string()),
        };

        send_msg_async(&mut stream, &response).await?;

        if should_disconnect {
            return Ok(());
        }
    }
}

fn handle_request(session: &mut DuckDbSession, request: &IpcRequest) -> Result<Value> {
    match request.method.as_str() {
        "initialize" => Ok(json!({})),
        "connect" => {
            session.connect(connect_config(&request.params)?)?;
            Ok(json!({}))
        }
        "disconnect" => {
            session.disconnect();
            Ok(json!({}))
        }
        "ping" => {
            session.ping()?;
            Ok(json!({}))
        }
        "current_database" => Ok(json!(session.current_database())),
        "switch_database" => {
            anyhow::bail!("DuckDB does not support switching databases within one file connection")
        }
        "switch_schema" => Ok(json!({})),
        "query" => Ok(serde_json::to_value(
            session.query(string_param(&request.params, "sql")?),
        )?),
        method if method.starts_with("metadata.") => {
            crate::metadata::handle(session, method, &request.params)
        }
        method => anyhow::bail!("unsupported method: {method}"),
    }
}
