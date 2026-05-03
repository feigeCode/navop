use crate::duckdb_session::DuckDbSession;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, connect_config, string_param};
use anyhow::{Context, Result};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions,
    tokio::{Stream, prelude::*},
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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

async fn handle_connection(stream: Stream) -> Result<()> {
    let mut reader = BufReader::new(&stream);
    let mut writer = &stream;
    let mut session = DuckDbSession::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }

        let request = serde_json::from_str::<JsonRpcRequest>(line.trim_end())?;
        let id = request.id;
        let should_disconnect = request.method == "disconnect";
        let response = match handle_request(&mut session, request) {
            Ok(result) => JsonRpcResponse::result(id, result),
            Err(error) => JsonRpcResponse::error(id, -32000, error.to_string()),
        };
        let line = serde_json::to_string(&response)?;
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        if should_disconnect {
            return Ok(());
        }
    }
}

fn handle_request(session: &mut DuckDbSession, request: JsonRpcRequest) -> Result<Value> {
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
