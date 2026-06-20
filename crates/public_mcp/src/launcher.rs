use crate::discovery::{public_mcp_discovery_path, read_discovery};
use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::io::{AsyncWriteExt, copy};
use tokio::net::TcpStream;

pub async fn run_stdio_bridge(discovery_path: Option<PathBuf>) -> Result<()> {
    let path = discovery_path.unwrap_or_else(public_mcp_discovery_path);
    let discovery = read_discovery(&path)
        .with_context(|| format!("failed to read public MCP discovery: {}", path.display()))?;
    let mut stream = TcpStream::connect(discovery.socket_addr()?)
        .await
        .context("failed to connect to OnetCli public MCP runtime")?;

    stream.write_all(discovery.token.as_bytes()).await?;
    stream.write_all(b"\n").await?;

    let (mut tcp_read, mut tcp_write) = stream.into_split();
    let stdin_task = tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        copy(&mut stdin, &mut tcp_write).await
    });
    let stdout_task = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        copy(&mut tcp_read, &mut stdout).await
    });

    tokio::select! {
        result = stdin_task => { result??; }
        result = stdout_task => { result??; }
    }

    Ok(())
}
