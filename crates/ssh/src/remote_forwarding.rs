use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use russh::ChannelOpenFailure;
use russh::client::{self, ChannelOpenHandle};
use tokio::io::copy_bidirectional;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, watch};
use tokio::time::timeout;

use crate::{RusshClient, SshClient, SshConnectConfig};

const LOCAL_TARGET_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_BIND_ALLOCATION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(crate) struct RemoteForwardTarget {
    host: Arc<str>,
    port: u16,
    bind_port: watch::Sender<u16>,
}

impl RemoteForwardTarget {
    pub(crate) fn new(host: String, port: u16, bind_port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            bind_port: watch::channel(bind_port).0,
        }
    }

    pub(crate) fn set_bind_port(&self, bind_port: u16) {
        self.bind_port.send_replace(bind_port);
    }

    async fn accepts_connected_port(&self, connected_port: u32) -> bool {
        let mut bind_port = self.bind_port.subscribe();
        let allocated_port = async {
            loop {
                let current = *bind_port.borrow_and_update();
                if current != 0 {
                    return current;
                }
                if bind_port.changed().await.is_err() {
                    return 0;
                }
            }
        };
        timeout(REMOTE_BIND_ALLOCATION_TIMEOUT, allocated_port)
            .await
            .is_ok_and(|bind_port| u32::from(bind_port) == connected_port)
    }

    pub(crate) async fn accept_channel(
        &self,
        channel: russh::Channel<client::Msg>,
        reply: ChannelOpenHandle,
        connected_address: String,
        connected_port: u32,
        originator_address: String,
        originator_port: u32,
    ) {
        if !self.accepts_connected_port(connected_port).await {
            tracing::warn!(
                target: "ssh.remote_forward",
                connected_address,
                connected_port,
                "拒绝不属于当前反向端口转发的通道"
            );
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return;
        }

        let connect = TcpStream::connect((self.host.as_ref(), self.port));
        let Ok(Ok(mut local)) = timeout(LOCAL_TARGET_CONNECT_TIMEOUT, connect).await else {
            tracing::error!(
                target: "ssh.remote_forward",
                target_host = self.host.as_ref(),
                target_port = self.port,
                originator_address,
                originator_port,
                "反向端口转发连接本地目标失败"
            );
            reply.reject(ChannelOpenFailure::ConnectFailed).await;
            return;
        };

        reply.accept().await;
        let mut remote = channel.into_stream();
        if let Err(error) = copy_bidirectional(&mut remote, &mut local).await {
            tracing::debug!(
                target: "ssh.remote_forward",
                %error,
                "反向端口转发连接结束"
            );
        }
    }
}

pub struct RemotePortForwardConfig {
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
}

pub struct RemotePortForwardTunnel {
    bind_host: String,
    bind_port: u16,
    client: Arc<Mutex<RusshClient>>,
}

impl RemotePortForwardTunnel {
    pub fn remote_addr(&self) -> String {
        format_host_port(&self.bind_host, self.bind_port)
    }

    pub async fn close(&mut self) -> Result<()> {
        let mut client = self.client.lock().await;
        let cancel_result = client
            .cancel_remote_forward(&self.bind_host, self.bind_port)
            .await
            .context("failed to cancel remote forwarding");
        let disconnect_result = client
            .disconnect()
            .await
            .context("failed to disconnect remote forwarding SSH session");
        finish_remote_forward_close(cancel_result, disconnect_result)
    }
}

fn finish_remote_forward_close(
    cancel_result: Result<()>,
    disconnect_result: Result<()>,
) -> Result<()> {
    match (cancel_result, disconnect_result) {
        (Err(cancel_error), Err(disconnect_error)) => Err(anyhow!(
            "failed to close remote forwarding: cancel: {cancel_error:#}; disconnect: {disconnect_error:#}"
        )),
        _ => Ok(()),
    }
}

pub async fn start_remote_port_forward_with_config(
    config: SshConnectConfig,
    forward_config: RemotePortForwardConfig,
) -> Result<RemotePortForwardTunnel> {
    let mut client = <RusshClient as SshClient>::connect(config).await?;
    let target = RemoteForwardTarget::new(
        forward_config.target_host,
        forward_config.target_port,
        forward_config.bind_port,
    );
    client.set_remote_forward_target(target.clone())?;
    let bind_port = client
        .request_remote_forward(&forward_config.bind_host, forward_config.bind_port)
        .await
        .with_context(|| {
            format!(
                "failed to request remote forwarding on {}:{}",
                forward_config.bind_host, forward_config.bind_port
            )
        })?;
    target.set_bind_port(bind_port);

    Ok(RemotePortForwardTunnel {
        bind_host: forward_config.bind_host,
        bind_port,
        client: Arc::new(Mutex::new(client)),
    })
}

fn format_host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::{RemoteForwardTarget, finish_remote_forward_close, format_host_port};

    #[test]
    fn formats_ipv4_and_ipv6_remote_addresses() {
        assert_eq!("127.0.0.1:8080", format_host_port("127.0.0.1", 8080));
        assert_eq!("[::1]:8080", format_host_port("::1", 8080));
    }

    #[tokio::test]
    async fn accepts_only_the_allocated_remote_port() {
        let target = RemoteForwardTarget::new("127.0.0.1".to_string(), 3000, 0);
        target.set_bind_port(18080);
        assert!(target.accepts_connected_port(18080).await);
        assert!(!target.accepts_connected_port(18081).await);
    }

    #[tokio::test]
    async fn waits_for_server_allocated_remote_port() {
        let target = RemoteForwardTarget::new("127.0.0.1".to_string(), 3000, 0);
        let waiting_target = target.clone();
        let acceptance =
            tokio::spawn(async move { waiting_target.accepts_connected_port(18080).await });

        tokio::task::yield_now().await;
        target.set_bind_port(18080);

        assert!(acceptance.await.unwrap());
    }

    #[test]
    fn close_succeeds_when_cancel_or_disconnect_succeeds() {
        assert!(finish_remote_forward_close(Ok(()), Err(anyhow!("disconnect"))).is_ok());
        assert!(finish_remote_forward_close(Err(anyhow!("cancel")), Ok(())).is_ok());
    }

    #[test]
    fn close_fails_when_cancel_and_disconnect_both_fail() {
        let error = finish_remote_forward_close(Err(anyhow!("cancel")), Err(anyhow!("disconnect")))
            .expect_err("both cleanup failures should be reported");

        assert!(error.to_string().contains("cancel"));
        assert!(error.to_string().contains("disconnect"));
    }
}
