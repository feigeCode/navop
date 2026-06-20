use crate::protocol::PublicMcpServer;
use anyhow::Result;
use rmcp::ServiceExt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub struct LoopbackMcpServer {
    bind_addr: SocketAddr,
    accept_task: JoinHandle<()>,
}

impl LoopbackMcpServer {
    pub async fn bind(protocol: PublicMcpServer, token: String) -> Result<Self> {
        let listener =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).await?;
        let bind_addr = listener.local_addr()?;
        let accept_task = tokio::spawn(accept_loop(listener, protocol, token));
        Ok(Self {
            bind_addr,
            accept_task,
        })
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }
}

impl Drop for LoopbackMcpServer {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

async fn accept_loop(listener: TcpListener, protocol: PublicMcpServer, token: String) {
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            break;
        };
        tokio::spawn(handle_client(stream, protocol.clone(), token.clone()));
    }
}

async fn handle_client(stream: TcpStream, protocol: PublicMcpServer, token: String) {
    if let Err(error) = handle_client_inner(stream, protocol, &token).await {
        tracing::debug!("public MCP client disconnected: {error}");
    }
}

async fn handle_client_inner(
    mut stream: TcpStream,
    protocol: PublicMcpServer,
    token: &str,
) -> Result<()> {
    let line = read_token_line(&mut stream).await?;
    if line.trim_end() != token {
        return Ok(());
    }

    let running_service = protocol.serve(stream).await?;
    running_service.waiting().await?;
    Ok(())
}

async fn read_token_line(stream: &mut TcpStream) -> Result<String> {
    let mut bytes = Vec::with_capacity(128);
    let mut byte = [0_u8; 1];
    loop {
        let read = stream.read(&mut byte).await?;
        if read == 0 || byte[0] == b'\n' {
            break;
        }
        bytes.push(byte[0]);
    }
    Ok(String::from_utf8(bytes)?)
}
