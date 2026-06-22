use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, oneshot};

use crate::socks5::{
    parse_connect_request, select_no_auth_method, socks5_method_selection, socks5_reply,
};
use crate::{RusshClient, SshClient, SshConnectConfig};

const SOCKS_REPLY_SUCCEEDED: u8 = 0x00;
const SOCKS_REPLY_GENERAL_FAILURE: u8 = 0x01;
const SOCKS_METHOD_NO_AUTH: u8 = 0x00;
const SOCKS_METHOD_NO_ACCEPTABLE: u8 = 0xff;

pub struct DynamicSocksConfig {
    pub bind_host: String,
    pub bind_port: u16,
}

pub struct DynamicSocksTunnel {
    local_addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    accept_task: Option<tokio::task::JoinHandle<()>>,
    client: Arc<Mutex<RusshClient>>,
}

impl DynamicSocksTunnel {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn close(&mut self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.accept_task.take() {
            let _ = task.await;
        }
        let mut guard = self.client.lock().await;
        guard.disconnect().await
    }
}

impl Drop for DynamicSocksTunnel {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.accept_task.take() {
            task.abort();
        }
    }
}

pub async fn start_dynamic_socks_forward(
    config: SshConnectConfig,
    socks_config: DynamicSocksConfig,
) -> Result<DynamicSocksTunnel> {
    let bind_addr = build_dynamic_socks_bind_addr(&socks_config.bind_host, socks_config.bind_port);
    let listener = TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("failed to bind SOCKS address: {bind_addr}"))?;
    let local_addr = listener.local_addr()?;
    let client = Arc::new(Mutex::new(
        <RusshClient as SshClient>::connect(config).await?,
    ));
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    let client_for_task = Arc::clone(&client);

    let accept_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accept_result = listener.accept() => {
                    let (inbound, inbound_addr) = match accept_result {
                        Ok(result) => result,
                        Err(error) => {
                            tracing::error!("SOCKS accept 失败: {}", error);
                            break;
                        }
                    };
                    tokio::spawn(handle_socks_connection(
                        inbound,
                        inbound_addr,
                        Arc::clone(&client_for_task),
                    ));
                }
            }
        }
    });

    Ok(DynamicSocksTunnel {
        local_addr,
        shutdown_tx: Some(shutdown_tx),
        accept_task: Some(accept_task),
        client,
    })
}

fn build_dynamic_socks_bind_addr(bind_host: &str, bind_port: u16) -> String {
    format!("{bind_host}:{bind_port}")
}

async fn handle_socks_connection(
    inbound: TcpStream,
    inbound_addr: SocketAddr,
    client: Arc<Mutex<RusshClient>>,
) {
    if let Err(error) = run_socks_connection(inbound, inbound_addr, client).await {
        tracing::debug!("SOCKS 连接结束: {}", error);
    }
}

async fn run_socks_connection(
    mut inbound: TcpStream,
    inbound_addr: SocketAddr,
    client: Arc<Mutex<RusshClient>>,
) -> Result<()> {
    negotiate_no_auth(&mut inbound).await?;
    let request = read_connect_request(&mut inbound).await?;
    let origin_host = inbound_addr.ip().to_string();
    let origin_port = inbound_addr.port();
    let direct_channel = {
        let mut guard = client.lock().await;
        guard
            .open_direct_tcpip_channel(&request.host, request.port, &origin_host, origin_port)
            .await
    };

    let Ok(channel) = direct_channel else {
        inbound
            .write_all(&socks5_reply(SOCKS_REPLY_GENERAL_FAILURE))
            .await?;
        bail!("failed to open SSH direct-tcpip channel");
    };
    inbound
        .write_all(&socks5_reply(SOCKS_REPLY_SUCCEEDED))
        .await?;
    let mut outbound = channel.into_stream();
    copy_bidirectional(&mut inbound, &mut outbound).await?;
    Ok(())
}

async fn negotiate_no_auth(inbound: &mut TcpStream) -> Result<()> {
    let mut head = [0u8; 2];
    inbound.read_exact(&mut head).await?;
    let mut greeting = vec![head[0], head[1]];
    let mut methods = vec![0u8; head[1] as usize];
    inbound.read_exact(&mut methods).await?;
    greeting.extend(methods);
    let method = match select_no_auth_method(&greeting) {
        Ok(method) => method,
        Err(error) => {
            inbound
                .write_all(&socks5_method_selection(SOCKS_METHOD_NO_ACCEPTABLE))
                .await?;
            return Err(error);
        }
    };
    debug_assert_eq!(method, SOCKS_METHOD_NO_AUTH);
    inbound.write_all(&socks5_method_selection(method)).await?;
    Ok(())
}

async fn read_connect_request(
    inbound: &mut TcpStream,
) -> Result<crate::socks5::Socks5ConnectRequest> {
    let mut head = [0u8; 4];
    inbound.read_exact(&mut head).await?;
    let mut request = head.to_vec();
    match head[3] {
        0x01 => read_address_and_port(inbound, &mut request, 4).await?,
        0x03 => {
            let mut len = [0u8; 1];
            inbound.read_exact(&mut len).await?;
            request.push(len[0]);
            read_address_and_port(inbound, &mut request, len[0] as usize).await?;
        }
        0x04 => read_address_and_port(inbound, &mut request, 16).await?,
        _ => bail!("unsupported SOCKS5 address type"),
    }
    parse_connect_request(&request)
}

async fn read_address_and_port(
    inbound: &mut TcpStream,
    request: &mut Vec<u8>,
    address_len: usize,
) -> Result<()> {
    let mut rest = vec![0u8; address_len + 2];
    inbound.read_exact(&mut rest).await?;
    request.extend(rest);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::build_dynamic_socks_bind_addr;

    #[test]
    fn dynamic_socks_bind_addr_uses_requested_host_and_port() {
        assert_eq!(
            build_dynamic_socks_bind_addr("127.0.0.1", 1080),
            "127.0.0.1:1080"
        );
    }
}
