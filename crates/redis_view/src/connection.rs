//! Redis 连接实现

use crate::types::*;
use async_trait::async_trait;
use redis_client::aio::MultiplexedConnection;
use redis_client::{AsyncCommands, Client, RedisResult};
use rust_i18n::t;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

const MAX_COLLECTION_ELEMENTS: i64 = 1000;

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
    async fn get(&self, key: &str) -> Result<Option<String>, RedisError>;

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
    async fn lrange(&self, key: &str, start: i64, stop: i64) -> Result<Vec<String>, RedisError>;

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
    async fn smembers(&self, key: &str) -> Result<Vec<String>, RedisError>;

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
}

/// Redis 连接实现
pub struct RedisConnectionImpl {
    config: RedisConnectionConfig,
    client: Option<Client>,
    db_connections: Arc<RwLock<HashMap<u8, MultiplexedConnection>>>,
}

impl RedisConnectionImpl {
    pub fn new(config: RedisConnectionConfig) -> Self {
        Self {
            config,
            client: None,
            db_connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn get_conn(&self) -> Result<MultiplexedConnection, RedisError> {
        self.get_db_conn(self.config.db_index).await
    }

    async fn get_db_conn(&self, db: u8) -> Result<MultiplexedConnection, RedisError> {
        if self.client.is_none() {
            return Err(RedisError::NotConnected);
        }

        if let Some(conn) = self.db_connections.read().await.get(&db) {
            return Ok(conn.clone());
        }

        let mut guard = self.db_connections.write().await;
        if let Some(conn) = guard.get(&db) {
            return Ok(conn.clone());
        }

        let (_, conn) = Self::open_connection_for_db(&self.config, db).await?;
        guard.insert(db, conn.clone());
        Ok(conn)
    }

    async fn open_connection_for_db(
        config: &RedisConnectionConfig,
        db: u8,
    ) -> Result<(Client, MultiplexedConnection), RedisError> {
        let db_config = Self::connection_config_for_db(config, db)?;
        let client = Client::open(db_config.to_url().as_str()).map_err(|e| {
            RedisError::connection_with_source(
                t!("RedisConnection.create_client_failed").to_string(),
                e,
            )
        })?;

        let conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| {
                RedisError::connection_with_source(
                    t!("RedisConnection.connect_failed").to_string(),
                    e,
                )
            })?;

        Ok((client, conn))
    }

    fn connection_config_for_db(
        config: &RedisConnectionConfig,
        db: u8,
    ) -> Result<RedisConnectionConfig, RedisError> {
        if config.mode == RedisConnectionMode::Cluster && db != 0 {
            return Err(RedisError::NotSupported(
                "Redis Cluster only supports database 0".to_string(),
            ));
        }

        let mut db_config = config.clone();
        db_config.db_index = db;
        Ok(db_config)
    }

    fn parse_info(info: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for line in info.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some((key, value)) = line.split_once(':') {
                map.insert(key.to_string(), value.to_string());
            }
        }
        map
    }

    fn parse_key_type(type_str: &str) -> RedisKeyType {
        type_str.parse().unwrap_or(RedisKeyType::None)
    }

    fn is_select_command(parts: &[String]) -> bool {
        parts
            .first()
            .is_some_and(|command| command.eq_ignore_ascii_case("SELECT"))
    }

    fn reject_select_command(parts: &[String]) -> Result<(), RedisError> {
        if Self::is_select_command(parts) {
            return Err(RedisError::NotSupported(
                "SELECT is not supported on multiplexed Redis connections; open the target database tab instead"
                    .to_string(),
            ));
        }

        Ok(())
    }

    async fn key_type_with_conn(
        conn: &mut MultiplexedConnection,
        key: &str,
    ) -> Result<RedisKeyType, RedisError> {
        let type_str: String = redis_client::cmd("TYPE")
            .arg(key)
            .query_async(&mut *conn)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "TYPE").to_string(),
                    e,
                )
            })?;
        Ok(Self::parse_key_type(&type_str))
    }

    async fn ttl_with_conn(conn: &mut MultiplexedConnection, key: &str) -> Result<i64, RedisError> {
        redis_client::cmd("TTL")
            .arg(key)
            .query_async(&mut *conn)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "TTL").to_string(),
                    e,
                )
            })
    }

    async fn key_size_with_conn(
        conn: &mut MultiplexedConnection,
        key: &str,
        key_type: RedisKeyType,
    ) -> Option<i64> {
        let command = match key_type {
            RedisKeyType::String => "STRLEN",
            RedisKeyType::List => "LLEN",
            RedisKeyType::Set => "SCARD",
            RedisKeyType::ZSet => "ZCARD",
            RedisKeyType::Hash => "HLEN",
            RedisKeyType::Stream => "XLEN",
            RedisKeyType::None => return None,
        };
        redis_client::cmd(command)
            .arg(key)
            .query_async::<i64>(&mut *conn)
            .await
            .ok()
    }

    async fn key_info_with_conn(
        conn: &mut MultiplexedConnection,
        key: &str,
    ) -> Result<KeyInfo, RedisError> {
        let key_type = Self::key_type_with_conn(conn, key).await?;
        if key_type == RedisKeyType::None {
            return Err(RedisError::KeyNotFound(key.to_string()));
        }

        let ttl = Self::ttl_with_conn(conn, key).await?;
        let size = Self::key_size_with_conn(conn, key, key_type).await;

        Ok(KeyInfo {
            name: key.to_string(),
            key_type,
            ttl,
            size,
            memory_usage: None,
        })
    }

    async fn scan_set_members(
        conn: &mut MultiplexedConnection,
        key: &str,
    ) -> Result<Vec<String>, RedisError> {
        let mut cursor: u64 = 0;
        let mut members: Vec<String> = Vec::new();
        loop {
            let (next, batch): (u64, Vec<String>) = redis_client::cmd("SSCAN")
                .arg(key)
                .arg(cursor)
                .arg("COUNT")
                .arg(200)
                .query_async(&mut *conn)
                .await
                .map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "SSCAN").to_string(),
                        e,
                    )
                })?;
            members.extend(batch);
            cursor = next;
            if cursor == 0 || members.len() >= MAX_COLLECTION_ELEMENTS as usize {
                break;
            }
        }
        members.truncate(MAX_COLLECTION_ELEMENTS as usize);
        Ok(members)
    }

    async fn scan_hash_fields(
        conn: &mut MultiplexedConnection,
        key: &str,
    ) -> Result<Vec<HashField>, RedisError> {
        let mut cursor: u64 = 0;
        let mut fields: Vec<HashField> = Vec::new();
        loop {
            let (next, batch): (u64, Vec<(String, String)>) = redis_client::cmd("HSCAN")
                .arg(key)
                .arg(cursor)
                .arg("COUNT")
                .arg(200)
                .query_async(&mut *conn)
                .await
                .map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "HSCAN").to_string(),
                        e,
                    )
                })?;
            fields.extend(
                batch
                    .into_iter()
                    .map(|(field, value)| HashField { field, value }),
            );
            cursor = next;
            if cursor == 0 || fields.len() >= MAX_COLLECTION_ELEMENTS as usize {
                break;
            }
        }
        fields.truncate(MAX_COLLECTION_ELEMENTS as usize);
        Ok(fields)
    }

    async fn zrange_with_scores_conn(
        conn: &mut MultiplexedConnection,
        key: &str,
    ) -> Result<Vec<ZSetMember>, RedisError> {
        let result: Vec<(String, f64)> = conn
            .zrange_withscores(key, 0, (MAX_COLLECTION_ELEMENTS - 1) as isize)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "ZRANGE").to_string(),
                    e,
                )
            })?;
        Ok(result
            .into_iter()
            .map(|(member, score)| ZSetMember { member, score })
            .collect())
    }

    async fn xrange_conn(
        conn: &mut MultiplexedConnection,
        key: &str,
    ) -> Result<Vec<StreamEntry>, RedisError> {
        let result: Vec<(String, Vec<(String, String)>)> = redis_client::cmd("XRANGE")
            .arg(key)
            .arg("-")
            .arg("+")
            .arg("COUNT")
            .arg(100)
            .query_async(&mut *conn)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "XRANGE").to_string(),
                    e,
                )
            })?;
        Ok(result
            .into_iter()
            .map(|(id, fields)| StreamEntry {
                id,
                fields: fields.into_iter().collect(),
            })
            .collect())
    }

    async fn key_value_detail_with_conn(
        conn: &mut MultiplexedConnection,
        key: &str,
    ) -> Result<KeyValueDetail, RedisError> {
        let key_info = Self::key_info_with_conn(conn, key).await?;
        let value = match key_info.key_type {
            RedisKeyType::String => {
                let value = redis_client::cmd("GET")
                    .arg(key)
                    .query_async::<Option<String>>(&mut *conn)
                    .await
                    .map_err(|e| {
                        RedisError::command_with_source(
                            t!("RedisConnection.command_failed", command = "GET").to_string(),
                            e,
                        )
                    })?;
                KeyValueContent::String(value.unwrap_or_default())
            }
            RedisKeyType::List => {
                let value = conn
                    .lrange(key, 0, (MAX_COLLECTION_ELEMENTS - 1) as isize)
                    .await
                    .map_err(|e| {
                        RedisError::command_with_source(
                            t!("RedisConnection.command_failed", command = "LRANGE").to_string(),
                            e,
                        )
                    })?;
                KeyValueContent::List(value)
            }
            RedisKeyType::Set => KeyValueContent::Set(Self::scan_set_members(conn, key).await?),
            RedisKeyType::ZSet => {
                KeyValueContent::ZSet(Self::zrange_with_scores_conn(conn, key).await?)
            }
            RedisKeyType::Hash => KeyValueContent::Hash(Self::scan_hash_fields(conn, key).await?),
            RedisKeyType::Stream => KeyValueContent::Stream(Self::xrange_conn(conn, key).await?),
            RedisKeyType::None => KeyValueContent::None,
        };

        Ok(KeyValueDetail { key_info, value })
    }
}

#[async_trait]
impl RedisConnection for RedisConnectionImpl {
    fn config(&self) -> &RedisConnectionConfig {
        &self.config
    }

    async fn connect(&mut self) -> Result<(), RedisError> {
        let (client, conn) =
            Self::open_connection_for_db(&self.config, self.config.db_index).await?;
        self.client = Some(client);
        let mut connections = self.db_connections.write().await;
        connections.clear();
        connections.insert(self.config.db_index, conn);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), RedisError> {
        self.db_connections.write().await.clear();
        self.client = None;
        Ok(())
    }

    async fn ping(&self) -> Result<(), RedisError> {
        let mut conn = self.get_conn().await?;
        redis_client::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "PING").to_string(),
                    e,
                )
            })?;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.client.is_some()
    }

    async fn get(&self, key: &str) -> Result<Option<String>, RedisError> {
        let mut conn = self.get_conn().await?;
        let result: RedisResult<Option<String>> = conn.get(key).await;
        result.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "GET").to_string(),
                e,
            )
        })
    }

    async fn set(&self, key: &str, value: &str, ttl: Option<i64>) -> Result<(), RedisError> {
        self.set_in_db(self.config.db_index, key, value, ttl).await
    }

    async fn set_in_db(
        &self,
        db: u8,
        key: &str,
        value: &str,
        ttl: Option<i64>,
    ) -> Result<(), RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        if let Some(ttl) = ttl {
            conn.set_ex(key, value, ttl as u64).await.map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "SETEX").to_string(),
                    e,
                )
            })
        } else {
            conn.set(key, value).await.map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "SET").to_string(),
                    e,
                )
            })
        }
    }

    async fn del(&self, keys: &[&str]) -> Result<i64, RedisError> {
        self.del_in_db(self.config.db_index, keys).await
    }

    async fn del_in_db(&self, db: u8, keys: &[&str]) -> Result<i64, RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        conn.del(keys).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "DEL").to_string(),
                e,
            )
        })
    }

    async fn exists(&self, key: &str) -> Result<bool, RedisError> {
        let mut conn = self.get_conn().await?;
        let count: i64 = conn.exists(key).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "EXISTS").to_string(),
                e,
            )
        })?;
        Ok(count > 0)
    }

    async fn keys(&self, pattern: &str) -> Result<Vec<String>, RedisError> {
        let mut conn = self.get_conn().await?;
        conn.keys(pattern).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "KEYS").to_string(),
                e,
            )
        })
    }

    async fn scan(
        &self,
        cursor: u64,
        pattern: &str,
        count: usize,
    ) -> Result<ScanResult, RedisError> {
        let mut conn = self.get_conn().await?;
        let (next_cursor, keys): (u64, Vec<String>) = redis_client::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(count)
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "SCAN").to_string(),
                    e,
                )
            })?;
        Ok(ScanResult::new(next_cursor, keys))
    }

    async fn scan_in_db(
        &self,
        db: u8,
        cursor: u64,
        pattern: &str,
        count: usize,
    ) -> Result<ScanResult, RedisError> {
        let mut conn = self.get_db_conn(db).await?;

        let (next_cursor, keys): (u64, Vec<String>) = redis_client::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(count)
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "SCAN").to_string(),
                    e,
                )
            })?;

        Ok(ScanResult::new(next_cursor, keys))
    }

    async fn key_type(&self, key: &str) -> Result<RedisKeyType, RedisError> {
        let mut conn = self.get_conn().await?;
        Self::key_type_with_conn(&mut conn, key).await
    }

    async fn key_types_batch(
        &self,
        keys: &[String],
    ) -> Result<Vec<(String, RedisKeyType)>, RedisError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let mut conn = self.get_conn().await?;
        let mut pipe = redis_client::pipe();
        for key in keys {
            pipe.cmd("TYPE").arg(key);
        }

        let results: Vec<String> = pipe.query_async(&mut conn).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "TYPE (batch)").to_string(),
                e,
            )
        })?;

        Ok(keys
            .iter()
            .cloned()
            .zip(results.into_iter().map(|s| Self::parse_key_type(&s)))
            .collect())
    }

    async fn key_types_batch_in_db(
        &self,
        db: u8,
        keys: &[String],
    ) -> Result<Vec<(String, RedisKeyType)>, RedisError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let mut conn = self.get_db_conn(db).await?;
        let mut pipe = redis_client::pipe();
        for key in keys {
            pipe.cmd("TYPE").arg(key);
        }

        let results: Vec<String> = pipe.query_async(&mut conn).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "TYPE (batch)").to_string(),
                e,
            )
        })?;

        Ok(keys
            .iter()
            .cloned()
            .zip(results.into_iter().map(|s| Self::parse_key_type(&s)))
            .collect())
    }

    async fn ttl(&self, key: &str) -> Result<i64, RedisError> {
        let mut conn = self.get_conn().await?;
        Self::ttl_with_conn(&mut conn, key).await
    }

    async fn expire(&self, key: &str, seconds: i64) -> Result<bool, RedisError> {
        self.expire_in_db(self.config.db_index, key, seconds).await
    }

    async fn expire_in_db(&self, db: u8, key: &str, seconds: i64) -> Result<bool, RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        let result: i64 = conn.expire(key, seconds).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "EXPIRE").to_string(),
                e,
            )
        })?;
        Ok(result == 1)
    }

    async fn persist(&self, key: &str) -> Result<bool, RedisError> {
        self.persist_in_db(self.config.db_index, key).await
    }

    async fn persist_in_db(&self, db: u8, key: &str) -> Result<bool, RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        let result: i64 = conn.persist(key).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "PERSIST").to_string(),
                e,
            )
        })?;
        Ok(result == 1)
    }

    async fn rename(&self, old_key: &str, new_key: &str) -> Result<(), RedisError> {
        self.rename_in_db(self.config.db_index, old_key, new_key)
            .await
    }

    async fn rename_in_db(&self, db: u8, old_key: &str, new_key: &str) -> Result<(), RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        conn.rename(old_key, new_key).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "RENAME").to_string(),
                e,
            )
        })
    }

    async fn hgetall(&self, key: &str) -> Result<Vec<HashField>, RedisError> {
        let mut conn = self.get_conn().await?;
        let result: Vec<(String, String)> = conn.hgetall(key).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "HGETALL").to_string(),
                e,
            )
        })?;
        Ok(result
            .into_iter()
            .map(|(field, value)| HashField { field, value })
            .collect())
    }

    async fn hset(&self, key: &str, field: &str, value: &str) -> Result<(), RedisError> {
        self.hset_in_db(self.config.db_index, key, field, value)
            .await
    }

    async fn hset_in_db(
        &self,
        db: u8,
        key: &str,
        field: &str,
        value: &str,
    ) -> Result<(), RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        conn.hset(key, field, value).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "HSET").to_string(),
                e,
            )
        })
    }

    async fn hdel(&self, key: &str, fields: &[&str]) -> Result<i64, RedisError> {
        self.hdel_in_db(self.config.db_index, key, fields).await
    }

    async fn hdel_in_db(&self, db: u8, key: &str, fields: &[&str]) -> Result<i64, RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        conn.hdel(key, fields).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "HDEL").to_string(),
                e,
            )
        })
    }

    async fn hlen(&self, key: &str) -> Result<i64, RedisError> {
        let mut conn = self.get_conn().await?;
        conn.hlen(key).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "HLEN").to_string(),
                e,
            )
        })
    }

    async fn lrange(&self, key: &str, start: i64, stop: i64) -> Result<Vec<String>, RedisError> {
        let mut conn = self.get_conn().await?;
        conn.lrange(key, start as isize, stop as isize)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "LRANGE").to_string(),
                    e,
                )
            })
    }

    async fn lpush(&self, key: &str, values: &[&str]) -> Result<i64, RedisError> {
        self.lpush_in_db(self.config.db_index, key, values).await
    }

    async fn lpush_in_db(&self, db: u8, key: &str, values: &[&str]) -> Result<i64, RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        conn.lpush(key, values).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "LPUSH").to_string(),
                e,
            )
        })
    }

    async fn rpush(&self, key: &str, values: &[&str]) -> Result<i64, RedisError> {
        self.rpush_in_db(self.config.db_index, key, values).await
    }

    async fn rpush_in_db(&self, db: u8, key: &str, values: &[&str]) -> Result<i64, RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        conn.rpush(key, values).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "RPUSH").to_string(),
                e,
            )
        })
    }

    async fn lset(&self, key: &str, index: i64, value: &str) -> Result<(), RedisError> {
        self.lset_in_db(self.config.db_index, key, index, value)
            .await
    }

    async fn lset_in_db(
        &self,
        db: u8,
        key: &str,
        index: i64,
        value: &str,
    ) -> Result<(), RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        conn.lset(key, index as isize, value).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "LSET").to_string(),
                e,
            )
        })
    }

    async fn llen(&self, key: &str) -> Result<i64, RedisError> {
        let mut conn = self.get_conn().await?;
        conn.llen(key).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "LLEN").to_string(),
                e,
            )
        })
    }

    async fn smembers(&self, key: &str) -> Result<Vec<String>, RedisError> {
        let mut conn = self.get_conn().await?;
        conn.smembers(key).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "SMEMBERS").to_string(),
                e,
            )
        })
    }

    async fn sadd(&self, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        self.sadd_in_db(self.config.db_index, key, members).await
    }

    async fn sadd_in_db(&self, db: u8, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        conn.sadd(key, members).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "SADD").to_string(),
                e,
            )
        })
    }

    async fn srem(&self, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        self.srem_in_db(self.config.db_index, key, members).await
    }

    async fn srem_in_db(&self, db: u8, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        conn.srem(key, members).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "SREM").to_string(),
                e,
            )
        })
    }

    async fn scard(&self, key: &str) -> Result<i64, RedisError> {
        let mut conn = self.get_conn().await?;
        conn.scard(key).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "SCARD").to_string(),
                e,
            )
        })
    }

    async fn zrange_with_scores(
        &self,
        key: &str,
        start: i64,
        stop: i64,
    ) -> Result<Vec<ZSetMember>, RedisError> {
        let mut conn = self.get_conn().await?;
        let result: Vec<(String, f64)> = conn
            .zrange_withscores(key, start as isize, stop as isize)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "ZRANGE").to_string(),
                    e,
                )
            })?;
        Ok(result
            .into_iter()
            .map(|(member, score)| ZSetMember { member, score })
            .collect())
    }

    async fn zadd(&self, key: &str, members: &[(f64, &str)]) -> Result<i64, RedisError> {
        self.zadd_in_db(self.config.db_index, key, members).await
    }

    async fn zadd_in_db(
        &self,
        db: u8,
        key: &str,
        members: &[(f64, &str)],
    ) -> Result<i64, RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        let items: Vec<(f64, &str)> = members.iter().map(|(s, m)| (*s, *m)).collect();
        conn.zadd_multiple(key, &items).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "ZADD").to_string(),
                e,
            )
        })
    }

    async fn zrem(&self, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        self.zrem_in_db(self.config.db_index, key, members).await
    }

    async fn zrem_in_db(&self, db: u8, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        conn.zrem(key, members).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "ZREM").to_string(),
                e,
            )
        })
    }

    async fn zcard(&self, key: &str) -> Result<i64, RedisError> {
        let mut conn = self.get_conn().await?;
        conn.zcard(key).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "ZCARD").to_string(),
                e,
            )
        })
    }

    async fn xrange(
        &self,
        key: &str,
        start: &str,
        end: &str,
        count: Option<usize>,
    ) -> Result<Vec<StreamEntry>, RedisError> {
        let mut conn = self.get_conn().await?;
        let mut cmd = redis_client::cmd("XRANGE");
        cmd.arg(key).arg(start).arg(end);
        if let Some(c) = count {
            cmd.arg("COUNT").arg(c);
        }
        let result: Vec<(String, Vec<(String, String)>)> =
            cmd.query_async(&mut conn).await.map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "XRANGE").to_string(),
                    e,
                )
            })?;

        Ok(result
            .into_iter()
            .map(|(id, fields)| StreamEntry {
                id,
                fields: fields.into_iter().collect(),
            })
            .collect())
    }

    async fn xlen(&self, key: &str) -> Result<i64, RedisError> {
        let mut conn = self.get_conn().await?;
        redis_client::cmd("XLEN")
            .arg(key)
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "XLEN").to_string(),
                    e,
                )
            })
    }

    async fn info(&self, section: Option<&str>) -> Result<String, RedisError> {
        let mut conn = self.get_conn().await?;
        let mut cmd = redis_client::cmd("INFO");
        if let Some(s) = section {
            cmd.arg(s);
        }
        cmd.query_async(&mut conn).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "INFO").to_string(),
                e,
            )
        })
    }

    async fn dbsize(&self) -> Result<i64, RedisError> {
        let mut conn = self.get_conn().await?;
        redis_client::cmd("DBSIZE")
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "DBSIZE").to_string(),
                    e,
                )
            })
    }

    async fn select(&self, db: u8) -> Result<(), RedisError> {
        self.get_db_conn(db).await.map(|_| ())
    }

    async fn flushdb(&self) -> Result<(), RedisError> {
        let mut conn = self.get_conn().await?;
        redis_client::cmd("FLUSHDB")
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "FLUSHDB").to_string(),
                    e,
                )
            })
    }

    async fn execute_command(&self, command: &str) -> Result<RedisValue, RedisError> {
        let mut conn = self.get_conn().await?;
        let parts = parse_command_args(command);
        if parts.is_empty() {
            return Err(RedisError::command(
                t!("RedisConnection.empty_command").to_string(),
            ));
        }
        Self::reject_select_command(&parts)?;

        let mut cmd = redis_client::cmd(parts[0].as_str());
        for arg in &parts[1..] {
            cmd.arg(arg.as_str());
        }

        let result: redis_client::Value = cmd.query_async(&mut conn).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_execute_failed").to_string(),
                e,
            )
        })?;

        Ok(convert_redis_value(result))
    }

    async fn execute_command_in_db(&self, db: u8, command: &str) -> Result<RedisValue, RedisError> {
        let parts = parse_command_args(command);
        if parts.is_empty() {
            return Err(RedisError::command(
                t!("RedisConnection.empty_command").to_string(),
            ));
        }
        Self::reject_select_command(&parts)?;

        let mut conn = self.get_db_conn(db).await?;
        let mut cmd = redis_client::cmd(parts[0].as_str());
        for arg in &parts[1..] {
            cmd.arg(arg.as_str());
        }

        let result: redis_client::Value = cmd.query_async(&mut conn).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_execute_failed").to_string(),
                e,
            )
        })?;

        Ok(convert_redis_value(result))
    }

    async fn get_key_info(&self, key: &str) -> Result<KeyInfo, RedisError> {
        let mut conn = self.get_conn().await?;
        Self::key_info_with_conn(&mut conn, key).await
    }

    async fn get_key_value_detail(&self, key: &str) -> Result<KeyValueDetail, RedisError> {
        let mut conn = self.get_conn().await?;
        Self::key_value_detail_with_conn(&mut conn, key).await
    }

    async fn get_key_value_detail_in_db(
        &self,
        db: u8,
        key: &str,
    ) -> Result<KeyValueDetail, RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        Self::key_value_detail_with_conn(&mut conn, key).await
    }

    async fn get_databases_info(&self) -> Result<Vec<RedisDatabaseInfo>, RedisError> {
        let info = self.info(Some("keyspace")).await?;
        let mut databases_map = HashMap::new();

        for line in info.lines() {
            if line.starts_with("db") {
                if let Some((db_str, stats)) = line.split_once(':') {
                    if let Ok(index) = db_str[2..].parse::<u8>() {
                        let mut keys = 0i64;
                        let mut expires = 0i64;
                        let mut avg_ttl = 0i64;

                        for part in stats.split(',') {
                            if let Some((k, v)) = part.split_once('=') {
                                match k {
                                    "keys" => keys = v.parse().unwrap_or(0),
                                    "expires" => expires = v.parse().unwrap_or(0),
                                    "avg_ttl" => avg_ttl = v.parse().unwrap_or(0),
                                    _ => {}
                                }
                            }
                        }

                        databases_map.insert(
                            index,
                            RedisDatabaseInfo {
                                index,
                                keys,
                                expires,
                                avg_ttl,
                            },
                        );
                    }
                }
            }
        }

        let mut max_index = databases_map.keys().copied().max().unwrap_or(0);
        if max_index < 15 {
            max_index = 15;
        }

        let mut databases = Vec::with_capacity(max_index as usize + 1);
        for index in 0..=max_index {
            if let Some(db_info) = databases_map.remove(&index) {
                databases.push(db_info);
            } else {
                databases.push(RedisDatabaseInfo {
                    index,
                    keys: 0,
                    expires: 0,
                    avg_ttl: 0,
                });
            }
        }

        Ok(databases)
    }

    async fn get_server_info(&self) -> Result<RedisServerInfo, RedisError> {
        let info = self.info(None).await?;
        let map = Self::parse_info(&info);

        Ok(RedisServerInfo {
            version: map.get("redis_version").cloned().unwrap_or_default(),
            mode: map
                .get("redis_mode")
                .cloned()
                .unwrap_or_else(|| "standalone".to_string()),
            os: map.get("os").cloned().unwrap_or_default(),
            connected_clients: map
                .get("connected_clients")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            used_memory: map
                .get("used_memory")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            used_memory_human: map.get("used_memory_human").cloned().unwrap_or_default(),
            total_keys: 0, // 需要单独查询
            uptime_in_seconds: map
                .get("uptime_in_seconds")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(mode: RedisConnectionMode, db_index: u8) -> RedisConnectionConfig {
        RedisConnectionConfig {
            id: "test".to_string(),
            name: "test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 6379,
            password: None,
            username: None,
            db_index,
            use_tls: false,
            timeout: 10,
            mode,
        }
    }

    #[test]
    fn connection_config_for_db_uses_requested_db_in_standalone_url() {
        let base = config(RedisConnectionMode::Standalone, 0);

        let db_config = RedisConnectionImpl::connection_config_for_db(&base, 7).unwrap();

        assert_eq!(7, db_config.db_index);
        assert_eq!("redis://127.0.0.1:6379/7", db_config.to_url());
    }

    #[test]
    fn connection_config_for_db_rejects_non_zero_cluster_db() {
        let base = config(RedisConnectionMode::Cluster, 0);

        let err = RedisConnectionImpl::connection_config_for_db(&base, 1).unwrap_err();

        assert!(matches!(err, RedisError::NotSupported(_)));
    }

    #[test]
    fn parse_key_type_returns_none_for_unknown_type() {
        assert_eq!(
            RedisKeyType::None,
            RedisConnectionImpl::parse_key_type("unexpected")
        );
    }

    #[test]
    fn command_name_detects_select_case_insensitively() {
        assert!(RedisConnectionImpl::is_select_command(&[
            "select".to_string()
        ]));
        assert!(RedisConnectionImpl::is_select_command(&[
            "SELECT".to_string()
        ]));
        assert!(!RedisConnectionImpl::is_select_command(
            &["get".to_string()]
        ));
        assert!(!RedisConnectionImpl::is_select_command(&[]));
    }
}

fn parse_command_args(command: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' => {
                escaped = true;
            }
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

/// 转换 redis 库的 Value 到我们的 RedisValue
fn convert_redis_value(value: redis_client::Value) -> RedisValue {
    match value {
        redis_client::Value::Nil => RedisValue::Nil,
        redis_client::Value::Int(i) => RedisValue::Integer(i),
        redis_client::Value::BulkString(bytes) => match String::from_utf8(bytes.clone()) {
            Ok(s) => RedisValue::String(s),
            Err(_) => RedisValue::Binary(bytes),
        },
        redis_client::Value::Array(arr) => {
            RedisValue::Bulk(arr.into_iter().map(convert_redis_value).collect())
        }
        redis_client::Value::SimpleString(s) => RedisValue::Status(s),
        redis_client::Value::Okay => RedisValue::Status("OK".to_string()),
        redis_client::Value::Double(f) => RedisValue::Float(f),
        redis_client::Value::Boolean(b) => RedisValue::Integer(if b { 1 } else { 0 }),
        _ => RedisValue::Nil,
    }
}
