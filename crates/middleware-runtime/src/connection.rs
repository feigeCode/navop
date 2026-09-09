//! 中间件标准管理接口:由 MQTT/RocketMQ 等后端 runtime 实现的统一契约。

use crate::types::{
    ClusterOverview, CreateTopicRequest, GroupConsumeDetail, MessagePage, MessageQuery,
    MiddlewareCapabilities, MiddlewareClientInfo, MiddlewareError, MiddlewareGroupInfo,
    MiddlewareMessage, MiddlewareMetrics, MiddlewareTopicInfo, SendMessageRequest, SendResult,
    TopicDetail,
};
use async_trait::async_trait;

/// 中间件标准管理接口。
///
/// 能力位约定:[`Self::capabilities`] 中声明为 false 的方法由调用方保证不调用
/// (通用视图按能力位隐藏入口);适配器实现仍应在无法满足能力时返回
/// [`MiddlewareError::Unsupported`] 作为防御兜底。
#[async_trait]
pub trait MiddlewareAdmin: Send + Sync {
    /// 声明后端支持的能力位
    fn capabilities(&self) -> MiddlewareCapabilities;

    /// 集群概览(cluster_overview 能力位)
    async fn cluster_overview(&self) -> Result<ClusterOverview, MiddlewareError>;

    /// 指标快照(metrics 能力位)
    async fn metrics_snapshot(&self) -> Result<MiddlewareMetrics, MiddlewareError>;

    /// Topic 列表(topics 能力位)
    async fn list_topics(&self) -> Result<Vec<MiddlewareTopicInfo>, MiddlewareError>;

    /// Topic 详情:各 Broker/队列偏移统计(topics 能力位)
    async fn topic_detail(&self, topic: &str) -> Result<TopicDetail, MiddlewareError>;

    /// 创建 Topic(topic_write 能力位)
    async fn create_topic(&self, req: CreateTopicRequest) -> Result<(), MiddlewareError>;

    /// 更新 Topic(topic_write 能力位)
    async fn update_topic(&self, req: CreateTopicRequest) -> Result<(), MiddlewareError>;

    /// 删除 Topic(topic_write 能力位)
    async fn delete_topic(&self, topic: &str) -> Result<(), MiddlewareError>;

    /// 发送消息(send_message 能力位)
    async fn send_message(&self, req: SendMessageRequest) -> Result<SendResult, MiddlewareError>;

    /// 订阅组列表(groups 能力位)
    async fn list_groups(&self) -> Result<Vec<MiddlewareGroupInfo>, MiddlewareError>;

    /// 订阅组客户端列表(clients 能力位)
    async fn group_clients(
        &self,
        group: &str,
    ) -> Result<Vec<MiddlewareClientInfo>, MiddlewareError>;

    /// 订阅组消费详情:按队列的消费进度与堆积(groups 能力位)
    async fn group_detail(&self, group: &str) -> Result<GroupConsumeDetail, MiddlewareError>;

    /// 消息查询(message_query 能力位)
    async fn query_messages(&self, query: MessageQuery) -> Result<MessagePage, MiddlewareError>;

    /// 消息详情(message_query 能力位)
    async fn message_detail(
        &self,
        topic: &str,
        message_id: &str,
    ) -> Result<MiddlewareMessage, MiddlewareError>;
}
