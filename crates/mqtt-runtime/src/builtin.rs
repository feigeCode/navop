//! MQTT 进程内连接实现(基于 rumqttc)。

use crate::connection::MqttConnection;
use crate::pubsub::{MQTT_MESSAGE_CHANNEL_CAPACITY, MqttPubSubHandle};
use crate::types::*;
use async_trait::async_trait;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, Transport};
use ssh::{HostKeyVerifier, LocalPortForwardTunnel, SshAuth, SshConnectConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, broadcast, watch};
use tokio::time::timeout;

const DEFAULT_SSH_TIMEOUT_SECONDS: u64 = 30;

struct ResolvedMqttConnectionTarget {
    host: String,
    port: u16,
    tunnel: Option<LocalPortForwardTunnel>,
}

fn normalize_direct_host(host: &str) -> String {
    if host.eq_ignore_ascii_case("localhost") {
        return "127.0.0.1".to_string();
    }
    host.to_string()
}

fn required_ssh_value(value: &str, key: &str) -> Result<String, MqttError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(MqttError::connection(format!(
            "ssh tunnel enabled but `{key}` is missing"
        )));
    }
    Ok(value.to_string())
}

fn build_ssh_auth(
    tunnel_config: &connection_tunnel::SshTunnelConfig,
) -> Result<SshAuth, MqttError> {
    match tunnel_config.auth_type.trim().to_ascii_lowercase().as_str() {
        "agent" => Ok(SshAuth::Agent),
        "pageant" => Ok(SshAuth::Pageant),
        "auto_publickey" | "auto_public_key" => Ok(SshAuth::AutoPublicKey),
        "private_key" => {
            let key_path = tunnel_config
                .private_key_path
                .as_deref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    MqttError::connection(
                        "ssh tunnel enabled but `ssh_private_key_path` is missing",
                    )
                })?;
            Ok(SshAuth::PrivateKey {
                key_path: key_path.to_string(),
                passphrase: tunnel_config.private_key_passphrase.clone(),
                certificate_path: None,
            })
        }
        "private_key_content" | "private_key_material" => {
            let private_key = tunnel_config
                .private_key_content
                .as_deref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    MqttError::connection(
                        "ssh tunnel enabled but `ssh_private_key_content` is missing",
                    )
                })?;
            Ok(SshAuth::PrivateKeyContent {
                private_key: private_key.to_string(),
                passphrase: tunnel_config.private_key_passphrase.clone(),
                certificate_path: None,
            })
        }
        _ => {
            let password = tunnel_config
                .password
                .as_deref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    MqttError::connection("ssh tunnel enabled but `ssh_password` is missing")
                })?;
            Ok(SshAuth::Password(password.to_string()))
        }
    }
}

async fn resolve_connection_target(
    config: &MqttConnectionConfig,
) -> Result<ResolvedMqttConnectionTarget, MqttError> {
    let Some(tunnel_config) = config.ssh_tunnel.as_ref().filter(|tunnel| tunnel.enabled) else {
        return Ok(ResolvedMqttConnectionTarget {
            host: normalize_direct_host(&config.host),
            port: config.port,
            tunnel: None,
        });
    };

    let ssh_host = required_ssh_value(&tunnel_config.host, "ssh_host")?;
    let ssh_username = required_ssh_value(&tunnel_config.username, "ssh_username")?;
    let auth = build_ssh_auth(tunnel_config)?;
    let target_host = tunnel_config
        .target_host
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| config.host.clone());
    let target_port = tunnel_config.target_port.unwrap_or(config.port);
    let timeout_secs = tunnel_config.timeout.unwrap_or(DEFAULT_SSH_TIMEOUT_SECONDS);

    let ssh_config = SshConnectConfig {
        host: ssh_host,
        port: tunnel_config.port,
        username: ssh_username,
        auth,
        timeout: Some(Duration::from_secs(timeout_secs)),
        keepalive_interval: None,
        keepalive_max: None,
        jump_server: None,
        proxy: None,
        keyboard_interactive_responder: None,
        host_key_verifier: HostKeyVerifier::default(),
        x11_forwarding: false,
        allow_legacy_algorithms: false,
    };

    let tunnel_result = timeout(
        Duration::from_secs(timeout_secs),
        ssh::start_local_port_forward(ssh_config, target_host, target_port),
    )
    .await;

    let tunnel = match tunnel_result {
        Ok(Ok(tunnel)) => tunnel,
        Ok(Err(error)) => {
            return Err(MqttError::Tunnel(format!(
                "failed to establish ssh tunnel: {error}"
            )));
        }
        Err(_) => {
            return Err(MqttError::Timeout(
                "ssh tunnel establishment timed out".to_string(),
            ));
        }
    };

    let local_addr = tunnel.local_addr();

    Ok(ResolvedMqttConnectionTarget {
        host: local_addr.ip().to_string(),
        port: local_addr.port(),
        tunnel: Some(tunnel),
    })
}

fn map_qos(qos: MqttQos) -> rumqttc::QoS {
    match qos {
        MqttQos::AtMostOnce => rumqttc::QoS::AtMostOnce,
        MqttQos::AtLeastOnce => rumqttc::QoS::AtLeastOnce,
        MqttQos::ExactlyOnce => rumqttc::QoS::ExactlyOnce,
    }
}

/// MQTT 连接实现(rumqttc AsyncClient + EventLoop)
pub struct MqttConnectionImpl {
    config: MqttConnectionConfig,
    client: Option<AsyncClient>,
    poll_task: Option<tokio::task::JoinHandle<()>>,
    connected: Arc<AtomicBool>,
    /// 连接状态 watch(poll 任务写,connect 等待初始 ConnAck)
    connected_tx: watch::Sender<bool>,
    subscriptions: Arc<Mutex<Vec<MqttSubscription>>>,
    message_tx: broadcast::Sender<MqttMessage>,
    tunnel: Option<LocalPortForwardTunnel>,
}

impl MqttConnectionImpl {
    pub fn new(config: MqttConnectionConfig) -> Self {
        let (message_tx, _) = broadcast::channel(MQTT_MESSAGE_CHANNEL_CAPACITY);
        let (connected_tx, _) = watch::channel(false);
        Self {
            config,
            client: None,
            poll_task: None,
            connected: Arc::new(AtomicBool::new(false)),
            connected_tx,
            subscriptions: Arc::new(Mutex::new(Vec::new())),
            message_tx,
            tunnel: None,
        }
    }

    fn require_client(&self) -> Result<&AsyncClient, MqttError> {
        self.client.as_ref().ok_or(MqttError::NotConnected)
    }

    async fn wait_for_connack(&self, mut rx: watch::Receiver<bool>) -> Result<(), MqttError> {
        let deadline = Duration::from_secs(self.config.timeout.max(1));
        match timeout(deadline, async {
            loop {
                if *rx.borrow_and_update() {
                    return Ok(());
                }
                if rx.changed().await.is_err() {
                    return Ok(());
                }
            }
        })
        .await
        {
            Ok(result) => result,
            Err(_) => Err(MqttError::Timeout(
                "mqtt broker did not acknowledge the connection in time".to_string(),
            )),
        }
    }
}

#[async_trait]
impl MqttConnection for MqttConnectionImpl {
    fn config(&self) -> &MqttConnectionConfig {
        &self.config
    }

    async fn connect(&mut self) -> Result<(), MqttError> {
        if self.client.is_some() && self.is_connected() {
            return Ok(());
        }

        let target = resolve_connection_target(&self.config).await?;
        self.tunnel = target.tunnel;

        let client_id = if self.config.client_id.trim().is_empty() {
            format!("navop-{}", uuid::Uuid::new_v4())
        } else {
            self.config.client_id.trim().to_string()
        };

        let mut options = MqttOptions::new(client_id, &target.host, target.port);
        options.set_keep_alive(Duration::from_secs(self.config.keep_alive_secs.max(1)));
        options.set_clean_session(self.config.clean_session);
        if let Some(username) = self
            .config
            .username
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        {
            let password = self.config.password.clone().unwrap_or_default();
            options.set_credentials(username.to_string(), password);
        }
        if self.config.use_tls {
            options.set_transport(Transport::tls_with_default_config());
        }

        let (client, event_loop) = AsyncClient::new(options, MQTT_MESSAGE_CHANNEL_CAPACITY);

        let connected = self.connected.clone();
        let connected_tx = self.connected_tx.clone();
        let message_tx = self.message_tx.clone();
        let subscriptions = self.subscriptions.clone();

        connected.store(false, Ordering::SeqCst);
        connected_tx.send_replace(false);

        let poll_task = tokio::spawn(async move {
            let mut event_loop = event_loop;
            loop {
                match event_loop.poll().await {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                        connected.store(true, Ordering::SeqCst);
                        let _ = connected_tx.send(true);
                    }
                    Ok(Event::Incoming(Incoming::Disconnect)) => {
                        connected.store(false, Ordering::SeqCst);
                        let _ = connected_tx.send(false);
                    }
                    Ok(Event::Incoming(Incoming::Publish(publish))) => {
                        let qos = MqttQos::from_u8(publish.qos as u8).unwrap_or_default();
                        let message = MqttMessage {
                            topic: publish.topic.clone(),
                            payload: publish.payload.to_vec(),
                            qos,
                            retain: publish.retain,
                            received_at: chrono::Utc::now(),
                        };
                        // lagged 时丢弃旧消息,不影响 poll 循环
                        let _ = message_tx.send(message);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(?error, "mqtt eventloop poll error, reconnecting");
                        connected.store(false, Ordering::SeqCst);
                        let _ = connected_tx.send(false);
                        // rumqttc 在下一次 poll 时自动重连;稍作退避避免忙等
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        });

        // 等待初始 ConnAck(带超时);clean_session=false 时 broker 可能回放保留订阅
        self.wait_for_connack(self.connected_tx.subscribe()).await?;

        // 断线重连后自动恢复本地订阅表中的订阅
        let resubscribe: Vec<MqttSubscription> = subscriptions.lock().await.clone();
        for subscription in resubscribe {
            if let Err(error) = client
                .subscribe(&subscription.topic_filter, map_qos(subscription.qos))
                .await
            {
                tracing::warn!(
                    %error,
                    topic = %subscription.topic_filter,
                    "failed to restore mqtt subscription"
                );
            }
        }

        self.client = Some(client);
        self.poll_task = Some(poll_task);

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), MqttError> {
        if let Some(client) = self.client.take() {
            let _ = client.disconnect().await;
        }
        if let Some(task) = self.poll_task.take() {
            task.abort();
        }
        self.connected.store(false, Ordering::SeqCst);
        self.connected_tx.send_replace(false);
        // 释放 SSH 隧道
        self.tunnel = None;
        Ok(())
    }

    async fn ping(&self) -> Result<(), MqttError> {
        if self.is_connected() {
            Ok(())
        } else {
            Err(MqttError::NotConnected)
        }
    }

    fn is_connected(&self) -> bool {
        self.client.is_some() && self.connected.load(Ordering::SeqCst)
    }

    async fn publish(
        &self,
        topic: &str,
        payload: &[u8],
        qos: MqttQos,
        retain: bool,
    ) -> Result<(), MqttError> {
        let client = self.require_client()?;
        client
            .publish(topic, map_qos(qos), retain, payload.to_vec())
            .await
            .map_err(|error| MqttError::Protocol(format!("publish failed: {error}")))?;
        Ok(())
    }

    async fn subscribe(&self, topic_filter: &str, qos: MqttQos) -> Result<(), MqttError> {
        let client = self.require_client()?;
        client
            .subscribe(topic_filter.to_string(), map_qos(qos))
            .await
            .map_err(|error| MqttError::Protocol(format!("subscribe failed: {error}")))?;
        self.subscriptions.lock().await.push(MqttSubscription {
            topic_filter: topic_filter.to_string(),
            qos,
        });
        Ok(())
    }

    async fn unsubscribe(&self, topic_filter: &str) -> Result<(), MqttError> {
        let client = self.require_client()?;
        client
            .unsubscribe(topic_filter.to_string())
            .await
            .map_err(|error| MqttError::Protocol(format!("unsubscribe failed: {error}")))?;
        self.subscriptions
            .lock()
            .await
            .retain(|subscription| subscription.topic_filter != topic_filter);
        Ok(())
    }

    async fn list_subscriptions(&self) -> Result<Vec<MqttSubscription>, MqttError> {
        Ok(self.subscriptions.lock().await.clone())
    }

    fn open_pubsub(&self) -> Result<MqttPubSubHandle, MqttError> {
        Ok(MqttPubSubHandle::new(self.message_tx.subscribe()))
    }
}
