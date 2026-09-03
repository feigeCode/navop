//! MQTT 领域类型:连接配置、错误、QoS、消息与订阅。

use connection_tunnel::SshTunnelConfig;
use one_core::storage::CredentialReference;
use serde::{Deserialize, Serialize};

/// MQTT 连接配置(运行时)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MqttConnectionConfig {
    /// 连接 ID
    #[serde(skip)]
    pub id: String,
    /// 连接名称
    #[serde(skip)]
    pub name: String,
    /// 主机地址
    pub host: String,
    /// 端口(非 TLS 默认 1883,TLS 默认 8883)
    pub port: u16,
    /// 客户端 ID(空串表示连接时自动生成)
    #[serde(default)]
    pub client_id: String,
    /// 用户名
    #[serde(default)]
    pub username: Option<String>,
    /// 密码
    #[serde(default)]
    pub password: Option<String>,
    /// 凭据本引用(仅持久层使用,运行时透传)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
    /// 是否启用 TLS
    #[serde(default)]
    pub use_tls: bool,
    /// 连接超时(秒)
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    /// keep-alive 间隔(秒)
    #[serde(default = "default_keep_alive")]
    pub keep_alive_secs: u64,
    /// MQTT 协议版本
    #[serde(default)]
    pub mqtt_version: MqttVersion,
    /// 清除会话
    #[serde(default = "default_true")]
    pub clean_session: bool,
    /// SSH 隧道配置
    #[serde(default)]
    pub ssh_tunnel: Option<SshTunnelConfig>,
}

fn default_timeout() -> u64 {
    10
}

fn default_keep_alive() -> u64 {
    30
}

fn default_true() -> bool {
    true
}

impl Default for MqttConnectionConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            host: "127.0.0.1".to_string(),
            port: 1883,
            client_id: String::new(),
            username: None,
            password: None,
            credential_reference: None,
            use_tls: false,
            timeout: default_timeout(),
            keep_alive_secs: default_keep_alive(),
            mqtt_version: MqttVersion::V311,
            clean_session: true,
            ssh_tunnel: None,
        }
    }
}

impl MqttConnectionConfig {
    /// 服务器信息显示
    pub fn server_info(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// MQTT 协议版本
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MqttVersion {
    /// MQTT 3.1.1
    #[default]
    V311,
    /// MQTT 5(预留)
    V5,
}

impl MqttVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V311 => "3.1.1",
            Self::V5 => "5.0",
        }
    }
}

/// MQTT 错误
#[derive(Debug, thiserror::Error)]
pub enum MqttError {
    #[error("连接错误: {0}")]
    Connect(String),
    #[error("协议错误: {0}")]
    Protocol(String),
    #[error("操作超时: {0}")]
    Timeout(String),
    #[error("尚未连接到 MQTT 服务器")]
    NotConnected,
    #[error("SSH 隧道错误: {0}")]
    Tunnel(String),
    #[error("配置错误: {0}")]
    Config(String),
    #[error("内部错误: {0}")]
    Internal(String),
}

impl MqttError {
    pub fn connection(message: impl Into<String>) -> Self {
        Self::Connect(message.into())
    }
}

/// MQTT QoS 等级
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MqttQos {
    /// 最多一次(0)
    #[default]
    AtMostOnce,
    /// 至少一次(1)
    AtLeastOnce,
    /// 恰好一次(2)
    ExactlyOnce,
}

impl MqttQos {
    pub fn as_u8(self) -> u8 {
        match self {
            Self::AtMostOnce => 0,
            Self::AtLeastOnce => 1,
            Self::ExactlyOnce => 2,
        }
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::AtMostOnce),
            1 => Some(Self::AtLeastOnce),
            2 => Some(Self::ExactlyOnce),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AtMostOnce => "QoS 0",
            Self::AtLeastOnce => "QoS 1",
            Self::ExactlyOnce => "QoS 2",
        }
    }
}

/// MQTT 消息
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MqttMessage {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: MqttQos,
    pub retain: bool,
    /// 接收时间
    pub received_at: chrono::DateTime<chrono::Utc>,
}

impl MqttMessage {
    /// payload 的 UTF-8 文本(无效 UTF-8 返回 None)
    pub fn payload_text(&self) -> Option<String> {
        String::from_utf8(self.payload.clone()).ok()
    }

    /// payload 的十六进制文本
    pub fn payload_hex(&self) -> String {
        let mut out = String::with_capacity(self.payload.len() * 3);
        for (index, byte) in self.payload.iter().enumerate() {
            if index > 0 {
                out.push(' ');
            }
            out.push_str(&format!("{byte:02X}"));
        }
        out
    }
}

/// MQTT 订阅
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttSubscription {
    /// 主题过滤器(支持 + 与 # 通配符)
    pub topic_filter: String,
    pub qos: MqttQos,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qos_roundtrip() {
        for qos in [
            MqttQos::AtMostOnce,
            MqttQos::AtLeastOnce,
            MqttQos::ExactlyOnce,
        ] {
            assert_eq!(MqttQos::from_u8(qos.as_u8()), Some(qos));
        }
        assert_eq!(MqttQos::from_u8(3), None);
    }

    #[test]
    fn message_payload_text_and_hex() {
        let message = MqttMessage {
            topic: "a/b".into(),
            payload: b"hi".to_vec(),
            qos: MqttQos::AtLeastOnce,
            retain: false,
            received_at: chrono::Utc::now(),
        };
        assert_eq!(message.payload_text().as_deref(), Some("hi"));
        assert_eq!(message.payload_hex(), "68 69");

        let binary = MqttMessage {
            payload: vec![0xFF, 0x00],
            ..message
        };
        assert!(binary.payload_text().is_none());
        assert_eq!(binary.payload_hex(), "FF 00");
    }

    #[test]
    fn config_defaults() {
        let config = MqttConnectionConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 1883);
        assert_eq!(config.timeout, 10);
        assert_eq!(config.keep_alive_secs, 30);
        assert!(config.clean_session);
        assert_eq!(config.mqtt_version, MqttVersion::V311);
        assert_eq!(config.server_info(), "127.0.0.1:1883");
    }
}
