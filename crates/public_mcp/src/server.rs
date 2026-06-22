use crate::protocol::PublicMcpServer;
use anyhow::Result;
use rmcp::ServiceExt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

pub struct LoopbackMcpServer {
    bind_addr: SocketAddr,
    accept_task: JoinHandle<()>,
    client_count: Arc<AtomicUsize>,
}

impl LoopbackMcpServer {
    pub async fn bind(protocol: PublicMcpServer, token: String) -> Result<Self> {
        let listener =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).await?;
        let bind_addr = listener.local_addr()?;
        let client_count = Arc::new(AtomicUsize::new(0));
        let accept_task =
            tokio::spawn(accept_loop(listener, protocol, token, client_count.clone()));
        Ok(Self {
            bind_addr,
            accept_task,
            client_count,
        })
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    pub fn client_count(&self) -> usize {
        self.client_count.load(Ordering::SeqCst)
    }
}

impl Drop for LoopbackMcpServer {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

async fn accept_loop(
    listener: TcpListener,
    protocol: PublicMcpServer,
    token: String,
    client_count: Arc<AtomicUsize>,
) {
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            break;
        };
        tokio::spawn(handle_client(
            stream,
            protocol.clone(),
            token.clone(),
            client_count.clone(),
        ));
    }
}

async fn handle_client(
    stream: TcpStream,
    protocol: PublicMcpServer,
    token: String,
    client_count: Arc<AtomicUsize>,
) {
    let _guard = ClientCountGuard::new(client_count);
    if let Err(error) = serve_on_stream(stream, protocol, &token).await {
        tracing::debug!("public MCP client disconnected: {error}");
    }
}

struct ClientCountGuard {
    client_count: Arc<AtomicUsize>,
}

impl ClientCountGuard {
    fn new(client_count: Arc<AtomicUsize>) -> Self {
        client_count.fetch_add(1, Ordering::SeqCst);
        Self { client_count }
    }
}

impl Drop for ClientCountGuard {
    fn drop(&mut self) {
        self.client_count.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 在任意支持 async read/write 的流上执行 token 校验并启动 rmcp 服务。
///
/// 暴露为 pub 主要是为了让协议层测试可以用 `tokio::io::duplex` 驱动,
/// 避免依赖真实 loopback TCP 绑定(在受限/CI 环境里 bind 可能被拒绝)。
pub async fn serve_on_stream<S>(mut stream: S, protocol: PublicMcpServer, token: &str) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // 逐字节读取 token 行,读完后的 stream 再交给 rmcp 处理后续 JSON-RPC。
    let line = read_token_line_from(&mut stream).await?;
    if line.trim_end() != token {
        return Ok(());
    }

    let running_service = protocol.serve(stream).await?;
    running_service.waiting().await?;
    Ok(())
}

async fn read_token_line_from<R: AsyncRead + Unpin>(reader: &mut R) -> Result<String> {
    let mut bytes = Vec::with_capacity(128);
    let mut byte = [0_u8; 1];
    loop {
        let read = reader.read(&mut byte).await?;
        if read == 0 || byte[0] == b'\n' {
            break;
        }
        bytes.push(byte[0]);
    }
    Ok(String::from_utf8(bytes)?)
}
