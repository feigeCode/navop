//! Redis 连接实现

use crate::builtin_pubsub as redis_pubsub;
use crate::key_codec;
use crate::types::*;
use crate::{RedisConnection, RedisPubSubHandle};
use async_trait::async_trait;
use redis_client::aio::{ConnectionManager, ConnectionManagerConfig};
use redis_client::{AsyncCommands, Client, RedisResult};
use rust_i18n::t;
use ssh::{
    HostKeyVerifier, LocalPortForwardTunnel, SshAuth, SshConnectConfig, start_local_port_forward,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::timeout;

const MAX_COLLECTION_ELEMENTS: i64 = 1000;
/// 单次加载集合类键值的内容预算（字节）。与条数上限共同保证内存有界。
const MAX_COLLECTION_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_CONNECTION_TIMEOUT_SECONDS: u64 = 10;
const DEFAULT_SSH_TIMEOUT_SECONDS: u64 = 30;

/// 集合类键值的加载预算：条数与字节任一触顶即停止，保证单次加载内存有界。
struct CollectionBudget {
    elements: usize,
    bytes: usize,
    truncated: bool,
}

impl CollectionBudget {
    fn new() -> Self {
        Self {
            elements: 0,
            bytes: 0,
            truncated: false,
        }
    }

    /// 纳入一个元素（给出其占用字节数）；返回 false 表示已达预算，调用方应停止。
    fn accept(&mut self, bytes: usize) -> bool {
        if self.elements >= MAX_COLLECTION_ELEMENTS as usize || self.bytes >= MAX_COLLECTION_BYTES {
            self.truncated = true;
            return false;
        }
        self.elements += 1;
        self.bytes = self.bytes.saturating_add(bytes);
        true
    }
}

fn string_content_from_bytes(value: Option<Vec<u8>>) -> KeyValueContent {
    KeyValueContent::String(value.unwrap_or_default())
}

struct ResolvedRedisConnectionTarget {
    host: String,
    port: u16,
    tunnel: Option<LocalPortForwardTunnel>,
}

fn normalize_direct_host(host: &str) -> String {
    if host.eq_ignore_ascii_case("localhost") {
        return "127.0.0.1".to_string();
    }

    host.to_string()
}

fn required_ssh_value(value: &str, key: &str) -> Result<String, RedisError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(RedisError::connection(format!(
            "ssh tunnel enabled but `{key}` is missing"
        )));
    }

    Ok(value.to_string())
}

fn build_ssh_auth(
    tunnel_config: &one_core::storage::RedisSshTunnelConfig,
) -> Result<SshAuth, RedisError> {
    match tunnel_config.auth_type.trim().to_ascii_lowercase().as_str() {
        "agent" => Ok(SshAuth::Agent),
        "pageant" => Ok(SshAuth::Pageant),
        "auto_publickey" | "auto_public_key" => Ok(SshAuth::AutoPublicKey),
        "private_key" => {
            let key_path = tunnel_config
                .private_key_path
                .as_deref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    RedisError::connection(
                        "ssh tunnel enabled but `ssh_private_key_path` is missing",
                    )
                })?;
            Ok(SshAuth::PrivateKey {
                key_path: key_path.to_string(),
                passphrase: tunnel_config.private_key_passphrase.clone(),
                certificate_path: None,
            })
        }
        "private_key_content" | "private_key_material" => {
            let private_key = tunnel_config
                .private_key_content
                .as_deref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    RedisError::connection(
                        "ssh tunnel enabled but `ssh_private_key_content` is missing",
                    )
                })?;
            Ok(SshAuth::PrivateKeyContent {
                private_key: private_key.to_string(),
                passphrase: tunnel_config.private_key_passphrase.clone(),
                certificate_path: None,
            })
        }
        // 组合认证：一个隧道配置里同时带密码与私钥两个因素。
        "password_and_private_key" => {
            let password = tunnel_config
                .password
                .as_deref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    RedisError::connection("ssh tunnel enabled but `ssh_password` is missing")
                })?;
            let content = tunnel_config
                .private_key_content
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let key_path = tunnel_config
                .private_key_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let key = match (content, key_path) {
                (Some(private_key), _) => SshAuth::PrivateKeyContent {
                    private_key: private_key.to_string(),
                    passphrase: tunnel_config.private_key_passphrase.clone(),
                    certificate_path: None,
                },
                (None, Some(key_path)) => SshAuth::PrivateKey {
                    key_path: key_path.to_string(),
                    passphrase: tunnel_config.private_key_passphrase.clone(),
                    certificate_path: None,
                },
                (None, None) => {
                    return Err(RedisError::connection(
                        "ssh tunnel enabled but `ssh_private_key_path` is missing",
                    ));
                }
            };
            Ok(SshAuth::Chain(vec![
                SshAuth::Password(password.to_string()),
                key,
            ]))
        }
        _ => {
            let password = tunnel_config
                .password
                .as_deref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    RedisError::connection("ssh tunnel enabled but `ssh_password` is missing")
                })?;
            Ok(SshAuth::Password(password.to_string()))
        }
    }
}

/// Redis 连接实现
pub struct RedisConnectionImpl {
    config: RedisConnectionConfig,
    client: Option<Client>,
    db_connections: Arc<RwLock<HashMap<u8, ConnectionManager>>>,
    /// 集群模式：按端点缓存的节点连接（MOVED 重试目标），与会话同生命周期。
    cluster_conns: Arc<tokio::sync::Mutex<HashMap<(String, u16), ConnectionManager>>>,
    tunnel: Option<LocalPortForwardTunnel>,
    resolved_endpoint: Option<(String, u16)>,
}

impl RedisConnectionImpl {
    pub fn new(config: RedisConnectionConfig) -> Self {
        Self {
            config,
            client: None,
            db_connections: Arc::new(RwLock::new(HashMap::new())),
            cluster_conns: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            tunnel: None,
            resolved_endpoint: None,
        }
    }

    fn current_endpoint(&self) -> (&str, u16) {
        match self.resolved_endpoint.as_ref() {
            Some((host, port)) => (host.as_str(), *port),
            None => (self.config.host.as_str(), self.config.port),
        }
    }

    async fn get_conn(&self) -> Result<ConnectionManager, RedisError> {
        self.get_db_conn(self.config.db_index).await
    }

    /// 集群模式下需要聚合的操作（SCAN/TYPE/DEL）是否应分发到全部 master。
    /// 仅 Cluster 模式且填了节点列表时启用；SSH 隧道下集群节点地址
    /// 不可直达，保持单端点行为（ponytail: 隧道内集群需逐节点配隧道后再支持）。
    fn cluster_fanout_enabled(&self) -> bool {
        self.config.mode == RedisConnectionMode::Cluster
            && !self.config.cluster_nodes.is_empty()
            && !self.config.ssh_tunnel.as_ref().is_some_and(|t| t.enabled)
    }

    /// 集群模式：解析用户填写的节点列表为 (host, port)，容错格式。
    fn parse_cluster_nodes(&self) -> Vec<(String, u16)> {
        self.config
            .cluster_nodes
            .iter()
            .filter_map(|entry| {
                let entry = entry.trim();
                if entry.is_empty() {
                    return None;
                }
                let (host, port) = entry.rsplit_once(':')?;
                let port: u16 = port.parse().ok()?;
                Some((normalize_direct_host(host), port))
            })
            .collect()
    }

    /// 逐个尝试节点列表直到一个能连上，返回可用 master 连接。
    async fn cluster_any_conn(&self) -> Result<ConnectionManager, RedisError> {
        if self.client.is_none() {
            return Err(RedisError::NotConnected);
        }
        let mut last_err = None;
        for (host, port) in self.parse_cluster_nodes() {
            match Self::open_connection_for_endpoint(&self.config, 0, &host, port).await {
                Ok((_, conn)) => return Ok(conn),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            RedisError::connection("cluster: no reachable node in the configured list")
        }))
    }

    /// 集群模式：通过 CLUSTER SLOTS 发现全部 master 端点，去重。
    /// 每次调用重新发现（调用方低频，连接自带 ConnectionManager 缓存，
    /// 避免引入失效缓存问题）。任一节点能答即可。
    async fn cluster_master_endpoints(&self) -> Result<Vec<(String, u16)>, RedisError> {
        let mut conn = self.cluster_any_conn().await?;
        let slots: redis_client::Value = redis_client::cmd("CLUSTER")
            .arg("SLOTS")
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "CLUSTER SLOTS").to_string(),
                    e,
                )
            })?;

        // CLUSTER SLOTS 返回 [[start, end, [host, port, id], [replica...]], ...]。
        // 手动解析：每个 slot 区间的第一个节点即 master。
        let mut masters = Vec::new();
        if let redis_client::Value::Array(slot_rows) = slots {
            for row in slot_rows {
                let redis_client::Value::Array(entry) = row else {
                    continue;
                };
                // entry: [start_slot, end_slot, master_info, replicas...]
                if entry.len() < 3 {
                    continue;
                }
                let Some(redis_client::Value::Array(node)) = entry.get(2) else {
                    continue;
                };
                let (
                    Some(redis_client::Value::BulkString(host_bytes)),
                    Some(redis_client::Value::Int(port)),
                ) = (node.first(), node.get(1))
                else {
                    continue;
                };
                let host = String::from_utf8_lossy(host_bytes).to_string();
                let Ok(port) = u16::try_from(*port) else {
                    continue;
                };
                let endpoint = (normalize_direct_host(&host), port);
                if !masters.contains(&endpoint) {
                    masters.push(endpoint);
                }
            }
        }
        if masters.is_empty() {
            return Err(RedisError::connection(
                "cluster: CLUSTER SLOTS returned no master endpoints",
            ));
        }
        Ok(masters)
    }

    async fn get_db_conn(&self, db: u8) -> Result<ConnectionManager, RedisError> {
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

        let (host, port) = self.current_endpoint();
        let (_, conn) = Self::open_connection_for_endpoint(&self.config, db, host, port).await?;
        guard.insert(db, conn.clone());
        Ok(conn)
    }

    async fn reconnect_db_conn(&self, db: u8) -> Result<ConnectionManager, RedisError> {
        if self.client.is_none() {
            return Err(RedisError::NotConnected);
        }

        // 先移除可能已失效的管理器：即使重连失败，后续调用也不会再命中死连接。
        self.db_connections.write().await.remove(&db);

        let (host, port) = self.current_endpoint();
        let (_, conn) = Self::open_connection_for_endpoint(&self.config, db, host, port).await?;
        self.db_connections.write().await.insert(db, conn.clone());
        Ok(conn)
    }

    async fn open_connection_for_endpoint(
        config: &RedisConnectionConfig,
        db: u8,
        host: &str,
        port: u16,
    ) -> Result<(Client, ConnectionManager), RedisError> {
        let db_config = Self::connection_config_for_endpoint(config, db, host, port)?;
        Self::open_connection_for_config(&db_config).await
    }

    async fn open_connection_for_config(
        config: &RedisConnectionConfig,
    ) -> Result<(Client, ConnectionManager), RedisError> {
        let client = Client::open(config.to_url().as_str()).map_err(|e| {
            RedisError::connection_with_source(
                t!("RedisConnection.create_client_failed").to_string(),
                e,
            )
        })?;

        let manager_config = Self::connection_manager_config(config);
        let conn = ConnectionManager::new_with_config(client.clone(), manager_config)
            .await
            .map_err(|e| {
                RedisError::connection_with_source(
                    t!("RedisConnection.connect_failed").to_string(),
                    e,
                )
            })?;

        Ok((client, conn))
    }

    fn connection_timeout_duration(config: &RedisConnectionConfig) -> Duration {
        let timeout = if config.timeout == 0 {
            DEFAULT_CONNECTION_TIMEOUT_SECONDS
        } else {
            config.timeout
        };
        Duration::from_secs(timeout)
    }

    fn connection_manager_config(config: &RedisConnectionConfig) -> ConnectionManagerConfig {
        let timeout = Self::connection_timeout_duration(config);
        ConnectionManagerConfig::new()
            .set_connection_timeout(timeout)
            .set_response_timeout(timeout)
            .set_number_of_retries(3)
            .set_max_delay(1_000)
    }

    #[cfg(test)]
    fn connection_config_for_db(
        config: &RedisConnectionConfig,
        db: u8,
    ) -> Result<RedisConnectionConfig, RedisError> {
        Self::connection_config_for_endpoint(config, db, &config.host, config.port)
    }

    fn connection_config_for_endpoint(
        config: &RedisConnectionConfig,
        db: u8,
        host: &str,
        port: u16,
    ) -> Result<RedisConnectionConfig, RedisError> {
        if config.mode == RedisConnectionMode::Cluster && db != 0 {
            return Err(RedisError::NotSupported(
                "Redis Cluster only supports database 0".to_string(),
            ));
        }

        let mut db_config = config.clone();
        db_config.db_index = db;
        db_config.host = host.to_string();
        db_config.port = port;
        Ok(db_config)
    }

    async fn resolve_connection_target(
        config: &RedisConnectionConfig,
    ) -> Result<ResolvedRedisConnectionTarget, RedisError> {
        let Some(tunnel_config) = config.ssh_tunnel.as_ref().filter(|tunnel| tunnel.enabled) else {
            return Ok(ResolvedRedisConnectionTarget {
                host: normalize_direct_host(&config.host),
                port: config.port,
                tunnel: None,
            });
        };

        let ssh_host = required_ssh_value(&tunnel_config.host, "ssh_host")?;
        let ssh_username = required_ssh_value(&tunnel_config.username, "ssh_username")?;
        let auth = build_ssh_auth(tunnel_config)?;
        let target_host = tunnel_config
            .target_host
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| config.host.clone());
        let target_port = tunnel_config.target_port.unwrap_or(config.port);
        let timeout_secs = tunnel_config.timeout.unwrap_or(DEFAULT_SSH_TIMEOUT_SECONDS);

        let ssh_config = SshConnectConfig {
            host: ssh_host,
            port: tunnel_config.port,
            username: ssh_username,
            auth,
            timeout: Some(Duration::from_secs(timeout_secs)),
            keepalive_interval: None,
            keepalive_max: None,
            jump_server: None,
            proxy: None,
            keyboard_interactive_responder: None,
            host_key_verifier: HostKeyVerifier::default(),
            x11_forwarding: false,
            allow_legacy_algorithms: false,
            forward_agent: false,
            agent_identities: Vec::new(),
        };

        let tunnel_result = timeout(
            Duration::from_secs(timeout_secs),
            start_local_port_forward(ssh_config, target_host, target_port),
        )
        .await;

        let tunnel = match tunnel_result {
            Ok(Ok(tunnel)) => tunnel,
            Ok(Err(error)) => {
                return Err(RedisError::connection(format!(
                    "failed to establish ssh tunnel: {error}"
                )));
            }
            Err(_) => {
                return Err(RedisError::connection(format!(
                    "ssh tunnel connection timed out after {timeout_secs}s"
                )));
            }
        };

        let local_addr = tunnel.local_addr();
        Ok(ResolvedRedisConnectionTarget {
            host: local_addr.ip().to_string(),
            port: local_addr.port(),
            tunnel: Some(tunnel),
        })
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

    fn is_reconnectable_redis_error(error: &redis_client::RedisError) -> bool {
        error.is_connection_dropped() || error.is_unrecoverable_error()
    }

    fn should_reconnect_after_redis_error(error: &RedisError) -> bool {
        match error {
            RedisError::Connection {
                source: Some(source),
                ..
            }
            | RedisError::Command {
                source: Some(source),
                ..
            } => source
                .downcast_ref::<redis_client::RedisError>()
                .is_some_and(Self::is_reconnectable_redis_error),
            _ => false,
        }
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

    /// 判断一条原始命令在连接掉线后是否可安全重放。
    ///
    /// 仅允许已知只读命令，避免重放 SET/INCR/EVAL 等有副作用的命令。
    /// `MEMORY` 只有 `USAGE` 子命令是只读的（`PURGE` 等会修改数据）。
    fn can_retry_raw_command(parts: &[String]) -> bool {
        let Some(command) = parts.first() else {
            return false;
        };
        if command.eq_ignore_ascii_case("MEMORY") {
            return parts
                .get(1)
                .is_some_and(|subcommand| subcommand.eq_ignore_ascii_case("USAGE"));
        }
        [
            "PING", "GET", "MGET", "EXISTS", "TYPE", "TTL", "PTTL", "SCAN", "SSCAN", "HSCAN",
            "ZSCAN", "KEYS", "STRLEN", "HLEN", "HGET", "HMGET", "HGETALL", "HKEYS", "HVALS",
            "LLEN", "LRANGE", "SCARD", "SMEMBERS", "ZCARD", "ZRANGE", "XRANGE", "XLEN", "INFO",
            "DBSIZE",
        ]
        .iter()
        .any(|read_only| command.eq_ignore_ascii_case(read_only))
    }

    /// 以已解析的 argv 直接在指定逻辑库执行命令。
    ///
    /// 供 Agent/CLI/MCP 工具等已持有 argv 的调用方复用，避免把参数重新拼回
    /// 命令行再解析，同时保持与 `execute_command_in_db` 相同的空命令与 SELECT 语义。
    pub async fn command_parts_in_db(
        &self,
        db: u8,
        parts: &[String],
    ) -> Result<RedisValue, RedisError> {
        if parts.is_empty() {
            return Err(RedisError::command(
                t!("RedisConnection.empty_command").to_string(),
            ));
        }
        Self::reject_select_command(parts)?;

        self.execute_parsed_command_in_db(db, parts).await
    }

    async fn execute_parsed_command_with_conn(
        conn: &mut ConnectionManager,
        parts: &[String],
    ) -> Result<RedisValue, RedisError> {
        let mut cmd = redis_client::cmd(parts[0].as_str());
        for arg in &parts[1..] {
            cmd.arg(arg.as_str());
        }

        let result: redis_client::Value = cmd.query_async(conn).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_execute_failed").to_string(),
                e,
            )
        })?;

        Ok(convert_redis_value(result))
    }

    async fn execute_parsed_command_in_db(
        &self,
        db: u8,
        parts: &[String],
    ) -> Result<RedisValue, RedisError> {
        let mut conn = self.get_db_conn(db).await?;
        match Self::execute_parsed_command_with_conn(&mut conn, parts).await {
            Ok(value) => Ok(value),
            Err(err) if Self::should_reconnect_after_redis_error(&err) => {
                // 无论命令是否可重放都重建连接，避免失效的管理器继续污染后续调用；
                // 但只有只读命令才会在重连后自动重放，写命令返回原始错误以免重复副作用。
                let replay_safe = Self::can_retry_raw_command(parts);
                let reconnect = self.reconnect_db_conn(db).await;
                if !replay_safe {
                    return Err(err);
                }
                let mut conn = reconnect?;
                Self::execute_parsed_command_with_conn(&mut conn, parts).await
            }
            Err(err) => Err(err),
        }
    }

    /// 从 MOVED 错误解析目标节点（host, port）。返回 None 表示非 MOVED 错误。
    fn moved_endpoint(error: &RedisError) -> Option<(String, u16)> {
        let source = std::error::Error::source(error)?;
        let redis_error = source.downcast_ref::<redis_client::RedisError>()?;
        if !matches!(redis_error.kind(), redis_client::ErrorKind::Moved) {
            return None;
        }
        // detail 形如 "13937 127.0.0.1:7003"
        let detail = redis_error.detail()?;
        let (_, addr) = detail.split_once(' ')?;
        let (host, port) = addr.rsplit_once(':')?;
        Some((host.to_string(), port.parse().ok()?))
    }

    /// 集群模式通用 MOVED 重试：先用默认 db 连接执行 `op`，遇 MOVED 时解析
    /// 目标节点，取/建该节点连接（按端点缓存）重试一次。非 MOVED 错误原样返回。
    /// 所有面向单键命令的 `*_in_db` 入口都应过这层，避免逐方法打补丁。
    async fn with_moved_retry<T, F>(&self, db: u8, mut op: F) -> Result<T, RedisError>
    where
        T: Send,
        F: for<'c> FnMut(
            &'c mut ConnectionManager,
        ) -> futures::future::BoxFuture<'c, Result<T, RedisError>>,
    {
        let mut retried_endpoint: Option<(String, u16)> = None;
        loop {
            let conn = match &retried_endpoint {
                None => self.get_db_conn(db).await?,
                Some(endpoint) => self.cluster_node_conn(endpoint).await?,
            };
            let mut conn = conn;
            match op(&mut conn).await {
                Ok(value) => return Ok(value),
                Err(err) => {
                    let Some(target) = Self::moved_endpoint(&err) else {
                        return Err(err);
                    };
                    if retried_endpoint.is_some() && Some(target.clone()) == retried_endpoint {
                        // 已重试到目标节点仍 MOVED：拓扑变动中，不再循环。
                        return Err(err);
                    }
                    retried_endpoint = Some(target);
                }
            }
        }
    }

    /// 取/建集群节点连接（按端点缓存，与 cluster_conns 同生命周期）。
    async fn cluster_node_conn(
        &self,
        endpoint: &(String, u16),
    ) -> Result<ConnectionManager, RedisError> {
        let mut cache = self.cluster_conns.lock().await;
        if let Some(conn) = cache.get(endpoint) {
            return Ok(conn.clone());
        }
        let (_, conn) =
            Self::open_connection_for_endpoint(&self.config, 0, &endpoint.0, endpoint.1).await?;
        cache.insert(endpoint.clone(), conn.clone());
        Ok(conn)
    }

    async fn key_type_with_conn(
        conn: &mut ConnectionManager,
        key: &str,
    ) -> Result<RedisKeyType, RedisError> {
        let key = key_codec::decode(key);
        let type_str: String = redis_client::cmd("TYPE")
            .arg(key.as_ref())
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

    async fn key_types_batch_with_conn(
        conn: &mut ConnectionManager,
        keys: &[String],
    ) -> RedisResult<Vec<String>> {
        let mut pipe = redis_client::pipe();
        for key in keys {
            pipe.cmd("TYPE").arg(key_codec::decode(key).as_ref());
        }

        pipe.query_async(conn).await
    }

    async fn ttl_with_conn(conn: &mut ConnectionManager, key: &str) -> Result<i64, RedisError> {
        redis_client::cmd("TTL")
            .arg(key_codec::decode(key).as_ref())
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
        conn: &mut ConnectionManager,
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
            .arg(key_codec::decode(key).as_ref())
            .query_async::<i64>(&mut *conn)
            .await
            .ok()
    }

    async fn key_info_with_conn(
        conn: &mut ConnectionManager,
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
        conn: &mut ConnectionManager,
        key: &str,
    ) -> Result<(Vec<Vec<u8>>, bool), RedisError> {
        let key = key_codec::decode(key);
        let mut cursor: u64 = 0;
        let mut members: Vec<Vec<u8>> = Vec::new();
        let mut budget = CollectionBudget::new();
        loop {
            let (next, batch): (u64, Vec<Vec<u8>>) = redis_client::cmd("SSCAN")
                .arg(key.as_ref())
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
            for member in batch {
                if !budget.accept(member.len()) {
                    return Ok((members, true));
                }
                members.push(member);
            }
            cursor = next;
            if cursor == 0 {
                return Ok((members, budget.truncated));
            }
        }
    }

    async fn scan_hash_fields(
        conn: &mut ConnectionManager,
        key: &str,
    ) -> Result<(Vec<HashField>, bool), RedisError> {
        let key = key_codec::decode(key);
        let mut cursor: u64 = 0;
        let mut fields: Vec<HashField> = Vec::new();
        let mut budget = CollectionBudget::new();
        loop {
            let (next, batch): (u64, Vec<(Vec<u8>, Vec<u8>)>) = redis_client::cmd("HSCAN")
                .arg(key.as_ref())
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
            for (field, value) in batch {
                let bytes = field.len().saturating_add(value.len());
                if !budget.accept(bytes) {
                    return Ok((fields, true));
                }
                fields.push(HashField { field, value });
            }
            cursor = next;
            if cursor == 0 {
                return Ok((fields, budget.truncated));
            }
        }
    }

    async fn zrange_with_scores_conn(
        conn: &mut ConnectionManager,
        key: &str,
    ) -> Result<(Vec<ZSetMember>, bool), RedisError> {
        let key = key_codec::decode(key);
        const BATCH: i64 = 200;
        let mut members: Vec<ZSetMember> = Vec::new();
        let mut budget = CollectionBudget::new();
        let mut start: i64 = 0;
        loop {
            let stop = start + BATCH - 1;
            let batch: Vec<(Vec<u8>, f64)> = conn
                .zrange_withscores(key.as_ref(), start as isize, stop as isize)
                .await
                .map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "ZRANGE").to_string(),
                        e,
                    )
                })?;
            if batch.is_empty() {
                return Ok((members, budget.truncated));
            }
            let batch_len = batch.len();
            for (member, score) in batch {
                if !budget.accept(member.len()) {
                    return Ok((members, true));
                }
                members.push(ZSetMember { member, score });
            }
            if batch_len < BATCH as usize {
                return Ok((members, budget.truncated));
            }
            start += BATCH;
        }
    }

    async fn lrange_capped(
        conn: &mut ConnectionManager,
        key: &str,
    ) -> Result<(Vec<Vec<u8>>, bool), RedisError> {
        let key = key_codec::decode(key);
        const BATCH: i64 = 200;
        let mut items: Vec<Vec<u8>> = Vec::new();
        let mut budget = CollectionBudget::new();
        let mut start: i64 = 0;
        loop {
            let stop = start + BATCH - 1;
            let batch: Vec<Vec<u8>> = conn
                .lrange(key.as_ref(), start as isize, stop as isize)
                .await
                .map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "LRANGE").to_string(),
                        e,
                    )
                })?;
            if batch.is_empty() {
                return Ok((items, budget.truncated));
            }
            let batch_len = batch.len();
            for item in batch {
                if !budget.accept(item.len()) {
                    return Ok((items, true));
                }
                items.push(item);
            }
            if batch_len < BATCH as usize {
                return Ok((items, budget.truncated));
            }
            start += BATCH;
        }
    }

    async fn xrange_conn(
        conn: &mut ConnectionManager,
        key: &str,
    ) -> Result<(Vec<StreamEntry>, bool), RedisError> {
        let result: Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)> = redis_client::cmd("XRANGE")
            .arg(key_codec::decode(key).as_ref())
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
        let mut entries: Vec<StreamEntry> = Vec::new();
        let mut budget = CollectionBudget::new();
        for (id, fields) in result {
            let bytes = fields.iter().fold(id.len(), |acc, (field, value)| {
                acc.saturating_add(field.len()).saturating_add(value.len())
            });
            if !budget.accept(bytes) {
                return Ok((entries, true));
            }
            entries.push(StreamEntry {
                id,
                fields: fields
                    .into_iter()
                    .map(|(field, value)| HashField { field, value })
                    .collect(),
            });
        }
        Ok((entries, budget.truncated))
    }

    /// 加载 String 值；超过字节预算时只取前 `MAX_COLLECTION_BYTES` 字节并标记截断。
    async fn load_string_value(
        conn: &mut ConnectionManager,
        key: &str,
        key_info: &KeyInfo,
    ) -> Result<(KeyValueContent, bool), RedisError> {
        let total = key_info.size.unwrap_or(0).max(0) as usize;
        let key = key_codec::decode(key);
        if total > MAX_COLLECTION_BYTES {
            let last = (MAX_COLLECTION_BYTES - 1) as isize;
            let value: Vec<u8> = conn.getrange(key.as_ref(), 0, last).await.map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "GETRANGE").to_string(),
                    e,
                )
            })?;
            return Ok((KeyValueContent::String(value), true));
        }

        let value: Option<Vec<u8>> = redis_client::cmd("GET")
            .arg(key.as_ref())
            .query_async(&mut *conn)
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "GET").to_string(),
                    e,
                )
            })?;
        Ok((string_content_from_bytes(value), false))
    }

    async fn key_value_detail_with_conn(
        conn: &mut ConnectionManager,
        key: &str,
    ) -> Result<KeyValueDetail, RedisError> {
        let key_info = Self::key_info_with_conn(conn, key).await?;
        let (value, truncated) = match key_info.key_type {
            RedisKeyType::String => Self::load_string_value(conn, key, &key_info).await?,
            RedisKeyType::List => {
                let (items, truncated) = Self::lrange_capped(conn, key).await?;
                (KeyValueContent::List(items), truncated)
            }
            RedisKeyType::Set => {
                let (items, truncated) = Self::scan_set_members(conn, key).await?;
                (KeyValueContent::Set(items), truncated)
            }
            RedisKeyType::ZSet => {
                let (items, truncated) = Self::zrange_with_scores_conn(conn, key).await?;
                (KeyValueContent::ZSet(items), truncated)
            }
            RedisKeyType::Hash => {
                let (fields, truncated) = Self::scan_hash_fields(conn, key).await?;
                (KeyValueContent::Hash(fields), truncated)
            }
            RedisKeyType::Stream => {
                let (entries, truncated) = Self::xrange_conn(conn, key).await?;
                (KeyValueContent::Stream(entries), truncated)
            }
            RedisKeyType::None => (KeyValueContent::None, false),
        };

        Ok(KeyValueDetail {
            key_info,
            value,
            truncated,
        })
    }
}

#[async_trait]
impl RedisConnection for RedisConnectionImpl {
    fn config(&self) -> &RedisConnectionConfig {
        &self.config
    }

    async fn connect(&mut self) -> Result<(), RedisError> {
        let target = Self::resolve_connection_target(&self.config).await?;
        let (client, conn) = Self::open_connection_for_endpoint(
            &self.config,
            self.config.db_index,
            &target.host,
            target.port,
        )
        .await?;
        self.client = Some(client);
        self.resolved_endpoint = Some((target.host, target.port));
        self.tunnel = target.tunnel;
        let mut connections = self.db_connections.write().await;
        connections.clear();
        connections.insert(self.config.db_index, conn);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), RedisError> {
        self.db_connections.write().await.clear();
        self.client = None;
        self.resolved_endpoint = None;
        self.tunnel = None;
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

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, RedisError> {
        let mut conn = self.get_conn().await?;
        let result: RedisResult<Option<Vec<u8>>> = conn.get(key_codec::decode(key).as_ref()).await;
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
        let key = key_codec::decode(key).into_owned();
        let value = value.to_string();
        self.with_moved_retry(db, move |conn| {
            let (key, value) = (key.clone(), value.clone());
            Box::pin(async move {
                if let Some(ttl) = ttl {
                    conn.set_ex(&key, &value, ttl as u64).await.map_err(|e| {
                        RedisError::command_with_source(
                            t!("RedisConnection.command_failed", command = "SETEX").to_string(),
                            e,
                        )
                    })
                } else {
                    conn.set(&key, &value).await.map_err(|e| {
                        RedisError::command_with_source(
                            t!("RedisConnection.command_failed", command = "SET").to_string(),
                            e,
                        )
                    })
                }
            })
        })
        .await
    }

    async fn del(&self, keys: &[&str]) -> Result<i64, RedisError> {
        self.del_in_db(self.config.db_index, keys).await
    }

    async fn del_in_db(&self, db: u8, keys: &[&str]) -> Result<i64, RedisError> {
        // 键按二进制安全表示还原成真实字节（见 key_codec），普通文本键恒等。
        let decoded_keys: Vec<Vec<u8>> = keys
            .iter()
            .map(|key| key_codec::decode(key).into_owned())
            .collect();
        let keys: Vec<&[u8]> = decoded_keys.iter().map(Vec::as_slice).collect();
        // 集群模式：DEL 须发到键所属 master；对不存在键 DEL 无副作用，
        // 逐 master 发整批累加即可。
        if self.cluster_fanout_enabled() && !keys.is_empty() {
            let masters = self.cluster_master_endpoints().await?;
            let mut deleted = 0i64;
            for (host, port) in masters {
                let (_, mut conn) =
                    Self::open_connection_for_endpoint(&self.config, db, &host, port).await?;
                let n: i64 = conn.del(&keys).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "DEL").to_string(),
                        e,
                    )
                })?;
                deleted += n;
            }
            return Ok(deleted);
        }

        let mut conn = self.get_db_conn(db).await?;
        conn.del(&keys).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "DEL").to_string(),
                e,
            )
        })
    }

    async fn exists(&self, key: &str) -> Result<bool, RedisError> {
        let mut conn = self.get_conn().await?;
        let count: i64 = conn
            .exists(key_codec::decode(key).as_ref())
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "EXISTS").to_string(),
                    e,
                )
            })?;
        Ok(count > 0)
    }

    async fn keys(&self, pattern: &str) -> Result<Vec<String>, RedisError> {
        let mut conn = self.get_conn().await?;
        // KEYS 同样可能返回二进制键，先按字节收下再编码成显示形态。
        let keys: Vec<Vec<u8>> = conn.keys(pattern).await.map_err(|e| {
            RedisError::command_with_source(
                t!("RedisConnection.command_failed", command = "KEYS").to_string(),
                e,
            )
        })?;
        Ok(keys.iter().map(|key| key_codec::encode(key)).collect())
    }

    async fn scan(
        &self,
        cursor: u64,
        pattern: &str,
        count: usize,
    ) -> Result<ScanResult, RedisError> {
        let mut conn = self.get_conn().await?;
        let reply: (u64, Vec<Vec<u8>>) = redis_client::cmd("SCAN")
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
        Ok(scan_result_from_reply(reply))
    }

    async fn scan_in_db(
        &self,
        db: u8,
        cursor: u64,
        pattern: &str,
        count: usize,
    ) -> Result<ScanResult, RedisError> {
        // 集群模式：SCAN 只会命中所连节点上的槽位，必须逐 master 扫描聚合。
        // 每个 master 的 cursor 独立，无法折进单个 u64，因此这里一次性扫完
        // 全部 master（每节点 cursor 到 0），finished=true 返回。
        // ponytail: 集群下「加载更多」退化为一次性全量；需要真分页时把返回结构
        // 升级为每节点 cursor 集，调用方同步改造。
        if self.cluster_fanout_enabled() && cursor == 0 {
            let masters = self.cluster_master_endpoints().await?;
            let mut all_keys: Vec<Vec<u8>> = Vec::new();
            for (host, port) in masters {
                let (_, mut conn) =
                    Self::open_connection_for_endpoint(&self.config, db, &host, port).await?;
                let mut node_cursor = 0u64;
                loop {
                    let (next, keys): (u64, Vec<Vec<u8>>) = redis_client::cmd("SCAN")
                        .arg(node_cursor)
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
                    all_keys.extend(keys);
                    node_cursor = next;
                    if node_cursor == 0 {
                        break;
                    }
                }
            }
            all_keys.sort();
            all_keys.dedup();
            return Ok(ScanResult::new(
                0,
                all_keys.iter().map(|key| key_codec::encode(key)).collect(),
            ));
        }

        let mut conn = self.get_db_conn(db).await?;

        let reply: (u64, Vec<Vec<u8>>) = redis_client::cmd("SCAN")
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

        Ok(scan_result_from_reply(reply))
    }

    async fn key_type(&self, key: &str) -> Result<RedisKeyType, RedisError> {
        let mut conn = self.get_conn().await?;
        match Self::key_type_with_conn(&mut conn, key).await {
            Ok(key_type) => Ok(key_type),
            Err(err) if Self::should_reconnect_after_redis_error(&err) => {
                let mut conn = self.reconnect_db_conn(self.config.db_index).await?;
                Self::key_type_with_conn(&mut conn, key).await
            }
            Err(err) => Err(err),
        }
    }

    async fn key_types_batch(
        &self,
        keys: &[String],
    ) -> Result<Vec<(String, RedisKeyType)>, RedisError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let mut conn = self.get_conn().await?;
        let results = match Self::key_types_batch_with_conn(&mut conn, keys).await {
            Ok(results) => results,
            Err(err) if Self::is_reconnectable_redis_error(&err) => {
                let mut conn = self.reconnect_db_conn(self.config.db_index).await?;
                Self::key_types_batch_with_conn(&mut conn, keys)
                    .await
                    .map_err(|e| {
                        RedisError::command_with_source(
                            t!("RedisConnection.command_failed", command = "TYPE (batch)")
                                .to_string(),
                            e,
                        )
                    })?
            }
            Err(err) => {
                return Err(RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "TYPE (batch)").to_string(),
                    err,
                ));
            }
        };

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

        // 集群模式：TYPE 必须发到键所属 master。pipeline 遇 MOVED 会整批失败，
        // 改为逐键 TYPE：先用缓存的节点连接试，MOVED 错误解析目标节点后
        // 建连重试一次。连接按端点缓存，同一节点重复命中不重建。
        // ponytail: 键量大时升级为按 MOVED 目标分组后整批发。
        if self.cluster_fanout_enabled() {
            let masters = self.cluster_master_endpoints().await?;
            let mut by_endpoint: std::collections::HashMap<(String, u16), ConnectionManager> =
                std::collections::HashMap::new();
            for (host, port) in &masters {
                let (_, conn) =
                    Self::open_connection_for_endpoint(&self.config, db, host, *port).await?;
                by_endpoint.insert((host.clone(), *port), conn);
            }
            let first = masters[0].clone();
            let mut result = Vec::with_capacity(keys.len());
            for key in keys {
                // 先试缓存的端点连接；MOVED 则解析目标节点建连后重试。
                let mut endpoint = first.clone();
                let key_type = loop {
                    let conn = match by_endpoint.get_mut(&endpoint) {
                        Some(conn) => conn,
                        None => {
                            let (_, conn) = Self::open_connection_for_endpoint(
                                &self.config,
                                db,
                                &endpoint.0,
                                endpoint.1,
                            )
                            .await?;
                            by_endpoint.insert(endpoint.clone(), conn);
                            by_endpoint.get_mut(&endpoint).expect("just inserted")
                        }
                    };
                    match Self::key_type_with_conn(conn, key).await {
                        Ok(key_type) => break key_type,
                        Err(e) => {
                            let Some(target) = Self::moved_endpoint(&e) else {
                                return Err(e);
                            };
                            if target == endpoint {
                                return Err(e);
                            }
                            endpoint = target;
                        }
                    }
                };
                result.push((key.clone(), key_type));
            }
            return Ok(result);
        }

        let mut conn = self.get_db_conn(db).await?;
        let results = match Self::key_types_batch_with_conn(&mut conn, keys).await {
            Ok(results) => results,
            Err(err) if Self::is_reconnectable_redis_error(&err) => {
                let mut conn = self.reconnect_db_conn(db).await?;
                Self::key_types_batch_with_conn(&mut conn, keys)
                    .await
                    .map_err(|e| {
                        RedisError::command_with_source(
                            t!("RedisConnection.command_failed", command = "TYPE (batch)")
                                .to_string(),
                            e,
                        )
                    })?
            }
            Err(err) => {
                return Err(RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "TYPE (batch)").to_string(),
                    err,
                ));
            }
        };

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
        let key = key_codec::decode(key).into_owned();
        self.with_moved_retry(db, move |conn| {
            let key = key.clone();
            Box::pin(async move {
                let result: i64 = conn.expire(&key, seconds).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "EXPIRE").to_string(),
                        e,
                    )
                })?;
                Ok(result == 1)
            })
        })
        .await
    }

    async fn persist(&self, key: &str) -> Result<bool, RedisError> {
        self.persist_in_db(self.config.db_index, key).await
    }

    async fn persist_in_db(&self, db: u8, key: &str) -> Result<bool, RedisError> {
        let key = key_codec::decode(key).into_owned();
        self.with_moved_retry(db, move |conn| {
            let key = key.clone();
            Box::pin(async move {
                let result: i64 = conn.persist(&key).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "PERSIST").to_string(),
                        e,
                    )
                })?;
                Ok(result == 1)
            })
        })
        .await
    }

    async fn rename(&self, old_key: &str, new_key: &str) -> Result<(), RedisError> {
        self.rename_in_db(self.config.db_index, old_key, new_key)
            .await
    }

    async fn rename_in_db(&self, db: u8, old_key: &str, new_key: &str) -> Result<(), RedisError> {
        // 旧键可能来自二进制键的转义显示形态；新键是用户在界面上输入的名字。
        let old_key = key_codec::decode(old_key).into_owned();
        let new_key = key_codec::decode(new_key).into_owned();
        self.with_moved_retry(db, move |conn| {
            let old_key = old_key.clone();
            let new_key = new_key.clone();
            Box::pin(async move {
                conn.rename(&old_key, &new_key).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "RENAME").to_string(),
                        e,
                    )
                })
            })
        })
        .await
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
        let key = key_codec::decode(key).into_owned();
        let field = field.to_string();
        let value = value.to_string();
        self.with_moved_retry(db, move |conn| {
            let (key, field, value) = (key.clone(), field.clone(), value.clone());
            Box::pin(async move {
                conn.hset(&key, &field, &value).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "HSET").to_string(),
                        e,
                    )
                })
            })
        })
        .await
    }

    async fn hdel(&self, key: &str, fields: &[&str]) -> Result<i64, RedisError> {
        self.hdel_in_db(self.config.db_index, key, fields).await
    }

    async fn hdel_in_db(&self, db: u8, key: &str, fields: &[&str]) -> Result<i64, RedisError> {
        let key = key_codec::decode(key).into_owned();
        let fields: Vec<String> = fields.iter().map(|s| s.to_string()).collect();
        self.with_moved_retry(db, move |conn| {
            let key = key.clone();
            let fields = fields.clone();
            Box::pin(async move {
                let refs: Vec<&str> = fields.iter().map(String::as_str).collect();
                conn.hdel(&key, refs.as_slice()).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "HDEL").to_string(),
                        e,
                    )
                })
            })
        })
        .await
    }

    async fn hlen(&self, key: &str) -> Result<i64, RedisError> {
        let mut conn = self.get_conn().await?;
        conn.hlen(key_codec::decode(key).as_ref())
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "HLEN").to_string(),
                    e,
                )
            })
    }

    async fn lrange(&self, key: &str, start: i64, stop: i64) -> Result<Vec<Vec<u8>>, RedisError> {
        let mut conn = self.get_conn().await?;
        conn.lrange(
            key_codec::decode(key).as_ref(),
            start as isize,
            stop as isize,
        )
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
        let key = key_codec::decode(key).into_owned();
        let values: Vec<String> = values.iter().map(|s| s.to_string()).collect();
        self.with_moved_retry(db, move |conn| {
            let key = key.clone();
            let values = values.clone();
            Box::pin(async move {
                let refs: Vec<&str> = values.iter().map(String::as_str).collect();
                conn.lpush(&key, refs.as_slice()).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "LPUSH").to_string(),
                        e,
                    )
                })
            })
        })
        .await
    }

    async fn rpush(&self, key: &str, values: &[&str]) -> Result<i64, RedisError> {
        self.rpush_in_db(self.config.db_index, key, values).await
    }

    async fn rpush_in_db(&self, db: u8, key: &str, values: &[&str]) -> Result<i64, RedisError> {
        let key = key_codec::decode(key).into_owned();
        let values: Vec<String> = values.iter().map(|s| s.to_string()).collect();
        self.with_moved_retry(db, move |conn| {
            let key = key.clone();
            let values = values.clone();
            Box::pin(async move {
                let refs: Vec<&str> = values.iter().map(String::as_str).collect();
                conn.rpush(&key, refs.as_slice()).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "RPUSH").to_string(),
                        e,
                    )
                })
            })
        })
        .await
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
        let key = key_codec::decode(key).into_owned();
        let value = value.to_string();
        self.with_moved_retry(db, move |conn| {
            let (key, value) = (key.clone(), value.clone());
            Box::pin(async move {
                conn.lset(&key, index as isize, &value).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "LSET").to_string(),
                        e,
                    )
                })
            })
        })
        .await
    }

    async fn llen(&self, key: &str) -> Result<i64, RedisError> {
        let mut conn = self.get_conn().await?;
        conn.llen(key_codec::decode(key).as_ref())
            .await
            .map_err(|e| {
                RedisError::command_with_source(
                    t!("RedisConnection.command_failed", command = "LLEN").to_string(),
                    e,
                )
            })
    }

    async fn sadd(&self, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        self.sadd_in_db(self.config.db_index, key, members).await
    }

    async fn sadd_in_db(&self, db: u8, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        let key = key_codec::decode(key).into_owned();
        let members: Vec<String> = members.iter().map(|s| s.to_string()).collect();
        self.with_moved_retry(db, move |conn| {
            let key = key.clone();
            let members = members.clone();
            Box::pin(async move {
                let refs: Vec<&str> = members.iter().map(String::as_str).collect();
                conn.sadd(&key, refs.as_slice()).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "SADD").to_string(),
                        e,
                    )
                })
            })
        })
        .await
    }

    async fn srem(&self, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        self.srem_in_db(self.config.db_index, key, members).await
    }

    async fn srem_in_db(&self, db: u8, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        let key = key_codec::decode(key).into_owned();
        let members: Vec<String> = members.iter().map(|s| s.to_string()).collect();
        self.with_moved_retry(db, move |conn| {
            let key = key.clone();
            let members = members.clone();
            Box::pin(async move {
                let refs: Vec<&str> = members.iter().map(String::as_str).collect();
                conn.srem(&key, refs.as_slice()).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "SREM").to_string(),
                        e,
                    )
                })
            })
        })
        .await
    }

    async fn scard(&self, key: &str) -> Result<i64, RedisError> {
        let mut conn = self.get_conn().await?;
        conn.scard(key_codec::decode(key).as_ref())
            .await
            .map_err(|e| {
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
        let result: Vec<(Vec<u8>, f64)> = conn
            .zrange_withscores(
                key_codec::decode(key).as_ref(),
                start as isize,
                stop as isize,
            )
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
        let key = key_codec::decode(key).into_owned();
        let members: Vec<(f64, String)> =
            members.iter().map(|(s, m)| (*s, m.to_string())).collect();
        self.with_moved_retry(db, move |conn| {
            let key = key.clone();
            let members = members.clone();
            Box::pin(async move {
                let items: Vec<(f64, &str)> =
                    members.iter().map(|(s, m)| (*s, m.as_str())).collect();
                conn.zadd_multiple(&key, &items).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "ZADD").to_string(),
                        e,
                    )
                })
            })
        })
        .await
    }

    async fn zrem(&self, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        self.zrem_in_db(self.config.db_index, key, members).await
    }

    async fn zrem_in_db(&self, db: u8, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        let key = key_codec::decode(key).into_owned();
        let members: Vec<String> = members.iter().map(|s| s.to_string()).collect();
        self.with_moved_retry(db, move |conn| {
            let key = key.clone();
            let members = members.clone();
            Box::pin(async move {
                let refs: Vec<&str> = members.iter().map(String::as_str).collect();
                conn.zrem(&key, refs.as_slice()).await.map_err(|e| {
                    RedisError::command_with_source(
                        t!("RedisConnection.command_failed", command = "ZREM").to_string(),
                        e,
                    )
                })
            })
        })
        .await
    }

    async fn zcard(&self, key: &str) -> Result<i64, RedisError> {
        let mut conn = self.get_conn().await?;
        conn.zcard(key_codec::decode(key).as_ref())
            .await
            .map_err(|e| {
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
        cmd.arg(key_codec::decode(key).as_ref()).arg(start).arg(end);
        if let Some(c) = count {
            cmd.arg("COUNT").arg(c);
        }
        let result: Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)> =
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
                fields: fields
                    .into_iter()
                    .map(|(field, value)| HashField { field, value })
                    .collect(),
            })
            .collect())
    }

    async fn xlen(&self, key: &str) -> Result<i64, RedisError> {
        let mut conn = self.get_conn().await?;
        redis_client::cmd("XLEN")
            .arg(key_codec::decode(key).as_ref())
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
        let parts = parse_command_args(command);
        self.command_parts_in_db(self.config.db_index, &parts).await
    }

    async fn execute_command_in_db(&self, db: u8, command: &str) -> Result<RedisValue, RedisError> {
        let parts = parse_command_args(command);
        self.command_parts_in_db(db, &parts).await
    }

    async fn get_key_info(&self, key: &str) -> Result<KeyInfo, RedisError> {
        let mut conn = self.get_conn().await?;
        match Self::key_info_with_conn(&mut conn, key).await {
            Ok(info) => Ok(info),
            Err(err) if Self::should_reconnect_after_redis_error(&err) => {
                let mut conn = self.reconnect_db_conn(self.config.db_index).await?;
                Self::key_info_with_conn(&mut conn, key).await
            }
            Err(err) => Err(err),
        }
    }

    async fn get_key_value_detail(&self, key: &str) -> Result<KeyValueDetail, RedisError> {
        let mut conn = self.get_conn().await?;
        match Self::key_value_detail_with_conn(&mut conn, key).await {
            Ok(detail) => Ok(detail),
            Err(err) if Self::should_reconnect_after_redis_error(&err) => {
                let mut conn = self.reconnect_db_conn(self.config.db_index).await?;
                Self::key_value_detail_with_conn(&mut conn, key).await
            }
            Err(err) => Err(err),
        }
    }

    async fn get_key_value_detail_in_db(
        &self,
        db: u8,
        key: &str,
    ) -> Result<KeyValueDetail, RedisError> {
        // 集群模式：单键命令会命中任意节点，MOVED 重试到所属 master。
        // 键保持显示形态，由 key_value_detail_with_conn 及其下游在命令边界还原字节。
        let key = key.to_string();
        self.with_moved_retry(db, move |conn| {
            let key = key.clone();
            Box::pin(async move { Self::key_value_detail_with_conn(conn, &key).await })
        })
        .await
    }

    async fn get_databases_info(&self) -> Result<Vec<RedisDatabaseInfo>, RedisError> {
        // 集群模式：键计数应汇总全部 master（集群仅 db0 有效）。
        if self.cluster_fanout_enabled() {
            let masters = self.cluster_master_endpoints().await?;
            let mut total = 0i64;
            let mut expires = 0i64;
            for (host, port) in masters {
                let (_, mut conn) =
                    Self::open_connection_for_endpoint(&self.config, 0, &host, port).await?;
                let info: String = redis_client::cmd("INFO")
                    .arg("keyspace")
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| {
                        RedisError::command_with_source(
                            t!("RedisConnection.command_failed", command = "INFO").to_string(),
                            e,
                        )
                    })?;
                for line in info.lines() {
                    if let Some(("db0", stats)) = line.split_once(':') {
                        for part in stats.split(',') {
                            if let Some((k, v)) = part.split_once('=') {
                                match k {
                                    "keys" => total += v.parse().unwrap_or(0),
                                    "expires" => expires += v.parse().unwrap_or(0),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            return Ok(vec![RedisDatabaseInfo {
                index: 0,
                keys: total,
                expires,
                avg_ttl: 0,
            }]);
        }

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

    async fn open_pubsub(&self) -> Result<RedisPubSubHandle, RedisError> {
        if self.client.is_none() {
            return Err(RedisError::NotConnected);
        }
        let (host, port) = self.current_endpoint();
        let config =
            Self::connection_config_for_endpoint(&self.config, self.config.db_index, host, port)?;
        redis_pubsub::start_pubsub_listener(config)
            .await
            .map_err(|e| RedisError::command(e.to_string()))
    }
}

#[doc(hidden)]
#[allow(dead_code)]
pub fn parse_command_args_for_test(command: &str) -> Vec<String> {
    parse_command_args(command)
}

#[cfg(test)]
pub(crate) mod tests {
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
            cluster_nodes: Vec::new(),
            ssh_tunnel: None,
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
    fn connection_config_for_endpoint_preserves_db_and_uses_resolved_tunnel_target() {
        let base = config(RedisConnectionMode::Standalone, 0);

        let db_config =
            RedisConnectionImpl::connection_config_for_endpoint(&base, 7, "127.0.0.1", 49152)
                .unwrap();

        assert_eq!(7, db_config.db_index);
        assert_eq!("127.0.0.1", db_config.host);
        assert_eq!(49152, db_config.port);
        assert_eq!("redis://127.0.0.1:49152/7", db_config.to_url());
    }

    #[test]
    fn tls_config_builds_rediss_url_supported_by_client() {
        let mut tls_config = config(RedisConnectionMode::Standalone, 0);
        tls_config.use_tls = true;

        let url = tls_config.to_url();
        assert_eq!("rediss://127.0.0.1:6379/0", url);

        // 故障面：redis 客户端未启用 TLS feature 时，`Client::open` 会直接返回
        // `InvalidClientConfig("can't connect with TLS, the feature is not enabled")`，
        // 表现为「无法创建 Redis 客户端」。
        Client::open(url.as_str()).expect("启用 use_tls 时 redis 客户端必须支持 rediss://");
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
    fn reconnect_check_detects_dropped_redis_connection() {
        let redis_error = redis_client::RedisError::from(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "broken pipe",
        ));
        let error = RedisError::command_with_source("TYPE failed", redis_error);

        assert!(RedisConnectionImpl::should_reconnect_after_redis_error(
            &error
        ));
    }

    #[test]
    fn reconnect_check_ignores_non_connection_command_errors() {
        let redis_error =
            redis_client::RedisError::from((redis_client::ErrorKind::ResponseError, "WRONGTYPE"));
        let error = RedisError::command_with_source("TYPE failed", redis_error);

        assert!(!RedisConnectionImpl::should_reconnect_after_redis_error(
            &error
        ));
    }

    #[test]
    fn connection_timeout_duration_uses_config_timeout_seconds() {
        let mut base = config(RedisConnectionMode::Standalone, 0);
        base.timeout = 3;

        assert_eq!(
            std::time::Duration::from_secs(3),
            RedisConnectionImpl::connection_timeout_duration(&base)
        );
    }

    #[test]
    fn connection_timeout_duration_falls_back_when_timeout_is_zero() {
        let mut base = config(RedisConnectionMode::Standalone, 0);
        base.timeout = 0;

        assert_eq!(
            std::time::Duration::from_secs(10),
            RedisConnectionImpl::connection_timeout_duration(&base)
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

    #[test]
    fn raw_command_retry_is_limited_to_readonly_commands() {
        assert!(RedisConnectionImpl::can_retry_raw_command(&[
            "get".to_string(),
            "key".to_string()
        ]));
        assert!(RedisConnectionImpl::can_retry_raw_command(&[
            "INFO".to_string()
        ]));
        assert!(!RedisConnectionImpl::can_retry_raw_command(&[
            "set".to_string(),
            "key".to_string(),
            "value".to_string()
        ]));
        assert!(!RedisConnectionImpl::can_retry_raw_command(&[
            "incr".to_string(),
            "counter".to_string()
        ]));
        assert!(!RedisConnectionImpl::can_retry_raw_command(&[]));
    }

    #[test]
    fn read_only_retry_allowlist_covers_extended_collection_commands() {
        for command in [
            "STRLEN", "HMGET", "HKEYS", "HVALS", "HGETALL", "SMEMBERS", "ZRANGE", "XRANGE",
        ] {
            assert!(
                RedisConnectionImpl::can_retry_raw_command(&[command.to_string()]),
                "{command} should be safe to replay"
            );
        }
        for command in ["SET", "DEL", "INCR", "EVAL", "MULTI", "SELECT"] {
            assert!(
                !RedisConnectionImpl::can_retry_raw_command(&[command.to_string()]),
                "{command} must not be replayed"
            );
        }
    }

    #[test]
    fn memory_retry_requires_the_read_only_usage_subcommand() {
        assert!(RedisConnectionImpl::can_retry_raw_command(&[
            "MEMORY".to_string(),
            "usage".to_string(),
            "key".to_string()
        ]));
        assert!(!RedisConnectionImpl::can_retry_raw_command(&[
            "MEMORY".to_string(),
            "PURGE".to_string()
        ]));
        assert!(!RedisConnectionImpl::can_retry_raw_command(&[
            "MEMORY".to_string()
        ]));
    }

    #[test]
    fn collection_budget_stops_at_element_limit() {
        let mut budget = CollectionBudget::new();
        for _ in 0..MAX_COLLECTION_ELEMENTS {
            assert!(budget.accept(1), "element within limit should be accepted");
        }
        assert!(
            !budget.accept(1),
            "element beyond the count limit must be rejected"
        );
        assert!(budget.truncated);
    }

    #[test]
    fn collection_budget_stops_at_byte_limit() {
        let mut budget = CollectionBudget::new();
        assert!(budget.accept(MAX_COLLECTION_BYTES));
        assert!(
            !budget.accept(1),
            "element beyond the byte limit must be rejected"
        );
        assert!(budget.truncated);
    }

    #[test]
    fn string_content_preserves_java_serialized_bytes() {
        let bytes = vec![0xac, 0xed, 0x00, 0x05, b's', b'r'];

        let content = string_content_from_bytes(Some(bytes.clone()));

        assert!(matches!(content, KeyValueContent::String(value) if value == bytes));
    }

    /// issue #485：SCAN 回复里可能混有非法 UTF-8 的二进制键。按 `Vec<String>` 解析会
    /// 整块失败（`Cannot convert from UTF-8`），因此必须先按字节收下再编码成显示形态。
    #[test]
    fn scan_reply_keeps_binary_keys_as_escaped_text() {
        let java_serialized = vec![0xac, 0xed, 0x00, 0x05, 0x74, 0x00, 0x03, 0x6b, 0x65, 0x79];
        let reply = redis_client::Value::Array(vec![
            redis_client::Value::BulkString(b"0".to_vec()),
            redis_client::Value::Array(vec![
                redis_client::Value::BulkString("普通键".as_bytes().to_vec()),
                redis_client::Value::BulkString(java_serialized.clone()),
            ]),
        ]);

        let (cursor, keys): (u64, Vec<Vec<u8>>) =
            redis_client::from_redis_value(&reply).expect("SCAN 回复应能按字节解析");
        let result = scan_result_from_reply((cursor, keys));

        assert_eq!(0, result.cursor);
        assert_eq!("普通键", result.keys[0]);
        assert_eq!(key_codec::encode(&java_serialized), result.keys[1]);
        assert_eq!(java_serialized, key_codec::decode(&result.keys[1]).as_ref());

        // 回归防线：旧实现用 Vec<String> 解析同一条回复会整块失败。
        assert!(
            redis_client::from_redis_value::<Vec<String>>(&reply).is_err(),
            "二进制键必须无法按 String 解析，否则该测试失去意义"
        );
    }

    /// issue #485 端到端：二进制键要能被 SCAN 列出、点开读到真实值、并被删除。
    /// 本机没有 Redis（127.0.0.1:6379）时跳过，与集群集成测试同策略。
    #[tokio::test]
    async fn binary_key_survives_scan_read_and_delete() {
        if tokio::net::TcpStream::connect("127.0.0.1:6379")
            .await
            .is_err()
        {
            return;
        }

        let java_serialized = vec![0xac, 0xed, 0x00, 0x05, 0x74, 0x00, 0x03, 0x6b, 0x65, 0x79];
        let mut connection = RedisConnectionImpl::new(config(RedisConnectionMode::Standalone, 0));
        connection.connect().await.expect("connect redis");
        {
            // 界面无法输入原始字节，直接下发原始命令写入二进制键。
            let mut conn = connection.get_db_conn(0).await.expect("db conn");
            redis_client::cmd("SET")
                .arg(java_serialized.as_slice())
                .arg("bin-value")
                .query_async::<()>(&mut conn)
                .await
                .expect("write binary key");
        }

        let escaped = key_codec::encode(&java_serialized);
        let scan = connection.scan_in_db(0, 0, "*", 1000).await.expect("scan");
        assert!(
            scan.keys.contains(&escaped),
            "SCAN 应把二进制键编码成转义显示形态: {:?}",
            scan.keys
        );

        let detail = connection
            .get_key_value_detail_in_db(0, &escaped)
            .await
            .expect("二进制键应能读回详情");
        assert_eq!(escaped, detail.key_info.name);
        assert!(
            matches!(detail.value, KeyValueContent::String(ref value) if value == b"bin-value"),
            "应读到二进制键的真实值: {:?}",
            detail.value
        );

        let deleted = connection
            .del_in_db(0, &[escaped.as_str()])
            .await
            .expect("delete binary key");
        assert_eq!(1, deleted);
        assert!(
            scan_after_delete_missing(&mut connection, &escaped).await,
            "删除后不应再扫到该键"
        );
    }

    async fn scan_after_delete_missing(
        connection: &mut RedisConnectionImpl,
        escaped: &str,
    ) -> bool {
        match connection.scan_in_db(0, 0, "*", 1000).await {
            Ok(scan) => !scan.keys.iter().any(|key| key == escaped),
            Err(_) => false,
        }
    }

    #[tokio::test]
    async fn command_parts_in_db_rejects_empty_parts_before_connecting() {
        let connection = RedisConnectionImpl::new(config(RedisConnectionMode::Standalone, 0));

        let error = connection
            .command_parts_in_db(0, &[])
            .await
            .expect_err("an empty argv must not reach the connection");

        assert!(matches!(error, RedisError::Command { .. }));
    }

    #[tokio::test]
    async fn command_parts_in_db_rejects_select_before_connecting() {
        let connection = RedisConnectionImpl::new(config(RedisConnectionMode::Standalone, 0));

        let error = connection
            .command_parts_in_db(0, &["SELECT".to_string(), "1".to_string()])
            .await
            .expect_err("SELECT must be rejected on multiplexed connections");

        assert!(matches!(error, RedisError::NotSupported(_)));
    }

    /// 集群集成验证（issue #414）：本机 3 master 集群（127.0.0.1:7001-7003，
    /// 各节点已有种子键）不存在时跳过。
    /// 验证 SCAN/TYPE 汇总全部 master，而不是只命中所连的那一个。
    #[tokio::test]
    async fn cluster_scan_aggregates_all_masters() {
        let reachable = tokio::net::TcpStream::connect("127.0.0.1:7001")
            .await
            .is_ok();
        if !reachable {
            return; // 本机无测试集群，跳过
        }

        let mut cfg = config(RedisConnectionMode::Cluster, 0);
        cfg.port = 7001;
        cfg.cluster_nodes = vec![
            "127.0.0.1:7001".to_string(),
            "127.0.0.1:7002".to_string(),
            "127.0.0.1:7003".to_string(),
        ];
        let mut conn = RedisConnectionImpl::new(cfg);
        conn.connect().await.expect("connect cluster");

        // 每个节点各自的键数（集群只有一个 db0）
        let mut per_node = Vec::new();
        for port in [7001u16, 7002, 7003] {
            let mut cfg = config(RedisConnectionMode::Standalone, 0);
            cfg.port = port;
            let mut node = RedisConnectionImpl::new(cfg);
            node.connect().await.expect("connect node");
            let dbs = node.get_databases_info().await.expect("dbsize");
            per_node.push(dbs.first().map(|d| d.keys).unwrap_or(0));
        }
        let total: i64 = per_node.iter().sum();
        assert!(total > 0, "集群应有种子键");

        // 集群聚合 SCAN 应拿到全部 master 的键
        let scan = conn.scan_in_db(0, 0, "*", 100).await.expect("cluster scan");
        assert!(scan.finished, "集群扫描一次扫完");
        assert_eq!(
            total as usize,
            scan.keys.len(),
            "SCAN 应汇总全部 master（各节点 {:?}）",
            per_node
        );

        // TYPE 批量也应跨 master 命中
        let sample: Vec<String> = scan.keys.iter().take(5).cloned().collect();
        let types = conn
            .key_types_batch_in_db(0, &sample)
            .await
            .expect("cluster type batch");
        assert_eq!(sample.len(), types.len());
        assert!(types.iter().all(|(_, t)| *t != RedisKeyType::None));

        // db 计数汇总
        let dbs = conn.get_databases_info().await.expect("cluster dbinfo");
        assert_eq!(total, dbs.first().map(|d| d.keys).unwrap_or(0));
    }

    /// 集群 MOVED 重试（issue #414 后续反馈）：单键命令打到非所属 master 时
    /// 应自动重定向。用 seed:moved 前缀（若存在）或动态写入键验证。
    #[tokio::test]
    async fn cluster_moved_retry_redirects_single_key_ops() {
        let reachable = tokio::net::TcpStream::connect("127.0.0.1:7001")
            .await
            .is_ok();
        if !reachable {
            return; // 本机无测试集群，跳过
        }

        let mut cfg = config(RedisConnectionMode::Cluster, 0);
        cfg.port = 7001;
        cfg.cluster_nodes = vec![
            "127.0.0.1:7001".to_string(),
            "127.0.0.1:7002".to_string(),
            "127.0.0.1:7003".to_string(),
        ];
        let mut conn = RedisConnectionImpl::new(cfg);
        conn.connect().await.expect("connect cluster");

        // 扫出全部键，找出至少一个不在 7001（所连入口）上的键。
        // 单节点视角：直接问 7001 有哪些键，再找出不在其中的。
        let scan = conn.scan_in_db(0, 0, "*", 500).await.expect("cluster scan");
        assert!(!scan.keys.is_empty(), "集群应有键");

        let mut node_cfg = config(RedisConnectionMode::Standalone, 0);
        node_cfg.port = 7001;
        let mut node1 = RedisConnectionImpl::new(node_cfg);
        node1.connect().await.expect("connect 7001");
        let own = node1.scan_in_db(0, 0, "*", 500).await.expect("node scan");
        let on_7001: std::collections::HashSet<String> = own.keys.into_iter().collect();

        // 选一个不在入口节点上的键，直连它必然 MOVED → 重试到正确 master。
        let foreign = scan
            .keys
            .iter()
            .find(|k| !on_7001.contains(*k))
            .expect("至少一个键不在入口节点（否则测试无意义，多写几个键再跑）");

        let detail = conn
            .get_key_value_detail_in_db(0, foreign)
            .await
            .expect("MOVED 重试后应拿到值详情");
        assert_eq!(detail.key_info.name, *foreign);

        // EXPIRE 同样应跨节点生效
        let expired = conn
            .expire_in_db(0, foreign, 3600)
            .await
            .expect("expire moved");
        assert!(expired, "EXPIRE 应在正确 master 上生效");
    }
}

/// SCAN 回复 → [`ScanResult`]。
///
/// 键先按字节收下、再编码成显示形态（见 [`key_codec`]）。Redis 的键是任意字节序列，
/// 若直接按 `Vec<String>` 解析，遇到 JDK 原生序列化等二进制键会整块失败
/// （`Cannot convert from UTF-8`，issue #485）。
fn scan_result_from_reply((cursor, keys): (u64, Vec<Vec<u8>>)) -> ScanResult {
    ScanResult::new(
        cursor,
        keys.iter().map(|key| key_codec::encode(key)).collect(),
    )
}

fn parse_command_args(command: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            // 在引号内识别常见控制字符转义,使其与 quote_command_arg 的转义反向一致。
            // 引号外的 \X 仍按原样保留为 X,沿用旧行为兼容已有用法。
            let decoded = if in_single || in_double {
                match ch {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                }
            } else {
                ch
            };
            current.push(decoded);
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
