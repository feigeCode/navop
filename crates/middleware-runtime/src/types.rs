//! 中间件标准数据模型:能力位、Topic/订阅组/客户端/消息/集群/指标模型与统一错误。

use serde::{Deserialize, Serialize};

/// 中间件能力位:声明后端支持的管理功能,UI 据此显示或隐藏对应入口。
///
/// 能力位为 false 的 [`crate::MiddlewareAdmin`] 方法由调用方保证不调用;
/// 适配器在无法满足能力时应返回 [`MiddlewareError::Unsupported`] 兜底。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiddlewareCapabilities {
    /// 是否支持 Topic 列表与详情查询
    pub topics: bool,
    /// 是否支持 Topic 创建/更新/删除
    pub topic_write: bool,
    /// 是否支持订阅组列表与消费详情
    pub groups: bool,
    /// 是否支持订阅组客户端查询
    pub clients: bool,
    /// 是否支持消息查询
    pub message_query: bool,
    /// 是否支持发送消息
    pub send_message: bool,
    /// 是否支持指标快照
    pub metrics: bool,
    /// 是否支持集群概览
    pub cluster_overview: bool,
}

/// 标准 Topic 信息(列表页一行)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MiddlewareTopicInfo {
    /// Topic 名称
    pub name: String,
    /// Topic 类型通用文本(NORMAL/FIFO/DELAY/TRANSACTION/RETRY/DLQ/SYSTEM/SUBSCRIPTION 等)
    pub topic_type: Option<String>,
    /// 队列数量
    pub queue_count: Option<u32>,
    /// 读写权限文本(如 "6"=读写、"4"=只读、"2"=只写,或后端自定义文本)
    pub perm: Option<String>,
    /// 消息总量
    pub message_count: Option<u64>,
    /// 描述信息
    pub description: Option<String>,
    /// 创建时间
    pub created_at: Option<String>,
}

/// 单个队列的偏移统计
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueStat {
    /// 所在 Broker 名称
    pub broker: String,
    /// 队列 ID
    pub queue_id: i32,
    /// 最小偏移
    pub min_offset: i64,
    /// 最大偏移
    pub max_offset: i64,
    /// 最后更新时间
    pub last_update: Option<String>,
}

/// Topic 详情:按 Broker/队列维度的偏移统计
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TopicDetail {
    /// Topic 名称
    pub topic: String,
    /// 各队列统计
    pub stats: Vec<QueueStat>,
}

/// 标准订阅组信息(订阅组列表页一行)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MiddlewareGroupInfo {
    /// 订阅组名称
    pub group: String,
    /// 客户端数量
    pub client_count: Option<u32>,
    /// 消费类型(PUSH/PULL/BROADCAST)
    pub consume_type: Option<String>,
    /// 消息模型(CLUSTERING/BROADCASTING)
    pub message_model: Option<String>,
    /// 消费 TPS
    pub tps: Option<f64>,
    /// 堆积总量
    pub total_diff: Option<i64>,
    /// 客户端版本
    pub version: Option<String>,
    /// 更新时间
    pub update_time: Option<String>,
}

/// 订阅组内的客户端信息
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MiddlewareClientInfo {
    /// 客户端 ID
    pub client_id: String,
    /// 客户端地址
    pub client_addr: Option<String>,
    /// 客户端语言(如 JAVA/CPP/GO)
    pub language: Option<String>,
    /// 客户端版本
    pub version: Option<String>,
    /// 订阅的 Topic 列表
    pub subscriptions: Vec<String>,
}

/// 订阅组在单个队列上的消费进度
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupQueueStat {
    /// Topic 名称
    pub topic: String,
    /// 所在 Broker 名称
    pub broker: String,
    /// 队列 ID
    pub queue_id: i32,
    /// Broker 端偏移
    pub broker_offset: i64,
    /// 消费者已提交偏移
    pub consumer_offset: i64,
    /// 堆积量(broker_offset - consumer_offset)
    pub diff: i64,
}

/// 订阅组消费详情:按队列维度的消费进度与堆积
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupConsumeDetail {
    /// 订阅组名称
    pub group: String,
    /// 各队列消费进度
    pub queues: Vec<GroupQueueStat>,
}

/// 标准消息模型(查询结果)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MiddlewareMessage {
    /// 消息 ID
    pub message_id: String,
    /// 所属 Topic
    pub topic: String,
    /// 消息 Tag
    pub tag: Option<String>,
    /// 消息 Key
    pub key: Option<String>,
    /// 原始消息体(二进制)
    pub body: Option<Vec<u8>>,
    /// 消息体文本(UTF-8 可解码时提供)
    pub body_text: Option<String>,
    /// 存储时间
    pub store_time: Option<String>,
    /// 生成时间
    pub born_time: Option<String>,
    /// 存储 Broker 地址
    pub store_host: Option<String>,
    /// 生产者地址
    pub born_host: Option<String>,
    /// 重试次数
    pub retry_times: Option<u32>,
    /// 附加属性
    pub properties: Vec<(String, String)>,
}

/// 消息查询条件
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageQuery {
    /// 按时间窗口查询
    ByTimeWindow {
        /// Topic 名称
        topic: String,
        /// 开始时间(Unix 毫秒)
        begin_unix_ms: i64,
        /// 结束时间(Unix 毫秒)
        end_unix_ms: i64,
        /// 页码(从 1 开始)
        page: u32,
        /// 每页大小
        page_size: u32,
    },
    /// 按 Message Key 查询
    ByKey {
        /// Topic 名称
        topic: String,
        /// 消息 Key
        key: String,
    },
    /// 按 Message ID 查询
    ById {
        /// Topic 名称
        topic: String,
        /// 消息 ID
        message_id: String,
    },
}

/// 消息查询分页结果
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MessagePage {
    /// 当前页消息
    pub messages: Vec<MiddlewareMessage>,
    /// 结果总数(后端无法精确统计时可为估算值)
    pub total: u64,
    /// 是否还有更多页
    pub has_more: bool,
}

/// Broker 节点信息
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrokerInfo {
    /// Broker 名称
    pub name: String,
    /// Broker 地址
    pub address: String,
    /// Topic 数量
    pub topic_count: u64,
    /// 队列总数量
    pub queue_count: u64,
    /// Broker 版本(查询不到时为 None)
    pub version: Option<String>,
    /// 流入 TPS
    pub tps_in: f64,
    /// 流出 TPS
    pub tps_out: f64,
}

/// 集群信息
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClusterInfo {
    /// 集群名称
    pub name: String,
    /// Broker 列表
    pub brokers: Vec<BrokerInfo>,
}

/// 集群概览(概览页集群拓扑的标准表达)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClusterOverview {
    /// 集群列表
    pub clusters: Vec<ClusterInfo>,
}

/// 中间件运行指标快照(概览页指标卡片)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MiddlewareMetrics {
    /// 流入 TPS
    pub tps_in: f64,
    /// 流出 TPS
    pub tps_out: f64,
    /// Topic 总数
    pub topic_count: u64,
    /// 连接数(MQTT 为当前会话数,RocketMQ 为生产者/消费者连接数)
    pub connection_count: u64,
    /// 今日消息总量
    pub message_count_today: u64,
    /// 扩展指标(后端特有键值对)
    pub extras: Vec<(String, String)>,
}

/// 发送消息请求
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SendMessageRequest {
    /// 目标 Topic
    pub topic: String,
    /// 消息 Tag
    pub tag: Option<String>,
    /// 消息 Key
    pub key: Option<String>,
    /// 消息体
    pub body: Vec<u8>,
    /// 附加属性
    pub properties: Vec<(String, String)>,
}

/// 发送消息结果
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SendResult {
    /// 消息 ID
    pub message_id: String,
    /// 发送状态文本(如 OK/SLAVE_NOT_AVAILABLE)
    pub status: String,
}

/// 创建/更新 Topic 请求
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateTopicRequest {
    /// Topic 名称
    pub topic: String,
    /// Topic 类型(NORMAL/FIFO/DELAY/TRANSACTION 等,后端不支持类型概念时为 None)
    pub topic_type: Option<String>,
    /// 队列数量
    pub queue_count: Option<u32>,
    /// 读写权限文本
    pub perm: Option<String>,
    /// 扩展属性
    pub attributes: Vec<(String, String)>,
}

/// 中间件统一错误
#[derive(Debug, thiserror::Error)]
pub enum MiddlewareError {
    /// 配置错误(连接参数缺失或非法)
    #[error("配置错误: {0}")]
    Config(String),
    /// 协议错误(编解码或响应格式异常)
    #[error("协议错误: {0}")]
    Protocol(String),
    /// 不支持的操作(能力位未开放或后端无此概念)
    #[error("不支持的操作: {0}")]
    Unsupported(String),
    /// 操作超时
    #[error("操作超时: {0}")]
    Timeout(String),
    /// 连接错误(未连接/断连/IO 失败)
    #[error("连接错误: {0}")]
    Connection(String),
    /// 认证错误(鉴权失败或权限不足)
    #[error("认证错误: {0}")]
    Auth(String),
}

impl MiddlewareError {
    /// 构造"能力位未开放"的 Unsupported 错误
    pub fn unsupported_capability(capability: &str) -> Self {
        Self::Unsupported(format!("当前中间件不支持该能力: {capability}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// serde 往返断言辅助:序列化后再反序列化应得到相等结构
    fn assert_roundtrip<T>(value: &T)
    where
        T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("序列化失败");
        let back: T = serde_json::from_str(&json).expect("反序列化失败");
        assert_eq!(&back, value, "serde 往返不一致, json = {json}");
    }

    #[test]
    fn capabilities_default_all_false() {
        let caps = MiddlewareCapabilities::default();
        assert!(!caps.topics);
        assert!(!caps.topic_write);
        assert!(!caps.groups);
        assert!(!caps.clients);
        assert!(!caps.message_query);
        assert!(!caps.send_message);
        assert!(!caps.metrics);
        assert!(!caps.cluster_overview);
    }

    #[test]
    fn topic_models_roundtrip() {
        let topic = MiddlewareTopicInfo {
            name: "order-topic".into(),
            topic_type: Some("FIFO".into()),
            queue_count: Some(8),
            perm: Some("6".into()),
            message_count: Some(1024),
            description: Some("订单主题".into()),
            created_at: Some("2025-01-01 00:00:00".into()),
        };
        assert_roundtrip(&topic);

        let detail = TopicDetail {
            topic: "order-topic".into(),
            stats: vec![
                QueueStat {
                    broker: "broker-a".into(),
                    queue_id: 0,
                    min_offset: 0,
                    max_offset: 100,
                    last_update: Some("2025-01-01 00:00:00".into()),
                },
                QueueStat {
                    broker: "broker-b".into(),
                    queue_id: 1,
                    min_offset: 5,
                    max_offset: 0,
                    last_update: None,
                },
            ],
        };
        assert_roundtrip(&detail);
    }

    #[test]
    fn group_models_roundtrip() {
        let group = MiddlewareGroupInfo {
            group: "order-consumer".into(),
            client_count: Some(3),
            consume_type: Some("PUSH".into()),
            message_model: Some("CLUSTERING".into()),
            tps: Some(12.5),
            total_diff: Some(456),
            version: Some("V5_3_1".into()),
            update_time: Some("2025-01-02 08:00:00".into()),
        };
        assert_roundtrip(&group);

        let client = MiddlewareClientInfo {
            client_id: "10.0.0.1@1234".into(),
            client_addr: Some("10.0.0.1:28000".into()),
            language: Some("JAVA".into()),
            version: Some("V5_3_1".into()),
            subscriptions: vec!["order-topic".into(), "pay-topic".into()],
        };
        assert_roundtrip(&client);

        let detail = GroupConsumeDetail {
            group: "order-consumer".into(),
            queues: vec![GroupQueueStat {
                topic: "order-topic".into(),
                broker: "broker-a".into(),
                queue_id: 0,
                broker_offset: 1000,
                consumer_offset: 990,
                diff: 10,
            }],
        };
        assert_roundtrip(&detail);
    }

    #[test]
    fn message_models_roundtrip() {
        let message = MiddlewareMessage {
            message_id: "0A0A0A0A00001234".into(),
            topic: "order-topic".into(),
            tag: Some("create".into()),
            key: Some("order-1".into()),
            body: Some(vec![0xEF, 0xBB, 0xBF, 0xE8, 0xAE, 0xA2]),
            body_text: Some("订单".into()),
            store_time: Some("2025-01-02 08:00:00".into()),
            born_time: Some("2025-01-02 07:59:59".into()),
            store_host: Some("10.0.0.2:10911".into()),
            born_host: Some("10.0.0.1:52300".into()),
            retry_times: Some(2),
            properties: vec![
                ("KEYS".into(), "order-1".into()),
                ("UNIQ_KEY".into(), "0A0A0A0A00001234".into()),
            ],
        };
        assert_roundtrip(&message);

        let page = MessagePage {
            messages: vec![message],
            total: 42,
            has_more: true,
        };
        assert_roundtrip(&page);
    }

    #[test]
    fn message_query_variants_serialize() {
        let by_time = MessageQuery::ByTimeWindow {
            topic: "order-topic".into(),
            begin_unix_ms: 1_735_689_600_000,
            end_unix_ms: 1_735_780_000_000,
            page: 1,
            page_size: 20,
        };
        let json = serde_json::to_string(&by_time).expect("序列化失败");
        assert!(
            json.starts_with(r#"{"ByTimeWindow":"#),
            "时间窗口变体应为外标签格式: {json}"
        );
        assert_roundtrip(&by_time);

        let by_key = MessageQuery::ByKey {
            topic: "order-topic".into(),
            key: "order-1".into(),
        };
        assert_roundtrip(&by_key);

        let by_id = MessageQuery::ById {
            topic: "order-topic".into(),
            message_id: "0A0A0A0A00001234".into(),
        };
        assert_roundtrip(&by_id);
    }

    #[test]
    fn cluster_and_metrics_roundtrip() {
        let overview = ClusterOverview {
            clusters: vec![ClusterInfo {
                name: "DefaultCluster".into(),
                brokers: vec![BrokerInfo {
                    name: "broker-a".into(),
                    address: "10.0.0.2:10911".into(),
                    topic_count: 120,
                    queue_count: 960,
                    version: Some("V5_3_1".into()),
                    tps_in: 100.5,
                    tps_out: 98.25,
                }],
            }],
        };
        assert_roundtrip(&overview);

        let metrics = MiddlewareMetrics {
            tps_in: 100.5,
            tps_out: 98.25,
            topic_count: 120,
            connection_count: 45,
            message_count_today: 8_640_000,
            extras: vec![("broker_count".into(), "2".into())],
        };
        assert_roundtrip(&metrics);
    }

    #[test]
    fn send_and_create_topic_roundtrip() {
        let request = SendMessageRequest {
            topic: "order-topic".into(),
            tag: Some("create".into()),
            key: Some("order-1".into()),
            body: b"{\"orderId\":1}".to_vec(),
            properties: vec![("TRACE".into(), "true".into())],
        };
        assert_roundtrip(&request);

        let result = SendResult {
            message_id: "0A0A0A0A00001234".into(),
            status: "OK".into(),
        };
        assert_roundtrip(&result);

        let create = CreateTopicRequest {
            topic: "new-topic".into(),
            topic_type: Some("NORMAL".into()),
            queue_count: Some(8),
            perm: Some("6".into()),
            attributes: vec![("perm".into(), "6".into())],
        };
        assert_roundtrip(&create);
    }

    #[test]
    fn models_tolerate_missing_fields() {
        // 契约向前兼容:JSON 缺失字段时按默认值反序列化,新增字段不破坏旧数据
        let topic: MiddlewareTopicInfo =
            serde_json::from_str(r#"{"name":"only-name"}"#).expect("缺失字段应可反序列化");
        assert_eq!(topic.name, "only-name");
        assert_eq!(topic.topic_type, None);
        assert_eq!(topic.queue_count, None);

        let page: MessagePage = serde_json::from_str("{}").expect("空对象应可反序列化");
        assert!(page.messages.is_empty());
        assert_eq!(page.total, 0);
        assert!(!page.has_more);
    }

    #[test]
    fn unsupported_capability_message() {
        let err = MiddlewareError::unsupported_capability("cluster_overview");
        assert!(err.to_string().contains("cluster_overview"));
        match &err {
            MiddlewareError::Unsupported(_) => {}
            other => panic!("应为 Unsupported 变体, 实际: {other:?}"),
        }
    }
}
