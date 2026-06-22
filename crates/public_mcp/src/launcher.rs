use crate::discovery::{DiscoveryDocument, read_discovery};
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::timeout;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn load_discovery(path: &Path) -> Result<DiscoveryDocument> {
    if !path.exists() {
        bail!(
            "OnetCli public MCP discovery file is missing at {}. Start OnetCli and enable MCP in Settings > General > MCP.",
            path.display()
        );
    }
    let discovery = read_discovery(path)
        .with_context(|| format!("failed to read public MCP discovery: {}", path.display()))?;
    discovery.validate_for_stdio_bridge()?;
    Ok(discovery)
}

pub async fn connect_to_runtime(discovery: &DiscoveryDocument) -> Result<TcpStream> {
    discovery.validate_for_stdio_bridge()?;
    let addr = discovery.socket_addr()?;
    let mut stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .with_context(|| runtime_unavailable_message("timed out connecting", addr, discovery))?
        .with_context(|| runtime_unavailable_message("failed to connect", addr, discovery))?;

    stream
        .write_all(discovery.token.as_bytes())
        .await
        .context("failed to write public MCP token handshake")?;
    stream
        .write_all(b"\n")
        .await
        .context("failed to finish public MCP token handshake")?;
    Ok(stream)
}

fn runtime_unavailable_message(
    action: &str,
    addr: std::net::SocketAddr,
    discovery: &DiscoveryDocument,
) -> String {
    format!(
        "{action} to OnetCli public MCP runtime at {addr}; discovery may be stale \
         (pid {}, mode {:?}). Start OnetCli and enable MCP in Settings > General > MCP.",
        discovery.pid, discovery.mode
    )
}
