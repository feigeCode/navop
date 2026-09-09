//! RocketMQ 管理实现:落地 `middleware_runtime::MiddlewareAdmin` 契约。
//!
//! 数据流对齐 apache/rocketmq-dashboard(控制台)的管理员调用路径:
//! - 集群概览/路由:NameServer `GET_BROKER_CLUSTER_INFO`(106)/`GET_ROUTEINFO_BY_TOPIC`(105)
//! - Topic:NameServer `GET_ALL_TOPIC_LIST_FROM_NAMESERVER`(206)+
//!   Broker `GET_ALL_TOPIC_CONFIG`(21) 补全队列/权限;写操作
//!   `UPDATE_AND_CREATE_TOPIC`(17)/`DELETE_TOPIC_IN_BROKER`(215)/`DELETE_TOPIC_IN_NAMESRV`(216)
//! - 订阅组:Broker `GET_ALL_SUBSCRIPTIONGROUP_CONFIG`(201)/
//!   `GET_CONSUME_STATS`(208)/`GET_CONSUMER_CONNECTION_LIST`(203)
//! - 消息:`QUERY_MESSAGE`(12)按 Key、`VIEW_MESSAGE_BY_ID`(33)按偏移(commitlog 二进制)、
//!   时间窗口经 `SEARCH_OFFSET_BY_TIMESTAMP`(29)+`QUERY_CONSUME_QUEUE`(321)遍历

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::message::{StoredMessage, decode_stored_message, parse_msg_id};
use crate::protocol::dto::{
    ClusterInfoBody, ConsumeStatsBody, ConsumerConnectionBody, KvTableBody, MessageExtBody,
    QueryConsumeQueueResponseBodyBody, QueryResultBody, QueueData, SubscriptionGroupWrapperBody,
    TopicConfigBody, TopicConfigSerializeWrapperBody, TopicListBody, TopicRouteData,
    TopicStatsTableBody,
};
use crate::protocol::{RemotingCommand, RequestCode, ResponseCode};
use crate::remoting::RemotingClient;
use crate::types::RocketmqParams;
use async_trait::async_trait;
use chrono::{Local, TimeZone};
use middleware_runtime::types::{
    BrokerInfo, ClusterInfo, ClusterOverview, CreateTopicRequest, GroupConsumeDetail,
    GroupQueueStat, MessagePage, MessageQuery, MiddlewareCapabilities, MiddlewareClientInfo,
    MiddlewareError, MiddlewareGroupInfo, MiddlewareMessage, MiddlewareMetrics,
    MiddlewareTopicInfo, QueueStat, SendMessageRequest, SendResult, TopicDetail,
};

/// %RETRY% 重试队列 Topic 前缀
const RETRY_TOPIC_PREFIX: &str = "%RETRY%";
/// %DLQ% 死信队列 Topic 前缀
const DLQ_TOPIC_PREFIX: &str = "%DLQ%";
/// 系统 Topic 前缀(官方 `TopicValidator.SYSTEM_TOPIC_PREFIX`)
const SYSTEM_TOPIC_PREFIX: &str = "rmq_sys";
/// 控制台建 Topic 使用的默认模板 Topic(官方 `MixAll.DEFAULT_TOPIC`)
const DEFAULT_TOPIC_KEY: &str = "TBW102";
/// 订阅组在线度统计的抽样上限(超过则跳过逐组连接查询,避免请求风暴)
const GROUP_ENRICH_LIMIT: usize = 64;
/// 消息查询单页上限
const MESSAGE_PAGE_MAX: u32 = 100;
/// 无类型信息时的 Topic 类型标记
const TOPIC_TYPE_UNSPECIFIED: &str = "UNSPECIFIED";

/// RocketMQ 管理连接
pub struct RocketmqConnection {
    client: RemotingClient,
    /// 发送消息的轮询计数(队列选择)
    send_cursor: AtomicUsize,
}

impl RocketmqConnection {
    /// 创建管理连接(惰性连接,首次调用时建立)
    pub fn new(params: Arc<RocketmqParams>) -> Result<Self, MiddlewareError> {
        Ok(Self {
            client: RemotingClient::new(params)?,
            send_cursor: AtomicUsize::new(0),
        })
    }

    /// 测试连接:向 NameServer 拉取集群信息,能收到响应即成功
    pub async fn test_connection(&self) -> Result<(), MiddlewareError> {
        let request =
            RemotingCommand::create_request(RequestCode::GET_BROKER_CLUSTER_INFO, HashMap::new());
        self.client.invoke_namesrv(request).await.map(|_| ())
    }

    /// 关闭底层通道
    pub async fn close(&self) {
        self.client.close().await;
    }

    /// 拉取集群信息(全部集群 → Broker 主备组)
    async fn cluster_info(&self) -> Result<ClusterInfoBody, MiddlewareError> {
        let request =
            RemotingCommand::create_request(RequestCode::GET_BROKER_CLUSTER_INFO, HashMap::new());
        let response = self.client.invoke_namesrv(request).await?;
        decode_body(&response)
    }

    /// 拉取 Topic 路由
    async fn topic_route(&self, topic: &str) -> Result<TopicRouteData, MiddlewareError> {
        let mut ext = HashMap::new();
        ext.insert("topic".to_string(), topic.to_string());
        let request = RemotingCommand::create_request(RequestCode::GET_ROUTEINFO_BY_TOPIC, ext);
        let response = self.client.invoke_namesrv(request).await?;
        decode_body(&response)
    }

    /// 全部 Broker 主节点地址(按集群信息推导)
    async fn master_addrs(&self) -> Result<Vec<String>, MiddlewareError> {
        let cluster = self.cluster_info().await?;
        let mut addrs: Vec<String> = Vec::new();
        for broker in cluster.broker_addr_table.values() {
            if let Some(addr) = broker.select_broker_addr() {
                if !addrs.contains(&addr.to_string()) {
                    addrs.push(addr.to_string());
                }
            }
        }
        // 兜底:NameServer 直连场景(极少见)
        if addrs.is_empty() {
            return Err(MiddlewareError::Connection(
                "集群中未发现可用 Broker 主节点".into(),
            ));
        }
        Ok(addrs)
    }

    /// Broker 上全部 Topic 配置(取第一个可达主节点)
    async fn all_topic_configs(&self) -> Result<HashMap<String, TopicConfigBody>, MiddlewareError> {
        let mut last_error = None;
        for addr in self.master_addrs().await? {
            let request =
                RemotingCommand::create_request(RequestCode::GET_ALL_TOPIC_CONFIG, HashMap::new());
            match self.client.invoke_broker(&addr, request).await {
                Ok(response) => {
                    let wrapper: TopicConfigSerializeWrapperBody = decode_body(&response)?;
                    return Ok(wrapper.topic_config_table);
                }
                Err(error) => {
                    tracing::warn!("Broker {addr} 拉取 Topic 配置失败: {error}");
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            MiddlewareError::Connection("拉取 Topic 配置失败: 无可用 Broker".into())
        }))
    }

    /// Broker 运行时信息(逐主节点)
    async fn runtime_infos(&self, addrs: &[String]) -> HashMap<String, KvTableBody> {
        let mut infos = HashMap::new();
        for addr in addrs {
            let request = RemotingCommand::create_request(
                RequestCode::GET_BROKER_RUNTIME_INFO,
                HashMap::new(),
            );
            if let Ok(response) = self.client.invoke_broker(addr, request).await {
                if let Ok(table) = decode_body::<KvTableBody>(&response) {
                    infos.insert(addr.clone(), table);
                    continue;
                }
            }
            // 失败时保留空表,统计按 0 处理
            infos.insert(addr.clone(), KvTableBody::default());
        }
        infos
    }

    /// Topic 类型推断:%RETRY%/%DLQ%/系统前缀 → 对应类型;5.x 属性表优先
    fn infer_topic_type(name: &str, config: Option<&TopicConfigBody>) -> Option<String> {
        if let Some(attributes) = config.and_then(|item| item.attributes.as_ref()) {
            if let Some(kind) = attributes.get("+type").or_else(|| attributes.get("type")) {
                return Some(kind.clone());
            }
        }
        if name.starts_with(RETRY_TOPIC_PREFIX) {
            Some("RETRY".into())
        } else if name.starts_with(DLQ_TOPIC_PREFIX) {
            Some("DLQ".into())
        } else if is_system_topic(name) {
            Some("SYSTEM".into())
        } else {
            // 4.x 无类型信息
            Some(TOPIC_TYPE_UNSPECIFIED.into())
        }
    }

    /// 按路由选择发送队列(可写 QueueData 的轮询)
    fn select_send_queue<'a>(&self, route: &'a TopicRouteData) -> Option<(&'a QueueData, i32)> {
        let writable: Vec<&QueueData> = route
            .queue_datas
            .iter()
            .filter(|queue| queue.perm & 0x02 != 0 && queue.write_queue_nums > 0)
            .collect();
        let queue = writable.first()?;
        let cursor = self.send_cursor.fetch_add(1, Ordering::Relaxed);
        let queue_id = (cursor % queue.write_queue_nums as usize) as i32;
        Some((queue, queue_id))
    }

    /// 通过 VIEW_MESSAGE_BY_ID 拉取完整消息(commitlog 二进制 body)
    async fn view_message_by_offset(
        &self,
        broker_addr: &str,
        topic: &str,
        physic_offset: i64,
    ) -> Result<StoredMessage, MiddlewareError> {
        let mut ext = HashMap::new();
        ext.insert("topic".to_string(), topic.to_string());
        ext.insert("offset".to_string(), physic_offset.to_string());
        let request = RemotingCommand::create_request(RequestCode::VIEW_MESSAGE_BY_ID, ext);
        let response = self.client.invoke_broker(broker_addr, request).await?;
        let body = response
            .body
            .ok_or_else(|| MiddlewareError::Protocol("VIEW_MESSAGE_BY_ID 响应缺少 body".into()))?;
        decode_stored_message(&body)
    }

    /// MessageExtBody(JSON 查询结果) → 标准消息模型
    fn message_ext_to_model(message: &MessageExtBody) -> MiddlewareMessage {
        let properties = message.properties.clone().unwrap_or_default();
        let body = message.body.as_deref().and_then(|text| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(text).ok()
        });
        MiddlewareMessage {
            message_id: message
                .msg_id
                .clone()
                .or_else(|| properties.get("UNIQ_KEY").cloned())
                .unwrap_or_default(),
            topic: message.topic.clone(),
            tag: properties.get("TAGS").cloned(),
            key: properties.get("KEYS").cloned(),
            body_text: body
                .as_deref()
                .and_then(|bytes| String::from_utf8(bytes.to_vec()).ok()),
            body,
            store_time: Some(format_unix_ms(message.store_timestamp)),
            born_time: Some(format_unix_ms(message.born_timestamp)),
            store_host: Some(json_value_to_addr(&message.store_host)),
            born_host: Some(json_value_to_addr(&message.born_host)),
            retry_times: Some(message.reconsume_times.max(0) as u32),
            properties: {
                let mut items: Vec<(String, String)> = properties.into_iter().collect();
                items.sort();
                items
            },
        }
    }

    /// StoredMessage(commitlog) → 标准消息模型
    fn stored_to_model(message: &StoredMessage) -> MiddlewareMessage {
        MiddlewareMessage {
            message_id: message
                .properties
                .get("UNIQ_KEY")
                .cloned()
                .unwrap_or_else(|| message.offset_msg_id()),
            topic: message.topic.clone(),
            tag: message.properties.get("TAGS").cloned(),
            key: message.properties.get("KEYS").cloned(),
            body_text: String::from_utf8(message.body.clone()).ok(),
            body: Some(message.body.clone()),
            store_time: Some(format_unix_ms(message.store_timestamp)),
            born_time: Some(format_unix_ms(message.born_timestamp)),
            store_host: Some(message.store_host.clone()),
            born_host: Some(message.born_host.clone()),
            retry_times: Some(message.reconsume_times.max(0) as u32),
            properties: {
                let mut items: Vec<(String, String)> =
                    message.properties.clone().into_iter().collect();
                items.sort();
                items
            },
        }
    }
}

/// JSON 值(bornHost/storeHost 可能是字符串或对象)转 `ip:port` 文本
fn json_value_to_addr(value: &Option<serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(text)) => text.trim_start_matches('/').to_string(),
        Some(serde_json::Value::Object(map)) => {
            let host = map
                .get("address")
                .or_else(|| map.get("hostString"))
                .or_else(|| map.get("host"))
                .and_then(|item| item.as_str())
                .unwrap_or("unknown");
            let port = map.get("port").and_then(|item| item.as_u64()).unwrap_or(0);
            format!("{host}:{port}")
        }
        _ => String::new(),
    }
}

/// 是否系统 Topic(rmq_sys 前缀与常见内置 Topic)
fn is_system_topic(name: &str) -> bool {
    name.starts_with(SYSTEM_TOPIC_PREFIX)
        || name.starts_with('%')
        || matches!(
            name,
            "TBW102"
                | "SELF_TEST_TOPIC"
                | "BenchmarkTest"
                | "OFFSET_MOVED_EVENT"
                | "SCHEDULE_TOPIC_1931"
        )
}

/// Unix 毫秒 → 本地时间文本
fn format_unix_ms(timestamp_ms: i64) -> String {
    if timestamp_ms <= 0 {
        return String::new();
    }
    match Local.timestamp_millis_opt(timestamp_ms) {
        chrono::LocalResult::Single(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
        _ => String::new(),
    }
}

/// 响应 body JSON → DTO
fn decode_body<T: serde::de::DeserializeOwned>(
    response: &RemotingCommand,
) -> Result<T, MiddlewareError> {
    let body = response
        .body
        .as_deref()
        .ok_or_else(|| MiddlewareError::Protocol("响应缺少 body".into()))?;
    serde_json::from_slice(body)
        .map_err(|error| MiddlewareError::Protocol(format!("响应 body 解析失败: {error}")))
}

/// 运行时 TPS 文本(" 0.0 0.0 0.0" 形态)取首个数值
fn parse_tps(value: Option<&String>) -> f64 {
    value
        .and_then(|text| {
            text.split_whitespace()
                .next()
                .and_then(|first| first.parse::<f64>().ok())
        })
        .unwrap_or(0.0)
}

/// 官方消费类型枚举转通用文本。
///
/// 对照 release-4.9.4 `org.apache.rocketmq.common.protocol.heartbeat.ConsumeType`:
/// `CONSUME_ACTIVELY("PULL")` 主动(拉取式)消费、`CONSUME_PASSIVELY("PUSH")`
/// 被动(推送式)消费;未知值原样透传。
fn consume_type_text(kind: &str) -> String {
    match kind {
        "CONSUME_ACTIVELY" => "PULL".to_string(),
        "CONSUME_PASSIVELY" => "PUSH".to_string(),
        other => other.to_string(),
    }
}

/// 解析 perm 文本("6"/"4"/"2";兼容 "rw"/"r"/"w" 简写),默认读写
fn parse_perm(text: &str) -> i32 {
    match text.trim() {
        "6" | "rw" | "RW" => 6,
        "4" | "r" | "R" => 4,
        "2" | "w" | "W" => 2,
        other => other.parse().unwrap_or(6),
    }
}

/// Topic 类型 → 5.x attributes 表(4.x 服务端忽略未知字段)
fn topic_type_attributes(topic_type: Option<&str>) -> Option<String> {
    let kind = topic_type?.trim().to_ascii_uppercase();
    if kind.is_empty() || kind == TOPIC_TYPE_UNSPECIFIED || kind == "NORMAL" {
        return None;
    }
    match kind.as_str() {
        "FIFO" | "DELAY" | "TRANSACTION" => Some(format!("\"+type\"=\"{kind}\"")),
        _ => None,
    }
}

#[async_trait]
impl middleware_runtime::MiddlewareAdmin for RocketmqConnection {
    /// RocketMQ 管理面板能力全开
    fn capabilities(&self) -> MiddlewareCapabilities {
        MiddlewareCapabilities {
            topics: true,
            topic_write: true,
            groups: true,
            clients: true,
            message_query: true,
            send_message: true,
            metrics: true,
            cluster_overview: true,
        }
    }

    /// 集群概览:NameServer 集群信息 + 各 Broker Topic/队列统计
    async fn cluster_overview(&self) -> Result<ClusterOverview, MiddlewareError> {
        let cluster = self.cluster_info().await?;
        let topic_configs = self.all_topic_configs().await.unwrap_or_default();

        let mut clusters: Vec<ClusterInfo> = Vec::new();
        for (cluster_name, broker_names) in &cluster.cluster_addr_table {
            let mut brokers = Vec::new();
            for broker_name in broker_names {
                let Some(broker_data) = cluster.broker_addr_table.get(broker_name) else {
                    continue;
                };
                let Some(master_addr) = broker_data.select_broker_addr() else {
                    continue;
                };
                let runtime = self
                    .runtime_infos(std::slice::from_ref(&master_addr.to_string()))
                    .await
                    .remove(master_addr)
                    .unwrap_or_default();
                // Topic 配置按 brokerName 分组统计(同名主备组共享 Topic 集)
                let (topic_count, queue_count) =
                    topic_configs.values().fold((0u64, 0u64), |acc, config| {
                        let queue = config.write_queue_nums.max(0) as u64;
                        (acc.0 + 1, acc.1 + queue)
                    });
                brokers.push(BrokerInfo {
                    name: broker_name.clone(),
                    address: master_addr.to_string(),
                    topic_count,
                    queue_count,
                    version: runtime.table.get("brokerVersionDesc").cloned(),
                    tps_in: parse_tps(runtime.table.get("putTps")),
                    tps_out: parse_tps(runtime.table.get("getTransferedTps")),
                });
            }
            clusters.push(ClusterInfo {
                name: cluster_name.clone(),
                brokers,
            });
        }
        Ok(ClusterOverview { clusters })
    }

    /// 指标快照:聚合 Topic 数/TPS/今日消息量与连接数
    async fn metrics_snapshot(&self) -> Result<MiddlewareMetrics, MiddlewareError> {
        let topic_list = self.list_topic_names().await?;
        let master_addrs = self.master_addrs().await?;
        let runtimes = self.runtime_infos(&master_addrs).await;

        let mut tps_in = 0.0;
        let mut tps_out = 0.0;
        let mut message_today = 0u64;
        let mut broker_count = 0u64;
        for table in runtimes.values() {
            broker_count += 1;
            tps_in += parse_tps(table.table.get("putTps"));
            tps_out += parse_tps(table.table.get("getTransferedTps"));
            if let Some(total) = table
                .table
                .get("msgPutTotalTodayNow")
                .and_then(|text| text.parse::<u64>().ok())
            {
                message_today += total;
            }
        }

        // 连接数:有界抽样订阅组在线客户端(超过上限跳过)
        let mut connection_count = 0u64;
        if let Ok(groups) = self.list_group_names(&master_addrs).await {
            if groups.len() <= GROUP_ENRICH_LIMIT {
                for group in &groups {
                    if let Ok(connection) = self.consumer_connection(&master_addrs[0], group).await
                    {
                        connection_count += connection.connection_set.len() as u64;
                    }
                }
            }
        }

        Ok(MiddlewareMetrics {
            tps_in,
            tps_out,
            topic_count: topic_list.len() as u64,
            connection_count,
            message_count_today: message_today,
            extras: vec![
                ("broker_count".into(), broker_count.to_string()),
                ("msg_put_total_today".into(), message_today.to_string()),
            ],
        })
    }

    /// Topic 列表:NameServer 全量列表 + Broker 配置补全队列/权限
    async fn list_topics(&self) -> Result<Vec<MiddlewareTopicInfo>, MiddlewareError> {
        let names = self.list_topic_names().await?;
        let configs = self.all_topic_configs().await.unwrap_or_default();
        let mut topics: Vec<MiddlewareTopicInfo> = names
            .iter()
            .map(|name| {
                let config = configs.get(name);
                MiddlewareTopicInfo {
                    name: name.clone(),
                    topic_type: Self::infer_topic_type(name, config),
                    queue_count: config.map(|item| item.read_queue_nums.max(0) as u32),
                    perm: config.map(|item| item.perm_text()),
                    message_count: None,
                    description: None,
                    created_at: None,
                }
            })
            .collect();
        topics.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(topics)
    }

    /// Topic 详情:各 Broker/队列偏移统计
    async fn topic_detail(&self, topic: &str) -> Result<TopicDetail, MiddlewareError> {
        let route = self.topic_route(topic).await?;
        let mut stats = Vec::new();
        for broker in &route.broker_datas {
            let Some(addr) = broker.select_broker_addr() else {
                continue;
            };
            let mut ext = HashMap::new();
            ext.insert("topic".to_string(), topic.to_string());
            let request = RemotingCommand::create_request(RequestCode::GET_TOPIC_STATS_INFO, ext);
            let Ok(response) = self.client.invoke_broker(addr, request).await else {
                continue;
            };
            let table: TopicStatsTableBody = decode_body(&response)?;
            for (queue, offset) in table.offset_entries() {
                stats.push(QueueStat {
                    broker: queue.broker_name,
                    queue_id: queue.queue_id,
                    min_offset: offset.min_offset,
                    max_offset: offset.max_offset,
                    last_update: Some(format_unix_ms(offset.last_update_timestamp))
                        .filter(|text| !text.is_empty()),
                });
            }
        }
        stats.sort_by(|a, b| (&a.broker, a.queue_id).cmp(&(&b.broker, b.queue_id)));
        Ok(TopicDetail {
            topic: topic.to_string(),
            stats,
        })
    }

    /// 创建 Topic:向全部 Broker 主节点下发(4.x upsert 语义)
    async fn create_topic(&self, req: CreateTopicRequest) -> Result<(), MiddlewareError> {
        self.update_topic(req).await
    }

    /// 更新 Topic(官方 UPDATE_AND_CREATE_TOPIC 同一请求码)
    async fn update_topic(&self, req: CreateTopicRequest) -> Result<(), MiddlewareError> {
        let queue_count = req.queue_count.unwrap_or(8).max(1) as i32;
        let perm = parse_perm(req.perm.as_deref().unwrap_or("6"));
        let mut ext = HashMap::new();
        ext.insert("topic".to_string(), req.topic.clone());
        ext.insert("defaultTopic".to_string(), DEFAULT_TOPIC_KEY.to_string());
        ext.insert("readQueueNums".to_string(), queue_count.to_string());
        ext.insert("writeQueueNums".to_string(), queue_count.to_string());
        ext.insert("perm".to_string(), perm.to_string());
        ext.insert("topicFilterType".to_string(), "SINGLE_TAG".to_string());
        ext.insert("topicSysFlag".to_string(), "0".to_string());
        ext.insert("order".to_string(), "false".to_string());
        if let Some(attributes) = topic_type_attributes(req.topic_type.as_deref()) {
            ext.insert("attributes".to_string(), attributes);
        }

        let masters = self.master_addrs().await?;
        let mut last_error = None;
        let mut succeeded = 0usize;
        for addr in &masters {
            let request =
                RemotingCommand::create_request(RequestCode::UPDATE_AND_CREATE_TOPIC, ext.clone());
            match self.client.invoke_broker(addr, request).await {
                Ok(_) => succeeded += 1,
                Err(error) => {
                    tracing::warn!("Broker {addr} 创建 Topic {} 失败: {error}", req.topic);
                    last_error = Some(error);
                }
            }
        }
        if succeeded == 0 {
            return Err(last_error.unwrap_or_else(|| {
                MiddlewareError::Connection("创建 Topic 失败: 无可用 Broker".into())
            }));
        }
        Ok(())
    }

    /// 删除 Topic:Broker + NameServer 两级清理
    async fn delete_topic(&self, topic: &str) -> Result<(), MiddlewareError> {
        let masters = self.master_addrs().await?;
        let mut last_error = None;
        let mut succeeded = 0usize;

        let mut ext = HashMap::new();
        ext.insert("topic".to_string(), topic.to_string());
        for addr in &masters {
            let request =
                RemotingCommand::create_request(RequestCode::DELETE_TOPIC_IN_BROKER, ext.clone());
            match self.client.invoke_broker(addr, request).await {
                Ok(_) => succeeded += 1,
                Err(error) => {
                    tracing::warn!("Broker {addr} 删除 Topic {topic} 失败: {error}");
                    last_error = Some(error);
                }
            }
        }
        // NameServer 路由删除(逐地址尽力而为)
        for addr in self.client.params().normalized_namesrv_addrs() {
            let mut ns_ext = HashMap::new();
            ns_ext.insert("topic".to_string(), topic.to_string());
            let mut request =
                RemotingCommand::create_request(RequestCode::DELETE_TOPIC_IN_NAMESRV, ns_ext);
            if let Err(error) = self.client.invoke_addr(&addr, &mut request).await {
                tracing::warn!("NameServer {addr} 删除 Topic {topic} 失败: {error}");
                last_error = Some(error);
            }
        }
        if succeeded == 0 {
            return Err(last_error.unwrap_or_else(|| {
                MiddlewareError::Connection("删除 Topic 失败: 无可用 Broker".into())
            }));
        }
        Ok(())
    }

    /// 发送消息:按路由选队列,SEND_MESSAGE_V2(310) 优先,V1(10) 兜底
    async fn send_message(&self, req: SendMessageRequest) -> Result<SendResult, MiddlewareError> {
        let route = self.topic_route(&req.topic).await?;
        let Some((queue, queue_id)) = self.select_send_queue(&route) else {
            return Err(MiddlewareError::Protocol(format!(
                "Topic {} 无可写队列",
                req.topic
            )));
        };
        let broker_name = &queue.broker_name;
        let addr = route
            .broker_datas
            .iter()
            .find(|broker| &broker.broker_name == broker_name)
            .and_then(|broker| broker.select_broker_addr().map(str::to_string))
            .ok_or_else(|| {
                MiddlewareError::Protocol(format!("Topic {} 路由缺少 Broker 地址", req.topic))
            })?;

        // 消息属性:TAGS/KEYS + 附加属性(官方 properties 串 "k=v;k=v")
        let mut properties: BTreeMap<String, String> = BTreeMap::new();
        if let Some(tag) = req.tag.as_deref().filter(|tag| !tag.trim().is_empty()) {
            properties.insert("TAGS".into(), tag.to_string());
        }
        if let Some(key) = req.key.as_deref().filter(|key| !key.trim().is_empty()) {
            properties.insert("KEYS".into(), key.to_string());
        }
        for (key, value) in &req.properties {
            properties.insert(key.clone(), value.clone());
        }
        let properties_text = properties
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(";");

        let born_timestamp = chrono::Utc::now().timestamp_millis();
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let producer_group = format!("navop_admin_{}", &uid[..8]);

        // V2 短字段头(a=producerGroup b=topic c=defaultTopic d=defaultTopicQueueNums
        // e=queueId f=sysFlag g=bornTimestamp h=flag i=properties)
        let build_v2 = || {
            let mut ext = HashMap::new();
            ext.insert("a".to_string(), producer_group.clone());
            ext.insert("b".to_string(), req.topic.clone());
            ext.insert("c".to_string(), DEFAULT_TOPIC_KEY.to_string());
            ext.insert("d".to_string(), "8".to_string());
            ext.insert("e".to_string(), queue_id.to_string());
            ext.insert("f".to_string(), "0".to_string());
            ext.insert("g".to_string(), born_timestamp.to_string());
            ext.insert("h".to_string(), "0".to_string());
            ext.insert("i".to_string(), properties_text.clone());
            RemotingCommand::create_request(RequestCode::SEND_MESSAGE_V2, ext)
                .with_body(req.body.clone())
        };
        let build_v1 = || {
            let mut ext = HashMap::new();
            ext.insert("producerGroup".to_string(), producer_group.clone());
            ext.insert("topic".to_string(), req.topic.clone());
            ext.insert("defaultTopic".to_string(), DEFAULT_TOPIC_KEY.to_string());
            ext.insert("defaultTopicQueueNums".to_string(), "8".to_string());
            ext.insert("queueId".to_string(), queue_id.to_string());
            ext.insert("sysFlag".to_string(), "0".to_string());
            ext.insert("bornTimestamp".to_string(), born_timestamp.to_string());
            ext.insert("flag".to_string(), "0".to_string());
            ext.insert("properties".to_string(), properties_text.clone());
            RemotingCommand::create_request(RequestCode::SEND_MESSAGE, ext)
                .with_body(req.body.clone())
        };

        let response = match self.client.invoke_broker_raw(&addr, build_v2()).await {
            Ok(response) if response.code != ResponseCode::REQUEST_CODE_NOT_SUPPORTED => response,
            _ => self.client.invoke_broker(&addr, build_v1()).await?,
        };

        // 4.x SLAVE_NOT_AVAILABLE(11) 表示写入成功但同步失败
        let status = match response.code {
            0 => "OK".to_string(),
            11 => "SLAVE_NOT_AVAILABLE".to_string(),
            other => {
                return Err(MiddlewareError::Protocol(format!(
                    "发送失败 code={other}: {}",
                    response.remark.clone().unwrap_or_default()
                )));
            }
        };
        // 响应 ext:msgId/queueId/queueOffset(V1/V2 响应均用长字段名)
        let msg_id = response
            .ext_fields
            .get("msgId")
            .cloned()
            .or_else(|| properties.get("UNIQ_KEY").cloned())
            .unwrap_or_default();
        Ok(SendResult {
            message_id: msg_id,
            status,
        })
    }

    /// 订阅组列表:全部订阅组配置 + 有界在线度/堆积抽样
    async fn list_groups(&self) -> Result<Vec<MiddlewareGroupInfo>, MiddlewareError> {
        let masters = self.master_addrs().await?;
        let names = self.list_group_names(&masters).await?;

        let mut groups: Vec<MiddlewareGroupInfo> = Vec::new();
        let enriched = names.len() <= GROUP_ENRICH_LIMIT;
        for name in names {
            let mut info = MiddlewareGroupInfo {
                group: name.clone(),
                client_count: None,
                consume_type: None,
                message_model: None,
                tps: None,
                total_diff: None,
                version: None,
                update_time: None,
            };
            if enriched {
                if let Ok(connection) = self.consumer_connection(&masters[0], &name).await {
                    info.client_count = Some(connection.connection_set.len() as u32);
                    if !connection.connection_set.is_empty() {
                        let min_version = connection
                            .connection_set
                            .iter()
                            .map(|item| item.version)
                            .min()
                            .unwrap_or(0);
                        info.version = Some(min_version.to_string());
                    }
                    // 官方枚举转通用文本(见 consume_type_text 文档)
                    info.consume_type = connection.consume_type.as_deref().map(consume_type_text);
                    info.message_model = connection.message_model;
                }
                if let Ok(stats) = self.consume_stats(&masters[0], &name).await {
                    info.tps = Some(stats.consume_tps);
                    info.total_diff = Some(
                        stats
                            .offset_entries()
                            .iter()
                            .map(|(_, offset)| {
                                (offset.broker_offset - offset.consumer_offset).max(0)
                            })
                            .sum(),
                    );
                }
            }
            groups.push(info);
        }
        groups.sort_by(|a, b| a.group.cmp(&b.group));
        Ok(groups)
    }

    /// 订阅组客户端:各主节点在线连接合并
    async fn group_clients(
        &self,
        group: &str,
    ) -> Result<Vec<MiddlewareClientInfo>, MiddlewareError> {
        let masters = self.master_addrs().await?;
        let mut clients: Vec<MiddlewareClientInfo> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for addr in &masters {
            let connection = match self.consumer_connection(addr, group).await {
                Ok(connection) => connection,
                Err(MiddlewareError::Connection(_)) => continue,
                Err(error) => return Err(error),
            };
            let subscriptions = connection
                .subscription_table
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            for item in connection.connection_set {
                if !seen.insert(item.client_id.clone()) {
                    continue;
                }
                clients.push(MiddlewareClientInfo {
                    client_id: item.client_id,
                    client_addr: Some(item.client_addr),
                    language: Some(item.language),
                    version: Some(item.version.to_string()),
                    subscriptions: subscriptions.clone(),
                });
            }
        }
        clients.sort_by(|a, b| a.client_id.cmp(&b.client_id));
        Ok(clients)
    }

    /// 订阅组消费详情:各队列进度与堆积
    async fn group_detail(&self, group: &str) -> Result<GroupConsumeDetail, MiddlewareError> {
        let masters = self.master_addrs().await?;
        let mut queues: Vec<GroupQueueStat> = Vec::new();
        for addr in &masters {
            if let Ok(stats) = self.consume_stats(addr, group).await {
                for (queue, offset) in stats.offset_entries() {
                    queues.push(GroupQueueStat {
                        topic: queue.topic,
                        broker: queue.broker_name,
                        queue_id: queue.queue_id,
                        broker_offset: offset.broker_offset,
                        consumer_offset: offset.consumer_offset,
                        diff: (offset.broker_offset - offset.consumer_offset).max(0),
                    });
                }
            }
        }
        queues.sort_by(|a, b| {
            (&a.topic, &a.broker, a.queue_id).cmp(&(&b.topic, &b.broker, b.queue_id))
        });
        Ok(GroupConsumeDetail {
            group: group.to_string(),
            queues,
        })
    }

    /// 消息查询:ByKey 走索引;ById 解析偏移走 VIEW;时间窗口走消费队列遍历
    async fn query_messages(&self, query: MessageQuery) -> Result<MessagePage, MiddlewareError> {
        match query {
            MessageQuery::ByKey { topic, key } => {
                let route = self.topic_route(&topic).await?;
                self.query_by_key(&route, &topic, &key, None).await
            }
            MessageQuery::ById { topic, message_id } => self.query_by_id(&topic, &message_id).await,
            MessageQuery::ByTimeWindow {
                topic,
                begin_unix_ms,
                end_unix_ms,
                page,
                page_size,
            } => {
                let route = self.topic_route(&topic).await?;
                self.query_by_time_window(
                    &route,
                    &topic,
                    begin_unix_ms,
                    end_unix_ms,
                    page,
                    page_size,
                )
                .await
            }
        }
    }

    /// 消息详情:按消息 ID 拉取完整消息
    async fn message_detail(
        &self,
        topic: &str,
        message_id: &str,
    ) -> Result<MiddlewareMessage, MiddlewareError> {
        let mut page = self.query_by_id(topic, message_id).await?;
        page.messages
            .pop()
            .ok_or_else(|| MiddlewareError::Protocol(format!("消息不存在: {message_id}")))
    }
}

impl RocketmqConnection {
    /// NameServer 全量 Topic 名单
    async fn list_topic_names(&self) -> Result<Vec<String>, MiddlewareError> {
        let request = RemotingCommand::create_request(
            RequestCode::GET_ALL_TOPIC_LIST_FROM_NAMESERVER,
            HashMap::new(),
        );
        let response = self.client.invoke_namesrv(request).await?;
        let list: TopicListBody = decode_body(&response)?;
        Ok(list.topic_list)
    }

    /// Broker 上全部订阅组名(并集)
    async fn list_group_names(&self, masters: &[String]) -> Result<Vec<String>, MiddlewareError> {
        let mut names = std::collections::BTreeSet::new();
        for addr in masters {
            let request = RemotingCommand::create_request(
                RequestCode::GET_ALL_SUBSCRIPTIONGROUP_CONFIG,
                HashMap::new(),
            );
            if let Ok(response) = self.client.invoke_broker(addr, request).await {
                if let Ok(wrapper) = decode_body::<SubscriptionGroupWrapperBody>(&response) {
                    names.extend(wrapper.subscription_group_table.into_keys());
                }
            }
        }
        Ok(names.into_iter().collect())
    }

    /// 订阅组在线连接(离线/不存在归一化为空连接集)
    async fn consumer_connection(
        &self,
        addr: &str,
        group: &str,
    ) -> Result<ConsumerConnectionBody, MiddlewareError> {
        let mut ext = HashMap::new();
        ext.insert("consumerGroup".to_string(), group.to_string());
        let request =
            RemotingCommand::create_request(RequestCode::GET_CONSUMER_CONNECTION_LIST, ext);
        let response = self.client.invoke_broker_raw(addr, request).await?;
        if response.code == ResponseCode::CONSUMER_NOT_ONLINE
            || response.code == ResponseCode::SUBSCRIPTION_GROUP_NOT_EXIST
        {
            return Ok(ConsumerConnectionBody::default());
        }
        if response.code == ResponseCode::NO_PERMISSION {
            return Err(MiddlewareError::Auth(format!(
                "查询订阅组连接被拒绝: {}",
                response.remark.clone().unwrap_or_default()
            )));
        }
        if response.code != ResponseCode::SUCCESS {
            return Err(MiddlewareError::Protocol(format!(
                "查询订阅组连接失败 code={}: {}",
                response.code,
                response.remark.clone().unwrap_or_default()
            )));
        }
        decode_body(&response)
    }

    /// 订阅组消费进度
    async fn consume_stats(
        &self,
        addr: &str,
        group: &str,
    ) -> Result<ConsumeStatsBody, MiddlewareError> {
        let mut ext = HashMap::new();
        ext.insert("consumerGroup".to_string(), group.to_string());
        let request = RemotingCommand::create_request(RequestCode::GET_CONSUME_STATS, ext);
        let response = self.client.invoke_broker_raw(addr, request).await?;
        if response.code == ResponseCode::CONSUMER_NOT_ONLINE {
            return Ok(ConsumeStatsBody::default());
        }
        if response.code == ResponseCode::NO_PERMISSION {
            return Err(MiddlewareError::Auth(format!(
                "查询消费进度被拒绝: {}",
                response.remark.clone().unwrap_or_default()
            )));
        }
        if response.code != ResponseCode::SUCCESS {
            return Err(MiddlewareError::Protocol(format!(
                "查询消费进度失败 code={}: {}",
                response.code,
                response.remark.clone().unwrap_or_default()
            )));
        }
        decode_body(&response)
    }

    /// 按 Key 查询(QUERY_MESSAGE,索引路径)
    async fn query_by_key(
        &self,
        route: &TopicRouteData,
        topic: &str,
        key: &str,
        time_range: Option<(i64, i64)>,
    ) -> Result<MessagePage, MiddlewareError> {
        let addr = route
            .broker_datas
            .iter()
            .find_map(|broker| broker.select_broker_addr().map(str::to_string))
            .ok_or_else(|| {
                MiddlewareError::Protocol(format!("Topic {topic} 路由缺少 Broker 地址"))
            })?;
        let (begin, end) = time_range.unwrap_or((0, i64::MAX));
        let mut ext = HashMap::new();
        ext.insert("topic".to_string(), topic.to_string());
        ext.insert("key".to_string(), key.to_string());
        ext.insert("maxNum".to_string(), "64".to_string());
        ext.insert("beginTimestamp".to_string(), begin.to_string());
        ext.insert(
            "endTimestamp".to_string(),
            end.min(i64::from(i32::MAX)).to_string(),
        );
        let request = RemotingCommand::create_request(RequestCode::QUERY_MESSAGE, ext);

        let response = self.client.invoke_broker_raw(&addr, request).await?;
        if response.code == ResponseCode::QUERY_NOT_FOUND {
            return Ok(MessagePage::default());
        }
        if response.code != ResponseCode::SUCCESS {
            return Err(MiddlewareError::Protocol(format!(
                "消息查询失败 code={}: {}",
                response.code,
                response.remark.clone().unwrap_or_default()
            )));
        }
        let result: QueryResultBody = decode_body(&response)?;
        let total = result.message_list.len() as u64;
        let messages: Vec<MiddlewareMessage> = result
            .message_list
            .iter()
            .map(Self::message_ext_to_model)
            .collect();
        Ok(MessagePage {
            messages,
            total,
            has_more: false,
        })
    }

    /// 按消息 ID 查询:存储 ID 解析偏移 → VIEW_MESSAGE_BY_ID;
    /// 无法解析(客户端 UNIQ_KEY)时退化为 Key 查询
    async fn query_by_id(
        &self,
        topic: &str,
        message_id: &str,
    ) -> Result<MessagePage, MiddlewareError> {
        let route = self.topic_route(topic).await?;
        match parse_msg_id(message_id) {
            Ok((host, offset)) => {
                // 存储地址在路由中的 Broker 上按偏移拉取
                let addr = Self::resolve_broker_addr(&route, &host)?;
                let stored = self.view_message_by_offset(&addr, topic, offset).await?;
                Ok(MessagePage {
                    messages: vec![Self::stored_to_model(&stored)],
                    total: 1,
                    has_more: false,
                })
            }
            Err(_) => {
                // 客户端唯一键:按 UNIQ_KEY 索引查询(官方 queryMessageByUniqKey 路径)
                let mut ext = HashMap::new();
                ext.insert("topic".to_string(), topic.to_string());
                ext.insert("key".to_string(), message_id.to_string());
                ext.insert("maxNum".to_string(), "32".to_string());
                ext.insert("beginTimestamp".to_string(), "0".to_string());
                ext.insert("endTimestamp".to_string(), i64::from(i32::MAX).to_string());
                let request = RemotingCommand::create_request(RequestCode::QUERY_MESSAGE, ext);
                let addr = Self::route_master_addr(&route)?;
                let response = self.client.invoke_broker_raw(&addr, request).await?;
                if response.code == ResponseCode::QUERY_NOT_FOUND {
                    return Ok(MessagePage::default());
                }
                if response.code != ResponseCode::SUCCESS {
                    return Err(MiddlewareError::Protocol(format!(
                        "消息查询失败 code={}: {}",
                        response.code,
                        response.remark.clone().unwrap_or_default()
                    )));
                }
                let result: QueryResultBody = decode_body(&response)?;
                let messages: Vec<MiddlewareMessage> = result
                    .message_list
                    .iter()
                    .map(Self::message_ext_to_model)
                    .collect();
                Ok(MessagePage {
                    total: messages.len() as u64,
                    messages,
                    has_more: false,
                })
            }
        }
    }

    /// 路由中定位存储地址所属 Broker(存储地址匹配失败回退首个主节点)
    fn resolve_broker_addr(
        route: &TopicRouteData,
        store_host: &str,
    ) -> Result<String, MiddlewareError> {
        let store_addr = store_host.to_string();
        for broker in &route.broker_datas {
            for addr in broker.broker_addrs.values() {
                if addr.eq_ignore_ascii_case(&store_addr) {
                    return Ok(addr.clone());
                }
            }
        }
        // 地址形态不一致(端口 VIP 偏移等)时按 IP 前缀匹配
        let store_ip = store_host.split(':').next().unwrap_or_default();
        for broker in &route.broker_datas {
            for addr in broker.broker_addrs.values() {
                if addr.split(':').next().unwrap_or_default() == store_ip {
                    return Ok(addr.clone());
                }
            }
        }
        Self::route_master_addr(route)
    }

    /// 路由首个可用主节点地址
    fn route_master_addr(route: &TopicRouteData) -> Result<String, MiddlewareError> {
        route
            .broker_datas
            .iter()
            .find_map(|broker| broker.select_broker_addr().map(str::to_string))
            .ok_or_else(|| MiddlewareError::Protocol("路由缺少 Broker 地址".into()))
    }

    /// 时间窗口查询:SEARCH_OFFSET_BY_TIMESTAMP + QUERY_CONSUME_QUEUE 遍历,
    /// 再按偏移逐条 VIEW_MESSAGE_BY_ID 补全消息体
    async fn query_by_time_window(
        &self,
        route: &TopicRouteData,
        topic: &str,
        begin_unix_ms: i64,
        end_unix_ms: i64,
        page: u32,
        page_size: u32,
    ) -> Result<MessagePage, MiddlewareError> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, MESSAGE_PAGE_MAX);
        let skip = ((page - 1) as usize) * page_size as usize;

        // brokerName → 主节点地址
        let mut broker_addrs: HashMap<&str, String> = HashMap::new();
        for broker in &route.broker_datas {
            if let Some(addr) = broker.select_broker_addr() {
                broker_addrs.insert(broker.broker_name.as_str(), addr.to_string());
            }
        }

        // 收集各队列起始下标(按 begin 时间戳搜索)
        struct QueueScan {
            broker_name: String,
            queue_id: i32,
            index: i64,
            max_index: i64,
        }
        let mut scans: Vec<QueueScan> = Vec::new();
        for queue in &route.queue_datas {
            let Some(addr) = broker_addrs.get(queue.broker_name.as_str()) else {
                continue;
            };
            for queue_id in 0..queue.read_queue_nums.max(0) {
                let mut ext = HashMap::new();
                ext.insert("topic".to_string(), topic.to_string());
                ext.insert("queueId".to_string(), queue_id.to_string());
                ext.insert("timestamp".to_string(), begin_unix_ms.to_string());
                let request =
                    RemotingCommand::create_request(RequestCode::SEARCH_OFFSET_BY_TIMESTAMP, ext);
                let index = match self.client.invoke_broker_raw(addr, request).await {
                    Ok(response) => response
                        .ext_fields
                        .get("offset")
                        .and_then(|text| text.parse::<i64>().ok())
                        .unwrap_or(0),
                    Err(_) => 0,
                };
                let mut ext = HashMap::new();
                ext.insert("topic".to_string(), topic.to_string());
                ext.insert("queueId".to_string(), queue_id.to_string());
                ext.insert("index".to_string(), index.to_string());
                ext.insert("count".to_string(), "0".to_string());
                ext.insert("consumerGroup".to_string(), String::new());
                let request =
                    RemotingCommand::create_request(RequestCode::QUERY_CONSUME_QUEUE, ext);
                let max_index = match self.client.invoke_broker_raw(addr, request).await {
                    Ok(response) => decode_body::<QueryConsumeQueueResponseBodyBody>(&response)
                        .map(|body| body.max_queue_index)
                        .unwrap_or(index - 1),
                    Err(_) => index - 1,
                };
                if max_index >= index {
                    scans.push(QueueScan {
                        broker_name: queue.broker_name.clone(),
                        queue_id,
                        index,
                        max_index,
                    });
                }
            }
        }
        if scans.is_empty() {
            return Ok(MessagePage::default());
        }

        // 逐队列按页大小分批遍历消费队列,全局跳过前 skip 条
        let mut entries: Vec<(String, i64)> = Vec::new(); // (brokerName, physicOffset)
        let mut has_more = false;
        let batch = page_size as usize;
        let mut cursor: HashMap<(String, i32), i64> = scans
            .iter()
            .map(|scan| ((scan.broker_name.clone(), scan.queue_id), scan.index))
            .collect();
        loop {
            let mut progressed = false;
            for scan in &scans {
                if entries.len() >= skip + batch {
                    break;
                }
                let key = (scan.broker_name.clone(), scan.queue_id);
                let current = cursor.get(&key).copied().unwrap_or(0);
                if current > scan.max_index {
                    continue;
                }
                let Some(addr) = broker_addrs.get(scan.broker_name.as_str()) else {
                    continue;
                };
                let mut ext = HashMap::new();
                ext.insert("topic".to_string(), topic.to_string());
                ext.insert("queueId".to_string(), scan.queue_id.to_string());
                ext.insert("index".to_string(), current.to_string());
                ext.insert("count".to_string(), batch.to_string());
                ext.insert("consumerGroup".to_string(), String::new());
                let request =
                    RemotingCommand::create_request(RequestCode::QUERY_CONSUME_QUEUE, ext);
                let Ok(response) = self.client.invoke_broker_raw(addr, request).await else {
                    continue;
                };
                let Ok(body) = decode_body::<QueryConsumeQueueResponseBodyBody>(&response) else {
                    continue;
                };
                let count = body.queue_data.len() as i64;
                for entry in &body.queue_data {
                    entries.push((scan.broker_name.clone(), entry.physic_offset));
                }
                if count > 0 {
                    progressed = true;
                    cursor.insert(key, current + count);
                } else {
                    cursor.insert(key, scan.max_index + 1);
                }
                if body.max_queue_index >= current + count {
                    has_more = true;
                }
            }
            if !progressed {
                break;
            }
            if entries.len() >= skip + batch {
                break;
            }
        }
        let selected: Vec<(String, i64)> = entries.into_iter().skip(skip).take(batch).collect();
        has_more = has_more || !selected.is_empty();

        // 逐条 VIEW_MESSAGE_BY_ID 补全(过滤晚于 end 时间戳的消息)
        let mut messages = Vec::with_capacity(selected.len());
        for (broker_name, physic_offset) in selected {
            let Some(addr) = broker_addrs.get(broker_name.as_str()) else {
                continue;
            };
            if let Ok(stored) = self
                .view_message_by_offset(addr, topic, physic_offset)
                .await
            {
                if stored.store_timestamp > end_unix_ms {
                    continue;
                }
                messages.push(Self::stored_to_model(&stored));
            }
        }
        Ok(MessagePage {
            total: messages.len() as u64,
            messages,
            has_more,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_topic_detection() {
        assert!(is_system_topic("rmq_sys_wheel_timer"));
        assert!(is_system_topic("TBW102"));
        assert!(is_system_topic("SELF_TEST_TOPIC"));
        assert!(is_system_topic("SCHEDULE_TOPIC_1931"));
        assert!(!is_system_topic("order-topic"));
        assert!(!is_system_topic("order-topic-schedule"));
    }

    #[test]
    fn topic_type_inference() {
        assert_eq!(
            RocketmqConnection::infer_topic_type("%RETRY%order-group", None),
            Some("RETRY".into())
        );
        assert_eq!(
            RocketmqConnection::infer_topic_type("%DLQ%order-group", None),
            Some("DLQ".into())
        );
        assert_eq!(
            RocketmqConnection::infer_topic_type("rmq_sys_wheel_timer", None),
            Some("SYSTEM".into())
        );
        assert_eq!(
            RocketmqConnection::infer_topic_type("order-topic", None),
            Some("UNSPECIFIED".into())
        );
        // 5.x 属性表优先
        let config = TopicConfigBody {
            attributes: Some(HashMap::from([("+type".to_string(), "FIFO".to_string())])),
            ..Default::default()
        };
        assert_eq!(
            RocketmqConnection::infer_topic_type("order-topic", Some(&config)),
            Some("FIFO".into())
        );
    }

    #[test]
    fn parse_perm_text() {
        assert_eq!(parse_perm("6"), 6);
        assert_eq!(parse_perm("rw"), 6);
        assert_eq!(parse_perm("4"), 4);
        assert_eq!(parse_perm("r"), 4);
        assert_eq!(parse_perm("2"), 2);
        assert_eq!(parse_perm(""), 6);
        assert_eq!(parse_perm("bad"), 6);
    }

    #[test]
    fn topic_type_attributes_mapping() {
        assert_eq!(
            topic_type_attributes(Some("FIFO")),
            Some("\"+type\"=\"FIFO\"".to_string())
        );
        assert_eq!(
            topic_type_attributes(Some("DELAY")),
            Some("\"+type\"=\"DELAY\"".to_string())
        );
        assert_eq!(topic_type_attributes(Some("NORMAL")), None);
        assert_eq!(topic_type_attributes(None), None);
    }

    #[test]
    fn tps_text_parse() {
        assert_eq!(parse_tps(Some(&" 0.0 0.0 0.0".to_string())), 0.0);
        assert!((parse_tps(Some(&" 12.5 1.0 2.0".to_string())) - 12.5).abs() < f64::EPSILON);
        assert_eq!(parse_tps(None), 0.0);
        assert_eq!(parse_tps(Some(&"bad".to_string())), 0.0);
    }

    /// 消费类型映射回归测试(t6 审查 F1 修复)。
    ///
    /// 官方依据:release-4.9.4 `common/protocol/heartbeat/ConsumeType.java`:
    /// `CONSUME_ACTIVELY("PULL")`(主动拉取)、`CONSUME_PASSIVELY("PUSH")`(被动推送)。
    #[test]
    fn consume_type_mapping_matches_official_enum() {
        assert_eq!(consume_type_text("CONSUME_ACTIVELY"), "PULL");
        assert_eq!(consume_type_text("CONSUME_PASSIVELY"), "PUSH");
        // 未知值原样透传
        assert_eq!(consume_type_text("SOMETHING_ELSE"), "SOMETHING_ELSE");
        assert_eq!(consume_type_text(""), "");
    }

    #[test]
    fn unix_ms_format() {
        let text = format_unix_ms(1735710000000);
        assert!(!text.is_empty());
        assert!(text.contains("2025"));
        assert_eq!(format_unix_ms(0), "");
        assert_eq!(format_unix_ms(-1), "");
    }

    #[test]
    fn born_host_addr_extraction() {
        assert_eq!(
            json_value_to_addr(&Some(serde_json::Value::String("/10.0.0.9:52300".into()))),
            "10.0.0.9:52300"
        );
        assert_eq!(
            json_value_to_addr(&Some(
                serde_json::json!({"address": "10.0.0.9", "port": 52300})
            )),
            "10.0.0.9:52300"
        );
        assert_eq!(json_value_to_addr(&None), "");
    }

    #[test]
    fn message_models_map_fields() {
        let message = MessageExtBody {
            topic: "order-topic".into(),
            queue_id: 3,
            born_timestamp: 1735710000000,
            store_timestamp: 1735710000010,
            reconsume_times: 2,
            msg_id: Some("0A0A0A0200002A9F0000000000000B60".into()),
            properties: Some(HashMap::from([
                ("KEYS".to_string(), "order-1".to_string()),
                ("TAGS".to_string(), "create".to_string()),
            ])),
            body: Some(base64_encode(b"{\"orderId\":1}")),
            born_host: Some(serde_json::Value::String("/10.0.0.9:52300".into())),
            store_host: Some(serde_json::Value::String("/10.0.0.2:10911".into())),
            ..Default::default()
        };
        let model = RocketmqConnection::message_ext_to_model(&message);
        assert_eq!(model.message_id, "0A0A0A0200002A9F0000000000000B60");
        assert_eq!(model.tag.as_deref(), Some("create"));
        assert_eq!(model.key.as_deref(), Some("order-1"));
        assert_eq!(model.body_text.as_deref(), Some("{\"orderId\":1}"));
        assert_eq!(model.born_host.as_deref(), Some("10.0.0.9:52300"));
        assert_eq!(model.retry_times, Some(2));

        // StoredMessage 映射
        let stored = StoredMessage {
            topic: "order-topic".into(),
            queue_id: 0,
            physic_offset: 2912,
            born_timestamp: 1735710000000,
            store_timestamp: 1735710000010,
            reconsume_times: 0,
            store_host: "10.0.0.2:10911".into(),
            born_host: "10.0.0.9:52300".into(),
            body: b"hello".to_vec(),
            properties: HashMap::from([("TAGS".to_string(), "t".to_string())]),
            ..Default::default()
        };
        let model = RocketmqConnection::stored_to_model(&stored);
        assert_eq!(model.message_id, "0A00000200002A9F0000000000000B60");
        assert_eq!(model.body_text.as_deref(), Some("hello"));
        assert_eq!(model.tag.as_deref(), Some("t"));
    }

    fn base64_encode(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }
}
