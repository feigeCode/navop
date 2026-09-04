//! MQTT 实时消息流句柄(基于 tokio broadcast)。

use crate::types::MqttMessage;
use tokio::sync::broadcast;

/// 消息广播通道容量;lagged 时丢弃旧消息。
pub const MQTT_MESSAGE_CHANNEL_CAPACITY: usize = 1024;

/// MQTT 实时消息流接收端。
///
/// 由 [`crate::connection::MqttConnection::open_pubsub`] 创建;
/// 每个需要消费实时消息的 UI 视图应各自调用 `open_pubsub` 获取独立接收端。
/// 连接断开(drop sender)后 `recv` 返回 `None`。
pub struct MqttPubSubHandle {
    receiver: broadcast::Receiver<MqttMessage>,
}

impl MqttPubSubHandle {
    #[cfg_attr(not(feature = "builtin-mqtt"), allow(dead_code))]
    pub(crate) fn new(receiver: broadcast::Receiver<MqttMessage>) -> Self {
        Self { receiver }
    }

    /// 接收下一条消息;通道关闭(连接断开)时返回 `None`。
    pub async fn recv(&mut self) -> Option<MqttMessage> {
        loop {
            match self.receiver.recv().await {
                Ok(message) => return Some(message),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}
