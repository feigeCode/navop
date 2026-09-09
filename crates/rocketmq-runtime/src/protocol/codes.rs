//! RocketMQ Remoting 请求码/响应码常量。
//!
//! 数值与 apache/rocketmq `RequestCode.java` / `ResponseCode.java` 逐一对齐,
//! 仅收录管理面板所需条目(字段含义参考官方源码注释)。

/// Remoting 请求码(官方 `RequestCode.java` 枚举值的子集)
pub mod request_code {
    /// 发送消息(V1,长字段头)
    pub const SEND_MESSAGE: i32 = 10;
    /// 按消息 Key 查询消息(broker)
    pub const QUERY_MESSAGE: i32 = 12;
    /// 按物理偏移查看消息(broker)
    pub const VIEW_MESSAGE_BY_ID: i32 = 33;
    /// 创建/更新 Topic(broker,upsert 语义)
    pub const UPDATE_AND_CREATE_TOPIC: i32 = 17;
    /// 获取 broker 上全部 Topic 配置(broker)
    pub const GET_ALL_TOPIC_CONFIG: i32 = 21;
    /// 获取 broker 运行时信息(broker)
    pub const GET_BROKER_RUNTIME_INFO: i32 = 28;
    /// 按时间戳搜索队列偏移(broker)
    pub const SEARCH_OFFSET_BY_TIMESTAMP: i32 = 29;
    /// 创建/更新订阅组(broker)
    pub const UPDATE_AND_CREATE_SUBSCRIPTIONGROUP: i32 = 200;
    /// 获取 broker 上全部订阅组配置(broker)
    pub const GET_ALL_SUBSCRIPTIONGROUP_CONFIG: i32 = 201;
    /// 获取 Topic 各队列偏移统计(broker)
    pub const GET_TOPIC_STATS_INFO: i32 = 202;
    /// 获取订阅组在线客户端连接(broker)
    pub const GET_CONSUMER_CONNECTION_LIST: i32 = 203;
    /// 获取订阅组消费进度/堆积(broker)
    pub const GET_CONSUME_STATS: i32 = 208;
    /// 获取 NameServer 全量 Topic 列表(namesrv)
    pub const GET_ALL_TOPIC_LIST_FROM_NAMESERVER: i32 = 206;
    /// 删除 broker 上的 Topic(broker)
    pub const DELETE_TOPIC_IN_BROKER: i32 = 215;
    /// 删除 NameServer 上的 Topic 路由(namesrv)
    pub const DELETE_TOPIC_IN_NAMESRV: i32 = 216;
    /// 在 NameServer 注册静态 Topic 路由(namesrv)
    pub const REGISTER_TOPIC_IN_NAMESRV: i32 = 217;
    /// 遍历消费队列条目(broker,消息按时间窗口查询用)
    pub const QUERY_CONSUME_QUEUE: i32 = 321;
    /// 发送消息(V2,短字段头 a/b/c/...)
    pub const SEND_MESSAGE_V2: i32 = 310;
    /// 从 NameServer 按 Topic 获取路由(namesrv)
    pub const GET_ROUTEINFO_BY_TOPIC: i32 = 105;
    /// 从 NameServer 获取集群信息(namesrv)
    pub const GET_BROKER_CLUSTER_INFO: i32 = 106;
}

/// Remoting 响应码(官方 `ResponseCode.java` + `RemotingSysResponseCode.java` 的子集)
pub mod response_code {
    /// 成功
    pub const SUCCESS: i32 = 0;
    /// 系统错误
    pub const SYSTEM_ERROR: i32 = 1;
    /// 系统繁忙
    pub const SYSTEM_BUSY: i32 = 2;
    /// 请求码不支持(用于 V2 头降级 V1 判断)
    pub const REQUEST_CODE_NOT_SUPPORTED: i32 = 3;
    /// 从节点不可用(发送仍成功)
    pub const SLAVE_NOT_AVAILABLE: i32 = 11;
    /// 无权限(ACL 拒绝)
    pub const NO_PERMISSION: i32 = 16;
    /// Topic 不存在
    pub const TOPIC_NOT_EXIST: i32 = 17;
    /// 查询无结果
    pub const QUERY_NOT_FOUND: i32 = 22;
    /// 订阅组不存在
    pub const SUBSCRIPTION_GROUP_NOT_EXIST: i32 = 26;
    /// 消费者不在线(订阅组查询连接时表示无在线客户端)
    pub const CONSUMER_NOT_ONLINE: i32 = 206;
}

/// 请求码便捷引用(与 Java 端 `RequestCode.XXX` 同名)
pub use request_code as RequestCode;

/// 响应码便捷引用(与 Java 端 `ResponseCode.XXX` 同名)
pub use response_code as ResponseCode;

#[cfg(test)]
mod tests {
    use super::*;

    /// 关键请求码与官方源码数值对齐(RequestCode.java)
    #[test]
    fn request_codes_match_official_source() {
        assert_eq!(RequestCode::SEND_MESSAGE, 10);
        assert_eq!(RequestCode::SEND_MESSAGE_V2, 310);
        assert_eq!(RequestCode::QUERY_MESSAGE, 12);
        assert_eq!(RequestCode::VIEW_MESSAGE_BY_ID, 33);
        assert_eq!(RequestCode::GET_ROUTEINFO_BY_TOPIC, 105);
        assert_eq!(RequestCode::GET_BROKER_CLUSTER_INFO, 106);
        assert_eq!(RequestCode::UPDATE_AND_CREATE_TOPIC, 17);
        assert_eq!(RequestCode::GET_ALL_TOPIC_CONFIG, 21);
        assert_eq!(RequestCode::GET_BROKER_RUNTIME_INFO, 28);
        assert_eq!(RequestCode::SEARCH_OFFSET_BY_TIMESTAMP, 29);
        assert_eq!(RequestCode::GET_ALL_SUBSCRIPTIONGROUP_CONFIG, 201);
        assert_eq!(RequestCode::GET_TOPIC_STATS_INFO, 202);
        assert_eq!(RequestCode::GET_CONSUMER_CONNECTION_LIST, 203);
        assert_eq!(RequestCode::GET_CONSUME_STATS, 208);
        assert_eq!(RequestCode::GET_ALL_TOPIC_LIST_FROM_NAMESERVER, 206);
        assert_eq!(RequestCode::DELETE_TOPIC_IN_BROKER, 215);
        assert_eq!(RequestCode::DELETE_TOPIC_IN_NAMESRV, 216);
        assert_eq!(RequestCode::QUERY_CONSUME_QUEUE, 321);
    }

    /// 关键响应码与官方源码数值对齐(ResponseCode.java)
    #[test]
    fn response_codes_match_official_source() {
        assert_eq!(ResponseCode::SUCCESS, 0);
        assert_eq!(ResponseCode::SYSTEM_ERROR, 1);
        assert_eq!(ResponseCode::REQUEST_CODE_NOT_SUPPORTED, 3);
        assert_eq!(ResponseCode::NO_PERMISSION, 16);
        assert_eq!(ResponseCode::TOPIC_NOT_EXIST, 17);
        assert_eq!(ResponseCode::QUERY_NOT_FOUND, 22);
        assert_eq!(ResponseCode::SUBSCRIPTION_GROUP_NOT_EXIST, 26);
        assert_eq!(ResponseCode::CONSUMER_NOT_ONLINE, 206);
    }
}
