//! Remoting body DTO:字段名与官方 fastjson 序列化逐一对齐。
//!
//! 对照源码(4.9.x 位于 `common`/`remoting` 模块,5.x 统一迁移至 `remoting` 模块,
//! 字段名跨版本一致):
//! - 路由: `route/TopicRouteData.java` / `QueueData.java` / `BrokerData.java`
//! - 集群: `body/ClusterInfo.java`
//! - Topic: `body/TopicList.java` / `TopicConfigSerializeWrapper.java` / `common/TopicConfig.java`
//! - 统计: `admin/TopicStatsTable.java` / `TopicOffset.java` / `ConsumeStats.java` / `OffsetWrapper.java`
//! - 订阅组: `body/SubscriptionGroupWrapper.java` / `subscription/SubscriptionGroupConfig.java`
//! - 客户端: `body/ConsumerConnection.java` / `Connection.java`
//! - 运行时: `body/KVTable.java`
//! - 消息: `client/QueryResult.java` / `message/MessageExt.java`
//! - 消费队列: `body/QueryConsumeQueueResponseBody.java` / `ConsumeQueueData.java`
//!
//! 全部容器级 `#[serde(default)]` 前向兼容:服务端新增字段或省略字段均可解析。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 队列维度的路由信息(官方 `route/QueueData`)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QueueData {
    /// 所属 Broker 名称
    pub broker_name: String,
    /// 读队列数量
    pub read_queue_nums: i32,
    /// 写队列数量
    pub write_queue_nums: i32,
    /// 读写权限位:6=读写 4=只读 2=只写
    pub perm: i32,
    /// Topic 系统标志(事务/顺序等)
    pub topic_sys_flag: i32,
}

impl QueueData {
    /// perm 位转标准文本("6"/"4"/"2")
    pub fn perm_text(&self) -> String {
        self.perm.to_string()
    }
}

/// Broker 主备组信息(官方 `route/BrokerData`)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrokerData {
    /// 所属集群名
    pub cluster: String,
    /// Broker 名称(主备共享)
    pub broker_name: String,
    /// brokerId → 地址;brokerId=0 为主节点(MASTER_ID)
    pub broker_addrs: HashMap<i64, String>,
    /// 是否启用从节点代主
    #[serde(default)]
    pub enable_acting_master: bool,
}

impl BrokerData {
    /// 主节点地址(brokerId=0),无主时取任意节点(对齐官方 `selectBrokerAddr` 语义)
    pub fn select_broker_addr(&self) -> Option<&str> {
        if let Some(addr) = self.broker_addrs.get(&0) {
            return Some(addr);
        }
        self.broker_addrs.values().next().map(String::as_str)
    }
}

/// Topic 路由(官方 `route/TopicRouteData`,GET_ROUTEINFO_BY_TOPIC 响应 body)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TopicRouteData {
    /// 顺序 Topic 配置串
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_topic_conf: Option<String>,
    /// 队列维度路由
    pub queue_datas: Vec<QueueData>,
    /// Broker 维度路由
    pub broker_datas: Vec<BrokerData>,
    /// broker 地址 → filter server 列表(旧版过滤服务器,可忽略)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub filter_server_table: HashMap<String, Vec<String>>,
}

/// 集群信息(官方 `body/ClusterInfo`,GET_BROKER_CLUSTER_INFO 响应 body)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClusterInfoBody {
    /// brokerName → Broker 主备组
    pub broker_addr_table: HashMap<String, BrokerData>,
    /// 集群名 → brokerName 集合
    pub cluster_addr_table: HashMap<String, Vec<String>>,
}

/// Topic 列表(官方 `body/TopicList`,GET_ALL_TOPIC_LIST_FROM_NAMESERVER 响应 body)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TopicListBody {
    /// Topic 名称集合
    pub topic_list: Vec<String>,
    /// 冗余字段(旧版专用)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broker_addr: Option<String>,
}

/// Topic 配置(官方 `common/TopicConfig`)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TopicConfigBody {
    /// Topic 名称
    pub topic_name: String,
    /// 读队列数量(默认 8)
    pub read_queue_nums: i32,
    /// 写队列数量(默认 8)
    pub write_queue_nums: i32,
    /// 读写权限位:6=读写 4=只读 2=只写
    pub perm: i32,
    /// 过滤类型(SINGLE_TAG/MULTI_TAG)
    pub topic_filter_type: String,
    /// Topic 系统标志
    pub topic_sys_flag: i32,
    /// 是否顺序 Topic
    pub order: bool,
    /// 5.x 属性表(如 `+type` → NORMAL/FIFO/DELAY/TRANSACTION);4.x 无此字段
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<HashMap<String, String>>,
}

impl TopicConfigBody {
    /// perm 位转标准文本("6"/"4"/"2")
    pub fn perm_text(&self) -> String {
        self.perm.to_string()
    }
}

/// Topic 配置集合(官方 `body/TopicConfigSerializeWrapper`,GET_ALL_TOPIC_CONFIG 响应 body)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TopicConfigSerializeWrapperBody {
    /// topic → 配置
    pub topic_config_table: HashMap<String, TopicConfigBody>,
}

/// 消息队列坐标(官方 `message/MessageQueue`:topic + brokerName + queueId)
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MessageQueueBody {
    /// Topic 名称
    pub topic: String,
    /// 所在 Broker 名称
    pub broker_name: String,
    /// 队列 ID
    pub queue_id: i32,
}

/// 单队列偏移统计(官方 `admin/TopicOffset`)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TopicOffsetBody {
    /// 最小偏移
    pub min_offset: i64,
    /// 最大偏移
    pub max_offset: i64,
    /// 最后写入时间(Unix 毫秒)
    pub last_update_timestamp: i64,
}

/// Topic 队列统计表(官方 `admin/TopicStatsTable`,GET_TOPIC_STATS_INFO 响应 body)。
///
/// 官方 fastjson 将 `HashMap<MessageQueue, TopicOffset>` 的 POJO 键
/// 序列化为 JSON 字符串,故此处键为 String,经 [`Self::offset_entries`] 解析。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TopicStatsTableBody {
    /// 队列坐标(JSON 字符串)→ 偏移统计
    pub offset_table: HashMap<String, TopicOffsetBody>,
}

impl TopicStatsTableBody {
    /// 解析字符串键,产出(队列坐标, 偏移统计)列表;无法解析的键跳过
    pub fn offset_entries(&self) -> Vec<(MessageQueueBody, &TopicOffsetBody)> {
        self.offset_table
            .iter()
            .filter_map(|(key, offset)| {
                serde_json::from_str::<MessageQueueBody>(key)
                    .ok()
                    .map(|queue| (queue, offset))
            })
            .collect()
    }
}

/// 消费进度包装(官方 `admin/OffsetWrapper`)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OffsetWrapperBody {
    /// Broker 端最大偏移
    pub broker_offset: i64,
    /// 消费者已提交偏移
    pub consumer_offset: i64,
    /// 最后提交时间(Unix 毫秒)
    pub last_timestamp: i64,
}

/// 订阅组消费统计(官方 `admin/ConsumeStats`,GET_CONSUME_STATS 响应 body)。
///
/// 键为 fastjson 序列化的 `MessageQueue` JSON 字符串,经 [`Self::offset_entries`] 解析。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConsumeStatsBody {
    /// 消费 TPS
    pub consume_tps: f64,
    /// 队列坐标(JSON 字符串)→ 消费进度
    pub offset_table: HashMap<String, OffsetWrapperBody>,
}

impl ConsumeStatsBody {
    /// 解析字符串键,产出(队列坐标, 消费进度)列表;无法解析的键跳过
    pub fn offset_entries(&self) -> Vec<(MessageQueueBody, &OffsetWrapperBody)> {
        self.offset_table
            .iter()
            .filter_map(|(key, offset)| {
                serde_json::from_str::<MessageQueueBody>(key)
                    .ok()
                    .map(|queue| (queue, offset))
            })
            .collect()
    }
}

/// 订阅组配置(官方 `subscription/SubscriptionGroupConfig`)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SubscriptionGroupConfigBody {
    /// 订阅组名
    pub group_name: String,
    /// 是否允许消费
    pub consume_enable: bool,
    /// 是否允许从最小位点消费
    pub consume_from_min_enable: bool,
    /// 是否允许广播消费
    pub consume_broadcast_enable: bool,
    /// 是否顺序消费
    pub consume_message_orderly: bool,
    /// 重试队列数量
    pub retry_queue_nums: i32,
    /// 最大重试次数
    pub retry_max_times: i32,
    /// 消费落到的 brokerId
    pub broker_id: i64,
    /// 消费堆积时转移到的 brokerId
    pub which_broker_when_consume_slowly: i64,
    /// 客户端变更通知开关
    pub notify_consumer_ids_changed_enable: bool,
    /// 订阅组系统标志
    pub group_sys_flag: i32,
    /// 消费超时(分钟)
    pub consume_timeout_minute: i32,
}

/// 订阅组配置集合(官方 `body/SubscriptionGroupWrapper`,GET_ALL_SUBSCRIPTIONGROUP_CONFIG 响应 body)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SubscriptionGroupWrapperBody {
    /// 订阅组名 → 配置
    pub subscription_group_table: HashMap<String, SubscriptionGroupConfigBody>,
}

/// 客户端连接信息(官方 `body/Connection`)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConnectionBody {
    /// 客户端 ID(IP@PID)
    pub client_id: String,
    /// 客户端地址
    pub client_addr: String,
    /// 客户端语言(JAVA/CPP/GO/OTHER...)
    pub language: String,
    /// 客户端版本号(整型)
    pub version: i32,
}

/// 订阅组在线连接(官方 `body/ConsumerConnection`,GET_CONSUMER_CONNECTION_LIST 响应 body)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConsumerConnectionBody {
    /// 在线客户端集合
    pub connection_set: Vec<ConnectionBody>,
    /// 订阅的 Topic → 订阅数据(仅取键名)
    #[serde(default)]
    pub subscription_table: HashMap<String, serde_json::Value>,
    /// 消费类型(官方枚举:CONSUME_ACTIVELY=主动消费 PULL、CONSUME_PASSIVELY=被动消费 PUSH)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consume_type: Option<String>,
    /// 消息模型(CLUSTERING/BROADCASTING)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_model: Option<String>,
    /// 初始消费位点策略
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consume_from_where: Option<String>,
}

/// KV 表(官方 `body/KVTable`,GET_BROKER_RUNTIME_INFO 响应 body)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KvTableBody {
    /// 键值对(如 putTps/getTransferedTps/msgPutTotalTodayNow/brokerVersionDesc)
    pub table: HashMap<String, String>,
}

/// 消息扩展(官方 `message/MessageExt`,fastjson 序列化字段)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MessageExtBody {
    /// 所属 Topic
    pub topic: String,
    /// 消息标志
    pub flag: i32,
    /// 消息属性(Properties 字符串解析结果或原生 map)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, String>>,
    /// 消息体(fastjson 将 byte[] 序列化为 Base64 文本)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// 事务 ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    /// 所在 Broker 名称(5.x)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broker_name: Option<String>,
    /// 队列 ID
    pub queue_id: i32,
    /// 存储占用字节数
    pub store_size: i32,
    /// 队列偏移
    pub queue_offset: i64,
    /// 系统标志
    pub sys_flag: i32,
    /// 生成时间(Unix 毫秒)
    pub born_timestamp: i64,
    /// 生成主机(fastjson 序列化为字符串或对象,按原样保留)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub born_host: Option<serde_json::Value>,
    /// 存储时间(Unix 毫秒)
    pub store_timestamp: i64,
    /// 存储 Broker(fastjson 序列化为字符串或对象)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_host: Option<serde_json::Value>,
    /// 消息 ID(存储侧,IP+偏移的十六进制)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,
    /// commitlog 物理偏移
    pub commit_log_offset: i64,
    /// body CRC
    pub body_crc: i32,
    /// 重试次数
    pub reconsume_times: i32,
    /// 事务半消息偏移
    pub prepared_transaction_offset: i64,
}

/// Key 查询结果(官方 `client/QueryResult`,QUERY_MESSAGE 响应 body)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QueryResultBody {
    /// 命中消息列表
    pub message_list: Vec<MessageExtBody>,
    /// 索引文件最后更新时间(Unix 毫秒)
    pub index_last_update_timestamp: i64,
}

/// 消费队列条目(官方 `body/ConsumeQueueData`)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConsumeQueueDataBody {
    /// commitlog 物理偏移
    pub physic_offset: i64,
    /// 消息占用字节数
    pub physic_size: i32,
    /// Tag 哈希
    pub tags_code: i64,
}

/// 消费队列遍历结果(官方 `body/QueryConsumeQueueResponseBody`,QUERY_CONSUME_QUEUE 响应 body)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QueryConsumeQueueResponseBodyBody {
    /// 从 index 开始的条目
    pub queue_data: Vec<ConsumeQueueDataBody>,
    /// 队列最大下标(=最大偏移-1)
    pub max_queue_index: i64,
    /// 队列最小下标(=最小偏移)
    pub min_queue_index: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 路由/集群样本(字段名与 fastjson 序列化一致)
    #[test]
    fn route_and_cluster_samples() {
        let route: TopicRouteData = serde_json::from_str(
            r#"{
              "orderTopicConf": "4:8",
              "queueDatas": [
                {"brokerName":"broker-a","readQueueNums":8,"writeQueueNums":8,"perm":6,"topicSysFlag":0},
                {"brokerName":"broker-b","readQueueNums":4,"writeQueueNums":4,"perm":4,"topicSysFlag":0}
              ],
              "brokerDatas": [
                {"cluster":"DefaultCluster","brokerName":"broker-a",
                 "brokerAddrs":{"0":"10.0.0.2:10911","1":"10.0.0.3:10911"},
                 "enableActingMaster":false},
                {"cluster":"DefaultCluster","brokerName":"broker-b",
                 "brokerAddrs":{"0":"10.0.0.4:10911"}}
              ],
              "filterServerTable":{}
            }"#,
        )
        .expect("路由样本解析失败");
        assert_eq!(route.queue_datas.len(), 2);
        assert_eq!(route.queue_datas[0].read_queue_nums, 8);
        assert_eq!(route.queue_datas[0].perm_text(), "6");
        assert_eq!(route.broker_datas.len(), 2);
        assert_eq!(
            route.broker_datas[0].select_broker_addr(),
            Some("10.0.0.2:10911")
        );
        // 无主时回退任意节点
        assert_eq!(
            route.broker_datas[1].select_broker_addr(),
            Some("10.0.0.4:10911")
        );

        let cluster: ClusterInfoBody = serde_json::from_str(
            r#"{
              "brokerAddrTable": {
                "broker-a": {"cluster":"DefaultCluster","brokerName":"broker-a",
                             "brokerAddrs":{"0":"10.0.0.2:10911"}}
              },
              "clusterAddrTable": {"DefaultCluster":["broker-a"]}
            }"#,
        )
        .expect("集群样本解析失败");
        assert_eq!(cluster.cluster_addr_table["DefaultCluster"], ["broker-a"]);
        assert!(cluster.broker_addr_table.contains_key("broker-a"));
    }

    /// Topic 配置与列表样本
    #[test]
    fn topic_config_samples() {
        let wrapper: TopicConfigSerializeWrapperBody = serde_json::from_str(
            r#"{
              "topicConfigTable": {
                "order-topic": {"topicName":"order-topic","readQueueNums":8,"writeQueueNums":8,
                                "perm":6,"topicFilterType":"SINGLE_TAG","topicSysFlag":0,"order":false},
                "rmq_sys_wheel_timer": {"topicName":"rmq_sys_wheel_timer","readQueueNums":1,
                                "writeQueueNums":1,"perm":4,"topicFilterType":"SINGLE_TAG"}
              },
              "dataVersion": {}
            }"#,
        )
        .expect("Topic 配置样本解析失败");
        assert_eq!(wrapper.topic_config_table.len(), 2);
        assert_eq!(wrapper.topic_config_table["order-topic"].perm, 6);
        // 4.x 无 attributes 时默认 None
        assert_eq!(wrapper.topic_config_table["order-topic"].attributes, None);

        let list: TopicListBody = serde_json::from_str(
            r#"{"topicList":["order-topic","%RETRY%order-group","%DLQ%order-group"]}"#,
        )
        .expect("Topic 列表样本解析失败");
        assert_eq!(list.topic_list.len(), 3);
    }

    /// 统计类样本:Topic 偏移表 / 消费进度
    #[test]
    fn stats_samples() {
        // fastjson 将 MessageQueue 键序列化为 JSON 字符串
        let stats: TopicStatsTableBody = serde_json::from_str(
            r#"{
              "offsetTable": {
                "{\"brokerName\":\"broker-a\",\"queueId\":0,\"topic\":\"order-topic\"}":
                  {"minOffset":0,"maxOffset":1024,"lastUpdateTimestamp":1735710000000},
                "{\"brokerName\":\"broker-a\",\"queueId\":1,\"topic\":\"order-topic\"}":
                  {"minOffset":0,"maxOffset":512,"lastUpdateTimestamp":0}
              }
            }"#,
        )
        .expect("Topic 统计样本解析失败");
        assert_eq!(stats.offset_table.len(), 2);
        let entries = stats.offset_entries();
        assert_eq!(entries.len(), 2);
        let queue0 = entries
            .iter()
            .find(|(queue, _)| queue.queue_id == 0)
            .expect("queueId=0 条目");
        assert_eq!(queue0.1.max_offset, 1024);

        let consume: ConsumeStatsBody = serde_json::from_str(
            r#"{
              "consumeTps": 12.5,
              "offsetTable": {
                "{\"brokerName\":\"broker-a\",\"queueId\":0,\"topic\":\"order-topic\"}":
                  {"brokerOffset":1000,"consumerOffset":990,"lastTimestamp":1735710000000}
              }
            }"#,
        )
        .expect("消费统计样本解析失败");
        assert!((consume.consume_tps - 12.5).abs() < f64::EPSILON);
        let entries = consume.offset_entries();
        let (_, offset) = entries.first().unwrap();
        assert_eq!(offset.broker_offset - offset.consumer_offset, 10);
    }

    /// 订阅组/客户端连接样本
    #[test]
    fn subscription_and_connection_samples() {
        let wrapper: SubscriptionGroupWrapperBody = serde_json::from_str(
            r#"{
              "subscriptionGroupTable": {
                "order-group": {"groupName":"order-group","consumeEnable":true,
                                "retryQueueNums":1,"retryMaxTimes":16,
                                "whichBrokerWhenConsumeSlowly":1}
              },
              "dataVersion": {}
            }"#,
        )
        .expect("订阅组样本解析失败");
        assert!(wrapper.subscription_group_table["order-group"].consume_enable);

        let connection: ConsumerConnectionBody = serde_json::from_str(
            r#"{
              "connectionSet": [
                {"clientId":"10.0.0.9@2912","clientAddr":"10.0.0.9:28000",
                 "language":"JAVA","version":412}
              ],
              "subscriptionTable": {"order-topic":{}},
              "consumeType":"CONSUME_ACTIVELY",
              "messageModel":"CLUSTERING",
              "consumeFromWhere":"CONSUME_FROM_LAST_OFFSET"
            }"#,
        )
        .expect("连接样本解析失败");
        assert_eq!(connection.connection_set.len(), 1);
        assert_eq!(connection.connection_set[0].client_id, "10.0.0.9@2912");
        assert_eq!(connection.consume_type.as_deref(), Some("CONSUME_ACTIVELY"));
        assert_eq!(
            connection.subscription_table.keys().collect::<Vec<_>>(),
            ["order-topic"]
        );
    }

    /// 消息查询结果样本(Base64 body + 属性)
    #[test]
    fn message_query_result_sample() {
        let result: QueryResultBody = serde_json::from_str(
            r#"{
              "messageList": [
                {
                  "topic":"order-topic","flag":0,
                  "properties":{"KEYS":"order-1","TAGS":"create","UNIQ_KEY":"0A0A0A090000000000000B60FE9A6B00"},
                  "body":"eyJvcmRlcklkIjoxfQ==",
                  "queueId":3,"storeSize":241,"queueOffset":7,"sysFlag":0,
                  "bornTimestamp":1735710000000,"bornHost":"/10.0.0.9:52300",
                  "storeTimestamp":1735710000010,"storeHost":"/10.0.0.2:10911",
                  "msgId":"0A0A0A0200002A6F0000000000000B60",
                  "commitLogOffset":2912,"bodyCRC":123456789,"reconsumeTimes":0,
                  "preparedTransactionOffset":0
                }
              ],
              "indexLastUpdateTimestamp": 1735710001000
            }"#,
        )
        .expect("消息查询样本解析失败");
        let message = &result.message_list[0];
        assert_eq!(message.queue_id, 3);
        assert_eq!(message.commit_log_offset, 2912);
        assert_eq!(message.reconsume_times, 0);
        assert_eq!(message.properties.as_ref().unwrap()["TAGS"], "create");
        // body 为 Base64 文本(fastjson byte[] 序列化)
        assert_eq!(message.body.as_deref(), Some("eyJvcmRlcklkIjoxfQ=="));
        // bornHost 为字符串形态
        assert_eq!(message.born_host.as_ref().unwrap(), "/10.0.0.9:52300");
    }

    /// 消费队列遍历样本
    #[test]
    fn consume_queue_sample() {
        let body: QueryConsumeQueueResponseBodyBody = serde_json::from_str(
            r#"{
              "queueData": [
                {"physicOffset":2912,"physicSize":241,"tagsCode":787339571},
                {"physicOffset":3153,"physicSize":241,"tagsCode":787339571}
              ],
              "maxQueueIndex": 7,
              "minQueueIndex": 0
            }"#,
        )
        .expect("消费队列样本解析失败");
        assert_eq!(body.queue_data.len(), 2);
        assert_eq!(body.queue_data[0].physic_offset, 2912);
        assert_eq!(body.max_queue_index, 7);
    }

    /// KV 运行时信息样本
    #[test]
    fn kv_table_sample() {
        let kv: KvTableBody = serde_json::from_str(
            r#"{"table":{"putTps":" 0.0 0.0 0.0","getTransferedTps":" 1.2 1.2 1.2",
                     "msgPutTotalTodayNow":"1024","brokerVersionDesc":"V4_9_4"}}"#,
        )
        .expect("KV 样本解析失败");
        assert_eq!(kv.table["brokerVersionDesc"], "V4_9_4");
    }

    /// 缺失字段容忍(前向兼容)
    #[test]
    fn tolerate_missing_fields() {
        let route: TopicRouteData = serde_json::from_str("{}").unwrap();
        assert!(route.queue_datas.is_empty());
        let consume: ConsumerConnectionBody = serde_json::from_str("{}").unwrap();
        assert!(consume.connection_set.is_empty());
        let query: QueryResultBody = serde_json::from_str("{}").unwrap();
        assert!(query.message_list.is_empty());
    }
}
