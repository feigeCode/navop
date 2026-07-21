use async_trait::async_trait;

use crate::pubsub::RedisPubSubHandle;
use crate::types::*;

/// Redis 连接 trait
#[async_trait]
pub trait RedisConnection: Send + Sync {
    /// 获取配置
    fn config(&self) -> &RedisConnectionConfig;

    /// 连接到 Redis
    async fn connect(&mut self) -> Result<(), RedisError>;

    /// 断开连接
    async fn disconnect(&mut self) -> Result<(), RedisError>;

    /// 测试连接
    async fn ping(&self) -> Result<(), RedisError>;

    /// 是否已连接
    fn is_connected(&self) -> bool;

    // === 基础键操作 ===

    /// 获取键的值（String 类型）
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, RedisError>;

    /// 设置键的值
    async fn set(&self, key: &str, value: &str, ttl: Option<i64>) -> Result<(), RedisError>;
    /// 在指定数据库中设置键的值
    async fn set_in_db(
        &self,
        db: u8,
        key: &str,
        value: &str,
        ttl: Option<i64>,
    ) -> Result<(), RedisError>;

    /// 删除键
    async fn del(&self, keys: &[&str]) -> Result<i64, RedisError>;
    /// 在指定数据库中删除键
    async fn del_in_db(&self, db: u8, keys: &[&str]) -> Result<i64, RedisError>;

    /// 检查键是否存在
    async fn exists(&self, key: &str) -> Result<bool, RedisError>;

    /// 获取匹配模式的键列表（不推荐在生产环境使用）
    async fn keys(&self, pattern: &str) -> Result<Vec<String>, RedisError>;

    /// 扫描键（推荐使用）
    async fn scan(
        &self,
        cursor: u64,
        pattern: &str,
        count: usize,
    ) -> Result<ScanResult, RedisError>;

    /// 在指定数据库中扫描键
    async fn scan_in_db(
        &self,
        db: u8,
        cursor: u64,
        pattern: &str,
        count: usize,
    ) -> Result<ScanResult, RedisError>;

    /// 获取键的类型
    async fn key_type(&self, key: &str) -> Result<RedisKeyType, RedisError>;

    /// 批量获取多个键的类型（Pipeline）
    async fn key_types_batch(
        &self,
        keys: &[String],
    ) -> Result<Vec<(String, RedisKeyType)>, RedisError>;
    /// 在指定数据库中批量获取键类型
    async fn key_types_batch_in_db(
        &self,
        db: u8,
        keys: &[String],
    ) -> Result<Vec<(String, RedisKeyType)>, RedisError>;

    /// 获取键的 TTL（秒）
    async fn ttl(&self, key: &str) -> Result<i64, RedisError>;

    /// 设置键的过期时间
    async fn expire(&self, key: &str, seconds: i64) -> Result<bool, RedisError>;
    /// 在指定数据库中设置键的过期时间
    async fn expire_in_db(&self, db: u8, key: &str, seconds: i64) -> Result<bool, RedisError>;

    /// 移除键的过期时间
    async fn persist(&self, key: &str) -> Result<bool, RedisError>;
    /// 在指定数据库中移除键的过期时间
    async fn persist_in_db(&self, db: u8, key: &str) -> Result<bool, RedisError>;

    /// 重命名键
    async fn rename(&self, old_key: &str, new_key: &str) -> Result<(), RedisError>;
    /// 在指定数据库中重命名键
    async fn rename_in_db(&self, db: u8, old_key: &str, new_key: &str) -> Result<(), RedisError>;

    // === Hash 操作 ===

    /// 获取 Hash 所有字段和值
    async fn hgetall(&self, key: &str) -> Result<Vec<HashField>, RedisError>;

    /// 设置 Hash 字段值
    async fn hset(&self, key: &str, field: &str, value: &str) -> Result<(), RedisError>;
    /// 在指定数据库中设置 Hash 字段值
    async fn hset_in_db(
        &self,
        db: u8,
        key: &str,
        field: &str,
        value: &str,
    ) -> Result<(), RedisError>;

    /// 删除 Hash 字段
    async fn hdel(&self, key: &str, fields: &[&str]) -> Result<i64, RedisError>;
    /// 在指定数据库中删除 Hash 字段
    async fn hdel_in_db(&self, db: u8, key: &str, fields: &[&str]) -> Result<i64, RedisError>;

    /// 获取 Hash 字段数量
    async fn hlen(&self, key: &str) -> Result<i64, RedisError>;

    // === List 操作 ===

    /// 获取 List 范围内的元素
    async fn lrange(&self, key: &str, start: i64, stop: i64) -> Result<Vec<Vec<u8>>, RedisError>;

    /// 从左边推入元素
    async fn lpush(&self, key: &str, values: &[&str]) -> Result<i64, RedisError>;
    /// 在指定数据库中从左边推入元素
    async fn lpush_in_db(&self, db: u8, key: &str, values: &[&str]) -> Result<i64, RedisError>;

    /// 从右边推入元素
    async fn rpush(&self, key: &str, values: &[&str]) -> Result<i64, RedisError>;
    /// 在指定数据库中从右边推入元素
    async fn rpush_in_db(&self, db: u8, key: &str, values: &[&str]) -> Result<i64, RedisError>;

    /// 设置指定索引的元素值
    async fn lset(&self, key: &str, index: i64, value: &str) -> Result<(), RedisError>;
    /// 在指定数据库中设置指定索引的元素值
    async fn lset_in_db(
        &self,
        db: u8,
        key: &str,
        index: i64,
        value: &str,
    ) -> Result<(), RedisError>;

    /// 获取 List 长度
    async fn llen(&self, key: &str) -> Result<i64, RedisError>;

    // === Set 操作 ===

    /// 获取 Set 所有成员
    async fn smembers(&self, key: &str) -> Result<Vec<Vec<u8>>, RedisError>;

    /// 添加成员到 Set
    async fn sadd(&self, key: &str, members: &[&str]) -> Result<i64, RedisError>;
    /// 在指定数据库中添加成员到 Set
    async fn sadd_in_db(&self, db: u8, key: &str, members: &[&str]) -> Result<i64, RedisError>;

    /// 从 Set 移除成员
    async fn srem(&self, key: &str, members: &[&str]) -> Result<i64, RedisError>;
    /// 在指定数据库中从 Set 移除成员
    async fn srem_in_db(&self, db: u8, key: &str, members: &[&str]) -> Result<i64, RedisError>;

    /// 获取 Set 大小
    async fn scard(&self, key: &str) -> Result<i64, RedisError>;

    // === Sorted Set 操作 ===

    /// 获取 ZSet 范围内的成员（带分数）
    async fn zrange_with_scores(
        &self,
        key: &str,
        start: i64,
        stop: i64,
    ) -> Result<Vec<ZSetMember>, RedisError>;

    /// 添加成员到 ZSet
    async fn zadd(&self, key: &str, members: &[(f64, &str)]) -> Result<i64, RedisError>;
    /// 在指定数据库中添加成员到 ZSet
    async fn zadd_in_db(
        &self,
        db: u8,
        key: &str,
        members: &[(f64, &str)],
    ) -> Result<i64, RedisError>;

    /// 从 ZSet 移除成员
    async fn zrem(&self, key: &str, members: &[&str]) -> Result<i64, RedisError>;
    /// 在指定数据库中从 ZSet 移除成员
    async fn zrem_in_db(&self, db: u8, key: &str, members: &[&str]) -> Result<i64, RedisError>;

    /// 获取 ZSet 大小
    async fn zcard(&self, key: &str) -> Result<i64, RedisError>;

    // === Stream 操作 ===

    /// 获取 Stream 条目
    async fn xrange(
        &self,
        key: &str,
        start: &str,
        end: &str,
        count: Option<usize>,
    ) -> Result<Vec<StreamEntry>, RedisError>;

    /// 获取 Stream 长度
    async fn xlen(&self, key: &str) -> Result<i64, RedisError>;

    // === 服务器操作 ===

    /// 获取服务器信息
    async fn info(&self, section: Option<&str>) -> Result<String, RedisError>;

    /// 获取当前数据库键数量
    async fn dbsize(&self) -> Result<i64, RedisError>;

    /// 切换数据库
    async fn select(&self, db: u8) -> Result<(), RedisError>;

    /// 清空当前数据库
    async fn flushdb(&self) -> Result<(), RedisError>;

    /// 执行原始命令
    async fn execute_command(&self, command: &str) -> Result<RedisValue, RedisError>;

    /// 在指定数据库中执行原始命令
    async fn execute_command_in_db(&self, db: u8, command: &str) -> Result<RedisValue, RedisError>;

    // === 辅助方法 ===

    /// 获取键的详细信息
    async fn get_key_info(&self, key: &str) -> Result<KeyInfo, RedisError>;

    /// 获取键值详情
    async fn get_key_value_detail(&self, key: &str) -> Result<KeyValueDetail, RedisError>;

    /// 在指定数据库中获取键值详情
    async fn get_key_value_detail_in_db(
        &self,
        db: u8,
        key: &str,
    ) -> Result<KeyValueDetail, RedisError>;

    /// 获取数据库列表信息
    async fn get_databases_info(&self) -> Result<Vec<RedisDatabaseInfo>, RedisError>;

    /// 获取服务器摘要信息
    async fn get_server_info(&self) -> Result<RedisServerInfo, RedisError>;

    /// 打开一条独立的 Pub/Sub 监听连接。
    ///
    /// 返回的句柄绑定了一条**专用的** Redis 连接(订阅会让连接进入 pub/sub
    /// 模式,无法再发普通命令,因此不能复用主连接池)。句柄 drop 时,后台
    /// 监听任务会优雅停止。
    async fn open_pubsub(&self) -> Result<RedisPubSubHandle, RedisError>;
}
