use crate::discovery::{DiscoveryDocument, public_mcp_discovery_path, read_discovery};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncWriteExt, copy};
use tokio::net::TcpStream;
use tokio::time::timeout;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn run_stdio_bridge(discovery_path: Option<PathBuf>) -> Result<()> {
    let path = discovery_path.unwrap_or_else(public_mcp_discovery_path);
    let discovery = load_discovery(&path)?;
    let stream = connect_to_runtime(&discovery).await?;

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

pub fn parse_discovery_path_arg<I, S>(args: I) -> Result<Option<PathBuf>>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = args.into_iter().map(Into::into);
    let mut discovery_path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--discovery" => {
                let path = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--discovery requires a path"))?;
                if discovery_path.replace(PathBuf::from(path)).is_some() {
                    bail!("--discovery was provided more than once");
                }
            }
            "--help" | "-h" => bail!("usage: onetcli-public-mcp [--discovery <path>]"),
            other => bail!("unknown argument `{other}`"),
        }
    }
    Ok(discovery_path)
}

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
