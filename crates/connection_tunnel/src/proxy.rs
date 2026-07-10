use std::net::SocketAddr;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::JoinHandle;

use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use ssh::ProxyConnectConfig;
pub use ssh::ProxyType as ProxyTunnelType;

#[derive(Debug, Error)]
pub enum ProxyTunnelError {
    #[error("proxy `{0}` is required")]
    MissingField(&'static str),
    #[error("failed to establish proxy tunnel: {0}")]
    Establish(String),
}

impl ProxyTunnelError {
    pub fn field(&self) -> Option<&'static str> {
        match self {
            Self::MissingField(field) => Some(field),
            Self::Establish(_) => None,
        }
    }
}

#[derive(Clone)]
pub struct ProxyTunnelConfig {
    pub proxy_type: ProxyTunnelType,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

pub struct ProxyTunnel {
    local_addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl ProxyTunnel {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

impl Drop for ProxyTunnel {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl ProxyTunnelConfig {
    pub fn validate(&self) -> Result<(), ProxyTunnelError> {
        if self.host.trim().is_empty() {
            return Err(ProxyTunnelError::MissingField("host"));
        }
        if self.port == 0 {
            return Err(ProxyTunnelError::MissingField("port"));
        }
        if optional_value(&self.username).is_none() && optional_value(&self.password).is_some() {
            return Err(ProxyTunnelError::MissingField("username"));
        }
        Ok(())
    }

    pub fn to_ssh_proxy(&self) -> Result<ProxyConnectConfig, ProxyTunnelError> {
        self.validate()?;
        Ok(ProxyConnectConfig {
            proxy_type: self.proxy_type,
            host: self.host.trim().to_string(),
            port: self.port,
            username: optional_value(&self.username),
            password: optional_value(&self.password),
        })
    }
}

pub fn start_proxy_tunnel(
    config: ProxyTunnelConfig,
    target_host: impl Into<String>,
    target_port: u16,
) -> Result<ProxyTunnel, ProxyTunnelError> {
    let proxy = config.to_ssh_proxy()?;
    let target_host = target_host.into();
    if target_host.trim().is_empty() {
        return Err(ProxyTunnelError::MissingField("target_host"));
    }
    if target_port == 0 {
        return Err(ProxyTunnelError::MissingField("target_port"));
    }

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (ready_tx, ready_rx) = sync_channel(1);
    let worker = std::thread::Builder::new()
        .name("onetcli-proxy-tunnel".to_string())
        .spawn(move || run_proxy_worker(proxy, target_host, target_port, shutdown_rx, ready_tx))
        .map_err(|error| ProxyTunnelError::Establish(error.to_string()))?;
    let local_addr = ready_rx
        .recv()
        .map_err(|error| ProxyTunnelError::Establish(error.to_string()))??;

    Ok(ProxyTunnel {
        local_addr,
        shutdown_tx: Some(shutdown_tx),
        worker: Some(worker),
    })
}

fn run_proxy_worker(
    proxy: ProxyConnectConfig,
    target_host: String,
    target_port: u16,
    shutdown_rx: oneshot::Receiver<()>,
    ready_tx: SyncSender<Result<SocketAddr, ProxyTunnelError>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready_tx.send(Err(ProxyTunnelError::Establish(error.to_string())));
            return;
        }
    };
    runtime.block_on(run_proxy_listener(
        proxy,
        target_host,
        target_port,
        shutdown_rx,
        ready_tx,
    ));
}

async fn run_proxy_listener(
    proxy: ProxyConnectConfig,
    target_host: String,
    target_port: u16,
    mut shutdown_rx: oneshot::Receiver<()>,
    ready_tx: SyncSender<Result<SocketAddr, ProxyTunnelError>>,
) {
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(error) => {
            let _ = ready_tx.send(Err(ProxyTunnelError::Establish(error.to_string())));
            return;
        }
    };
    let local_addr = match listener.local_addr() {
        Ok(local_addr) => local_addr,
        Err(error) => {
            let _ = ready_tx.send(Err(ProxyTunnelError::Establish(error.to_string())));
            return;
        }
    };
    if ready_tx.send(Ok(local_addr)).is_err() {
        return;
    }

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => break,
            accepted = listener.accept() => match accepted {
                Ok((inbound, _)) => {
                    let proxy = proxy.clone();
                    let target_host = target_host.clone();
                    tokio::spawn(forward_connection(inbound, proxy, target_host, target_port));
                }
                Err(error) => {
                    tracing::error!("代理隧道 accept 失败: {error}");
                    break;
                }
            }
        }
    }
}

async fn forward_connection(
    mut inbound: TcpStream,
    proxy: ProxyConnectConfig,
    target_host: String,
    target_port: u16,
) {
    let mut outbound = match ssh::connect_via_proxy(&proxy, &target_host, target_port).await {
        Ok(stream) => stream,
        Err(error) => {
            tracing::error!("代理隧道连接目标失败: {error}");
            let _ = inbound.shutdown().await;
            return;
        }
    };
    if let Err(error) = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await {
        tracing::debug!("代理隧道连接结束: {error}");
    }
}

fn optional_value(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
