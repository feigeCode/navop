//! Redis 连接实现

use crate::builtin_pubsub as redis_pubsub;
use crate::types::*;
use crate::{RedisConnection, RedisPubSubHandle};
use async_trait::async_trait;
use redis_client::aio::{ConnectionManager, ConnectionManagerConfig};
use redis_client::{AsyncCommands, Client, RedisResult};
use rust_i18n::t;
use ssh::{LocalPortForwardTunnel, SshAuth, SshConnectConfig, start_local_port_forward};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::timeout;

const MAX_COLLECTION_ELEMENTS: i64 = 1000;
const DEFAULT_CONNECTION_TIMEOUT_SECONDS: u64 = 10;
const DEFAULT_SSH_TIMEOUT_SECONDS: u64 = 30;

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
    tunnel: Option<LocalPortForwardTunnel>,
    resolved_endpoint: Option<(String, u16)>,
}

impl RedisConnectionImpl {
    pub fn new(config: RedisConnectionConfig) -> Self {
        Self {
            config,
            client: None,
            db_connections: Arc::new(RwLock::new(HashMap::new())),
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

    fn can_retry_raw_command(parts: &[String]) -> bool {
        parts.first().is_some_and(|command| {
            matches!(
                command.to_ascii_uppercase().as_str(),
                "PING"
                    | "GET"
                    | "MGET"
                    | "EXISTS"
                    | "TYPE"
                    | "TTL"
                    | "PTTL"
                    | "SCAN"
                    | "SSCAN"
                    | "HSCAN"
                    | "ZSCAN"
                    | "KEYS"
                    | "HLEN"
                    | "HGET"
                    | "HGETALL"
                    | "LLEN"
                    | "LRANGE"
                    | "SCARD"
                    | "SMEMBERS"
                    | "ZCARD"
                    | "ZRANGE"
                    | "XRANGE"
                    | "XLEN"
                    | "INFO"
                    | "DBSIZE"
            )
        })
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
            Err(err)
                if Self::can_retry_raw_command(parts)
                    && Self::should_reconnect_after_redis_error(&err) =>
            {
                let mut conn = self.reconnect_db_conn(db).await?;
                Self::execute_parsed_command_with_conn(&mut conn, parts).await
            }
            Err(err) => Err(err),
        }
    }

    async fn key_type_with_conn(
        conn: &mut ConnectionManager,
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

    async fn key_types_batch_with_conn(
        conn: &mut ConnectionManager,
        keys: &[String],
    ) -> RedisResult<Vec<String>> {
        let mut pipe = redis_client::pipe();
        for key in keys {
            pipe.cmd("TYPE").arg(key);
        }

        pipe.query_async(conn).await
    }

    async fn ttl_with_conn(conn: &mut ConnectionManager, key: &str) -> Result<i64, RedisError> {
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
            .arg(key)
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
        conn: &mut ConnectionManager,
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
        conn: &mut ConnectionManager,
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
        conn: &mut ConnectionManager,
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
        conn: &mut ConnectionManager,
        key: &str,
    ) -> Result<KeyValueDetail, RedisError> {
        let key_info = Self::key_info_with_conn(conn, key).await?;
        let value = match key_info.key_type {
            RedisKeyType::String => {
                let value = redis_client::cmd("GET")
                    .arg(key)
                    .query_async::<Option<Vec<u8>>>(&mut *conn)
                    .await
                    .map_err(|e| {
                        RedisError::command_with_source(
                            t!("RedisConnection.command_failed", command = "GET").to_string(),
                            e,
                        )
                    })?;
                string_content_from_bytes(value)
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
        let parts = parse_command_args(command);
        if parts.is_empty() {
            return Err(RedisError::command(
                t!("RedisConnection.empty_command").to_string(),
            ));
        }
        Self::reject_select_command(&parts)?;

        self.execute_parsed_command_in_db(self.config.db_index, &parts)
            .await
    }

    async fn execute_command_in_db(&self, db: u8, command: &str) -> Result<RedisValue, RedisError> {
        let parts = parse_command_args(command);
        if parts.is_empty() {
            return Err(RedisError::command(
                t!("RedisConnection.empty_command").to_string(),
            ));
        }
        Self::reject_select_command(&parts)?;

        self.execute_parsed_command_in_db(db, &parts).await
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
        let mut conn = self.get_db_conn(db).await?;
        match Self::key_value_detail_with_conn(&mut conn, key).await {
            Ok(detail) => Ok(detail),
            Err(err) if Self::should_reconnect_after_redis_error(&err) => {
                let mut conn = self.reconnect_db_conn(db).await?;
                Self::key_value_detail_with_conn(&mut conn, key).await
            }
            Err(err) => Err(err),
        }
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
    fn string_content_preserves_java_serialized_bytes() {
        let bytes = vec![0xac, 0xed, 0x00, 0x05, b's', b'r'];

        let content = string_content_from_bytes(Some(bytes.clone()));

        assert!(matches!(content, KeyValueContent::String(value) if value == bytes));
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
