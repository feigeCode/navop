//! MQTT 连接 trait。

use crate::pubsub::MqttPubSubHandle;
use crate::types::{MqttConnectionConfig, MqttError, MqttQos, MqttSubscription};
use async_trait::async_trait;

/// MQTT 连接抽象(进程内 builtin 或未来的 IPC sidecar 共用)
#[async_trait]
pub trait MqttConnection: Send + Sync {
    /// 获取配置
    fn config(&self) -> &MqttConnectionConfig;

    /// 建立连接
    async fn connect(&mut self) -> Result<(), MqttError>;

    /// 断开连接
    async fn disconnect(&mut self) -> Result<(), MqttError>;

    /// 测试连接(默认实现:connect + ping + disconnect)
    async fn ping(&self) -> Result<(), MqttError>;

    /// 是否已连接
    fn is_connected(&self) -> bool;

    /// 发布消息
    async fn publish(
        &self,
        topic: &str,
        payload: &[u8],
        qos: MqttQos,
        retain: bool,
    ) -> Result<(), MqttError>;

    /// 订阅主题
    async fn subscribe(&self, topic_filter: &str, qos: MqttQos) -> Result<(), MqttError>;

    /// 取消订阅
    async fn unsubscribe(&self, topic_filter: &str) -> Result<(), MqttError>;

    /// 当前订阅列表
    async fn list_subscriptions(&self) -> Result<Vec<MqttSubscription>, MqttError>;

    /// 打开消息流句柄(用于 UI 消费实时消息)
    fn open_pubsub(&self) -> Result<MqttPubSubHandle, MqttError>;
}
