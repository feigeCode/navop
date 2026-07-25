use std::sync::Arc;

use base64::Engine as _;
use connection_tunnel::{TunnelGuard, resolve_connection_target};
use extension_host::{HostError, NativeDriverManifest, ProcessRpcSession};
use extension_protocol::blob::WireBytes;
use extension_protocol::conn::{ConnOpenParams, ConnOpenResult};
use extension_protocol::event_stream::{EventOpenResult, EventReadResult};
use extension_protocol::method;
use extension_protocol::redis::{
    RedisCommandParams, RedisCommandResult, RedisConnectionConfig as WireRedisConnectionConfig,
    RedisPipelineParams, RedisPipelineResult, RedisRespValue,
};

use async_trait::async_trait;

use crate::{
    HashField, KeyInfo, KeyValueContent, KeyValueDetail, PubSubMessage, PubSubMessageKind,
    RedisConnection, RedisConnectionConfig, RedisDatabaseInfo, RedisError, RedisKeyType,
    RedisPubSubHandle, RedisServerInfo, RedisValue, ScanResult, StreamEntry, SubscriptionCommand,
    ZSetMember,
};

/// Thin Redis adapter over the generic native process RPC session.
pub struct IpcRedisConnection {
    config: RedisConnectionConfig,
    session: Arc<ProcessRpcSession>,
    conn_id: u64,
    driver_context: String,
    tunnel: Option<TunnelGuard>,
}

impl IpcRedisConnection {
    pub async fn start(
        manifest: &NativeDriverManifest,
        config: RedisConnectionConfig,
    ) -> Result<Self, RedisError> {
        let driver_context = native_driver_context(manifest);
        let target =
            resolve_connection_target(&config.host, config.port, config.ssh_tunnel.as_ref())
                .await
                .map_err(|error| RedisError::connection(error.to_string()))?;
        let session_config = manifest
            .process_session_config(env!("CARGO_PKG_VERSION"), uuid::Uuid::new_v4().to_string());
        let session = Arc::new(
            ProcessRpcSession::start(session_config)
                .await
                .map_err(|error| host_connection_error(error, &driver_context))?,
        );
        let wire_config = wire_config_for_target(&config, &target.host, target.port);
        let open = ConnOpenParams::new(
            manifest.id.clone(),
            serde_json::to_value(wire_config).map_err(serialization_error)?,
        );
        let result: ConnOpenResult = session
            .request(
                method::CONN_OPEN,
                serde_json::to_value(open).map_err(serialization_error)?,
            )
            .await
            .map_err(|error| host_connection_error(error, &driver_context))?;
        Ok(Self {
            config,
            session,
            conn_id: result.conn_id,
            driver_context,
            tunnel: target.tunnel,
        })
    }

    pub fn config(&self) -> &RedisConnectionConfig {
        &self.config
    }

    pub fn conn_id(&self) -> u64 {
        self.conn_id
    }

    pub async fn command_bytes(
        &self,
        database: Option<u8>,
        args: Vec<Vec<u8>>,
    ) -> Result<RedisValue, RedisError> {
        let params = RedisCommandParams {
            conn_id: self.conn_id,
            database,
            args: args.into_iter().map(wire_bytes).collect(),
        };
        let result: RedisCommandResult = self
            .session
            .request(
                method::REDIS_COMMAND,
                serde_json::to_value(params).map_err(serialization_error)?,
            )
            .await
            .map_err(|error| redis_command_host_error(error, &self.driver_context))?;
        Ok(domain_value(result.value))
    }

    pub async fn pipeline_bytes(
        &self,
        database: Option<u8>,
        commands: Vec<Vec<Vec<u8>>>,
    ) -> Result<Vec<RedisValue>, RedisError> {
        let params = RedisPipelineParams {
            conn_id: self.conn_id,
            database,
            commands: commands
                .into_iter()
                .map(|args| args.into_iter().map(wire_bytes).collect())
                .collect(),
        };
        params
            .validate()
            .map_err(|message| RedisError::command(message.to_string()))?;
        let result: RedisPipelineResult = self
            .session
            .request(
                method::REDIS_PIPELINE,
                serde_json::to_value(params).map_err(serialization_error)?,
            )
            .await
            .map_err(|error| redis_command_host_error(error, &self.driver_context))?;
        Ok(result.values.into_iter().map(domain_value).collect())
    }

    pub async fn shutdown(&self) {
        let _ = self
            .session
            .request_value(
                method::CONN_CLOSE,
                serde_json::json!({ "conn_id": self.conn_id }),
            )
            .await;
        self.session.shutdown().await;
    }
}

#[async_trait]
impl RedisConnection for IpcRedisConnection {
    fn config(&self) -> &RedisConnectionConfig {
        &self.config
    }

    async fn connect(&mut self) -> Result<(), RedisError> {
        if self.session.is_closed() {
            return Err(RedisError::NotConnected);
        }
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), RedisError> {
        self.shutdown().await;
        self.tunnel = None;
        Ok(())
    }

    async fn ping(&self) -> Result<(), RedisError> {
        self.command(None, &["PING"]).await?;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        !self.session.is_closed()
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, RedisError> {
        match self.command(None, &["GET", key]).await? {
            RedisValue::Nil => Ok(None),
            value => Ok(Some(value_bytes(value)?)),
        }
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
        let mut args = vec!["SET".to_string(), key.to_string(), value.to_string()];
        if let Some(ttl) = ttl {
            args.extend(["EX".to_string(), ttl.to_string()]);
        }
        self.command_owned(Some(db), args).await?;
        Ok(())
    }

    async fn del(&self, keys: &[&str]) -> Result<i64, RedisError> {
        self.del_in_db(self.config.db_index, keys).await
    }

    async fn del_in_db(&self, db: u8, keys: &[&str]) -> Result<i64, RedisError> {
        let mut args = vec!["DEL".to_string()];
        args.extend(keys.iter().map(|key| (*key).to_string()));
        value_integer(self.command_owned(Some(db), args).await?)
    }

    async fn exists(&self, key: &str) -> Result<bool, RedisError> {
        Ok(value_integer(self.command(None, &["EXISTS", key]).await?)? > 0)
    }

    async fn keys(&self, pattern: &str) -> Result<Vec<String>, RedisError> {
        value_strings(self.command(None, &["KEYS", pattern]).await?)
    }

    async fn scan(
        &self,
        cursor: u64,
        pattern: &str,
        count: usize,
    ) -> Result<ScanResult, RedisError> {
        self.scan_in_db(self.config.db_index, cursor, pattern, count)
            .await
    }

    async fn scan_in_db(
        &self,
        db: u8,
        cursor: u64,
        pattern: &str,
        count: usize,
    ) -> Result<ScanResult, RedisError> {
        let value = self
            .command_owned(
                Some(db),
                vec![
                    "SCAN".into(),
                    cursor.to_string(),
                    "MATCH".into(),
                    pattern.into(),
                    "COUNT".into(),
                    count.to_string(),
                ],
            )
            .await?;
        parse_scan(value)
    }

    async fn key_type(&self, key: &str) -> Result<RedisKeyType, RedisError> {
        Ok(parse_key_type(&value_string(
            self.command(None, &["TYPE", key]).await?,
        )?))
    }

    async fn key_types_batch(
        &self,
        keys: &[String],
    ) -> Result<Vec<(String, RedisKeyType)>, RedisError> {
        self.key_types_batch_in_db(self.config.db_index, keys).await
    }

    async fn key_types_batch_in_db(
        &self,
        db: u8,
        keys: &[String],
    ) -> Result<Vec<(String, RedisKeyType)>, RedisError> {
        let commands = keys
            .iter()
            .map(|key| vec![b"TYPE".to_vec(), key.as_bytes().to_vec()])
            .collect();
        let values = self.pipeline_bytes(Some(db), commands).await?;
        keys.iter()
            .cloned()
            .zip(values)
            .map(|(key, value)| Ok((key, parse_key_type(&value_string(value)?))))
            .collect()
    }

    async fn ttl(&self, key: &str) -> Result<i64, RedisError> {
        value_integer(self.command(None, &["TTL", key]).await?)
    }

    async fn expire(&self, key: &str, seconds: i64) -> Result<bool, RedisError> {
        self.expire_in_db(self.config.db_index, key, seconds).await
    }

    async fn expire_in_db(&self, db: u8, key: &str, seconds: i64) -> Result<bool, RedisError> {
        Ok(value_integer(
            self.command_owned(
                Some(db),
                vec!["EXPIRE".into(), key.into(), seconds.to_string()],
            )
            .await?,
        )? > 0)
    }

    async fn persist(&self, key: &str) -> Result<bool, RedisError> {
        self.persist_in_db(self.config.db_index, key).await
    }

    async fn persist_in_db(&self, db: u8, key: &str) -> Result<bool, RedisError> {
        Ok(value_integer(self.command(Some(db), &["PERSIST", key]).await?)? > 0)
    }

    async fn rename(&self, old_key: &str, new_key: &str) -> Result<(), RedisError> {
        self.rename_in_db(self.config.db_index, old_key, new_key)
            .await
    }

    async fn rename_in_db(&self, db: u8, old_key: &str, new_key: &str) -> Result<(), RedisError> {
        self.command(Some(db), &["RENAME", old_key, new_key])
            .await?;
        Ok(())
    }

    async fn hgetall(&self, key: &str) -> Result<Vec<HashField>, RedisError> {
        parse_hash_fields(self.command(None, &["HGETALL", key]).await?)
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
        self.command(Some(db), &["HSET", key, field, value]).await?;
        Ok(())
    }

    async fn hdel(&self, key: &str, fields: &[&str]) -> Result<i64, RedisError> {
        self.hdel_in_db(self.config.db_index, key, fields).await
    }

    async fn hdel_in_db(&self, db: u8, key: &str, fields: &[&str]) -> Result<i64, RedisError> {
        let mut args = vec!["HDEL".to_string(), key.to_string()];
        args.extend(fields.iter().map(|field| (*field).to_string()));
        value_integer(self.command_owned(Some(db), args).await?)
    }

    async fn hlen(&self, key: &str) -> Result<i64, RedisError> {
        value_integer(self.command(None, &["HLEN", key]).await?)
    }

    async fn lrange(&self, key: &str, start: i64, stop: i64) -> Result<Vec<Vec<u8>>, RedisError> {
        value_byte_strings(
            self.command_owned(
                None,
                vec![
                    "LRANGE".into(),
                    key.into(),
                    start.to_string(),
                    stop.to_string(),
                ],
            )
            .await?,
        )
    }

    async fn lpush(&self, key: &str, values: &[&str]) -> Result<i64, RedisError> {
        self.lpush_in_db(self.config.db_index, key, values).await
    }

    async fn lpush_in_db(&self, db: u8, key: &str, values: &[&str]) -> Result<i64, RedisError> {
        list_push(self, Some(db), "LPUSH", key, values).await
    }

    async fn rpush(&self, key: &str, values: &[&str]) -> Result<i64, RedisError> {
        self.rpush_in_db(self.config.db_index, key, values).await
    }

    async fn rpush_in_db(&self, db: u8, key: &str, values: &[&str]) -> Result<i64, RedisError> {
        list_push(self, Some(db), "RPUSH", key, values).await
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
        self.command_owned(
            Some(db),
            vec!["LSET".into(), key.into(), index.to_string(), value.into()],
        )
        .await?;
        Ok(())
    }

    async fn llen(&self, key: &str) -> Result<i64, RedisError> {
        value_integer(self.command(None, &["LLEN", key]).await?)
    }

    async fn smembers(&self, key: &str) -> Result<Vec<Vec<u8>>, RedisError> {
        value_byte_strings(self.command(None, &["SMEMBERS", key]).await?)
    }

    async fn sadd(&self, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        self.sadd_in_db(self.config.db_index, key, members).await
    }

    async fn sadd_in_db(&self, db: u8, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        collection_update(self, Some(db), "SADD", key, members).await
    }

    async fn srem(&self, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        self.srem_in_db(self.config.db_index, key, members).await
    }

    async fn srem_in_db(&self, db: u8, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        collection_update(self, Some(db), "SREM", key, members).await
    }

    async fn scard(&self, key: &str) -> Result<i64, RedisError> {
        value_integer(self.command(None, &["SCARD", key]).await?)
    }

    async fn zrange_with_scores(
        &self,
        key: &str,
        start: i64,
        stop: i64,
    ) -> Result<Vec<ZSetMember>, RedisError> {
        parse_zset(
            self.command_owned(
                None,
                vec![
                    "ZRANGE".into(),
                    key.into(),
                    start.to_string(),
                    stop.to_string(),
                    "WITHSCORES".into(),
                ],
            )
            .await?,
        )
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
        let mut args = vec!["ZADD".to_string(), key.to_string()];
        for (score, member) in members {
            args.extend([score.to_string(), (*member).to_string()]);
        }
        value_integer(self.command_owned(Some(db), args).await?)
    }

    async fn zrem(&self, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        self.zrem_in_db(self.config.db_index, key, members).await
    }

    async fn zrem_in_db(&self, db: u8, key: &str, members: &[&str]) -> Result<i64, RedisError> {
        collection_update(self, Some(db), "ZREM", key, members).await
    }

    async fn zcard(&self, key: &str) -> Result<i64, RedisError> {
        value_integer(self.command(None, &["ZCARD", key]).await?)
    }

    async fn xrange(
        &self,
        key: &str,
        start: &str,
        end: &str,
        count: Option<usize>,
    ) -> Result<Vec<StreamEntry>, RedisError> {
        let mut args = vec!["XRANGE".into(), key.into(), start.into(), end.into()];
        if let Some(count) = count {
            args.extend(["COUNT".into(), count.to_string()]);
        }
        parse_stream_entries(self.command_owned(None, args).await?)
    }

    async fn xlen(&self, key: &str) -> Result<i64, RedisError> {
        value_integer(self.command(None, &["XLEN", key]).await?)
    }

    async fn info(&self, section: Option<&str>) -> Result<String, RedisError> {
        let value = match section {
            Some(section) => self.command(None, &["INFO", section]).await?,
            None => self.command(None, &["INFO"]).await?,
        };
        value_string(value)
    }

    async fn dbsize(&self) -> Result<i64, RedisError> {
        value_integer(self.command(None, &["DBSIZE"]).await?)
    }

    async fn select(&self, db: u8) -> Result<(), RedisError> {
        if db == self.config.db_index {
            Ok(())
        } else {
            Err(RedisError::NotSupported(
                "IPC connections select a database per request; use the *_in_db APIs".into(),
            ))
        }
    }

    async fn flushdb(&self) -> Result<(), RedisError> {
        self.command(None, &["FLUSHDB"]).await?;
        Ok(())
    }

    async fn execute_command(&self, command: &str) -> Result<RedisValue, RedisError> {
        self.execute_command_in_db(self.config.db_index, command)
            .await
    }

    async fn execute_command_in_db(&self, db: u8, command: &str) -> Result<RedisValue, RedisError> {
        let args = shell_words::split(command)
            .map_err(|error| RedisError::command(format!("invalid command: {error}")))?;
        if args.is_empty() {
            return Err(RedisError::command("command cannot be empty"));
        }
        self.command_owned(Some(db), args).await
    }

    async fn get_key_info(&self, key: &str) -> Result<KeyInfo, RedisError> {
        self.key_info_in_db(self.config.db_index, key).await
    }

    async fn get_key_value_detail(&self, key: &str) -> Result<KeyValueDetail, RedisError> {
        self.get_key_value_detail_in_db(self.config.db_index, key)
            .await
    }

    async fn get_key_value_detail_in_db(
        &self,
        db: u8,
        key: &str,
    ) -> Result<KeyValueDetail, RedisError> {
        let key_info = self.key_info_in_db(db, key).await?;
        let value = match key_info.key_type {
            RedisKeyType::String => match self.command(Some(db), &["GET", key]).await? {
                RedisValue::Nil => KeyValueContent::None,
                RedisValue::Binary(value) => KeyValueContent::String(value),
                value => KeyValueContent::String(value_string(value)?.into_bytes()),
            },
            RedisKeyType::List => KeyValueContent::List(value_byte_strings(
                self.command(Some(db), &["LRANGE", key, "0", "999"]).await?,
            )?),
            RedisKeyType::Set => KeyValueContent::Set(value_byte_strings(
                self.command(Some(db), &["SMEMBERS", key]).await?,
            )?),
            RedisKeyType::ZSet => KeyValueContent::ZSet(parse_zset(
                self.command(Some(db), &["ZRANGE", key, "0", "999", "WITHSCORES"])
                    .await?,
            )?),
            RedisKeyType::Hash => KeyValueContent::Hash(parse_hash_fields(
                self.command(Some(db), &["HGETALL", key]).await?,
            )?),
            RedisKeyType::Stream => KeyValueContent::Stream(parse_stream_entries(
                self.command(Some(db), &["XRANGE", key, "-", "+", "COUNT", "1000"])
                    .await?,
            )?),
            RedisKeyType::None => KeyValueContent::None,
        };
        Ok(KeyValueDetail { key_info, value })
    }

    async fn get_databases_info(&self) -> Result<Vec<RedisDatabaseInfo>, RedisError> {
        let values = parse_info(&self.info(Some("keyspace")).await?);
        let mut databases = values
            .into_iter()
            .filter_map(|(key, value)| {
                let index = key.strip_prefix("db")?.parse::<u8>().ok()?;
                let fields = value
                    .split(',')
                    .filter_map(|item| item.split_once('='))
                    .collect::<std::collections::HashMap<_, _>>();
                Some(RedisDatabaseInfo {
                    index,
                    keys: fields.get("keys").and_then(|v| v.parse().ok()).unwrap_or(0),
                    expires: fields
                        .get("expires")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0),
                    avg_ttl: fields
                        .get("avg_ttl")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0),
                })
            })
            .collect::<Vec<_>>();
        databases.sort_by_key(|database| database.index);
        Ok(databases)
    }

    async fn get_server_info(&self) -> Result<RedisServerInfo, RedisError> {
        let values = parse_info(&self.info(None).await?);
        Ok(RedisServerInfo {
            version: values.get("redis_version").cloned().unwrap_or_default(),
            mode: values.get("redis_mode").cloned().unwrap_or_default(),
            os: values.get("os").cloned().unwrap_or_default(),
            connected_clients: parse_info_i64(&values, "connected_clients"),
            used_memory: parse_info_i64(&values, "used_memory"),
            used_memory_human: values.get("used_memory_human").cloned().unwrap_or_default(),
            total_keys: self
                .get_databases_info()
                .await?
                .into_iter()
                .map(|database| database.keys)
                .sum(),
            uptime_in_seconds: parse_info_i64(&values, "uptime_in_seconds"),
        })
    }

    async fn open_pubsub(&self) -> Result<RedisPubSubHandle, RedisError> {
        let open: EventOpenResult = self
            .session
            .request(
                extension_protocol::method::EVENT_OPEN,
                serde_json::json!({
                    "conn_id": self.conn_id,
                    "kind": "redis_pubsub",
                    "capacity": 128
                }),
            )
            .await
            .map_err(|error| host_connection_error(error, &self.driver_context))?;
        let (command_tx, mut command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (message_tx, message_rx) = tokio::sync::mpsc::unbounded_channel();
        let session = Arc::clone(&self.session);
        let conn_id = self.conn_id;
        let stream_id = open.stream_id;
        tokio::spawn(async move {
            let mut closed = false;
            while !closed {
                tokio::select! {
                    command = command_rx.recv() => {
                        let Some(command) = command else { break; };
                        if matches!(command, SubscriptionCommand::Stop) { break; }
                        let control = match command {
                            SubscriptionCommand::Subscribe(value) => extension_protocol::redis::RedisPubSubControl::Subscribe(WireBytes::Utf8(value)),
                            SubscriptionCommand::PSubscribe(value) => extension_protocol::redis::RedisPubSubControl::PSubscribe(WireBytes::Utf8(value)),
                            SubscriptionCommand::Unsubscribe(value) => extension_protocol::redis::RedisPubSubControl::Unsubscribe(WireBytes::Utf8(value)),
                            SubscriptionCommand::PUnsubscribe(value) => extension_protocol::redis::RedisPubSubControl::PUnsubscribe(WireBytes::Utf8(value)),
                            SubscriptionCommand::Stop => unreachable!(),
                        };
                        if session.request_value(extension_protocol::method::REDIS_PUBSUB_CONTROL, serde_json::json!({
                            "conn_id": conn_id, "stream_id": stream_id, "control": control,
                        })).await.is_err() { break; }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                        let result: Result<EventReadResult, _> = session.request(
                            extension_protocol::method::EVENT_READ,
                            serde_json::json!({"stream_id": stream_id, "max_events": 64, "wait_ms": 50}),
                        ).await;
                        let Ok(result) = result else { break; };
                        closed = result.closed;
                        for value in result.events {
                            if let Some(message) = decode_pubsub_message(value) {
                                if message_tx.send(message).is_err() { break; }
                            }
                        }
                    }
                }
            }
            let _ = session
                .request_value(
                    extension_protocol::method::EVENT_CLOSE,
                    serde_json::json!({"stream_id": stream_id, "conn_id": conn_id}),
                )
                .await;
        });
        Ok(RedisPubSubHandle::new(command_tx, message_rx))
    }
}

fn decode_pubsub_message(value: serde_json::Value) -> Option<PubSubMessage> {
    let kind = match value.get("kind").and_then(serde_json::Value::as_str) {
        Some("pmessage") => PubSubMessageKind::PMessage,
        Some("smessage") => PubSubMessageKind::SMessage,
        Some("message") => PubSubMessageKind::Message,
        _ => return None,
    };
    let payload = value.get("payload")?;
    let payload = if payload.get("encoding")?.as_str()? == "base64" {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload.get("value")?.as_str()?)
            .ok()?;
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        payload.get("value")?.as_str()?.to_string()
    };
    Some(PubSubMessage {
        kind,
        channel: value.get("channel")?.as_str()?.to_string(),
        pattern: value
            .get("pattern")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        payload,
        received_at: chrono::Local::now(),
    })
}

impl IpcRedisConnection {
    async fn command(&self, database: Option<u8>, args: &[&str]) -> Result<RedisValue, RedisError> {
        self.command_bytes(
            database,
            args.iter().map(|arg| arg.as_bytes().to_vec()).collect(),
        )
        .await
    }

    async fn command_owned(
        &self,
        database: Option<u8>,
        args: Vec<String>,
    ) -> Result<RedisValue, RedisError> {
        self.command_bytes(database, args.into_iter().map(String::into_bytes).collect())
            .await
    }

    async fn key_info_in_db(&self, db: u8, key: &str) -> Result<KeyInfo, RedisError> {
        let values = self
            .pipeline_bytes(
                Some(db),
                vec![
                    vec![b"TYPE".to_vec(), key.as_bytes().to_vec()],
                    vec![b"TTL".to_vec(), key.as_bytes().to_vec()],
                ],
            )
            .await?;
        let key_type = parse_key_type(&value_string(values[0].clone())?);
        let ttl = value_integer(values[1].clone())?;
        let memory_usage =
            optional_integer(self.command(Some(db), &["MEMORY", "USAGE", key]).await);
        let size = match key_type {
            RedisKeyType::String => Some(value_integer(
                self.command(Some(db), &["STRLEN", key]).await?,
            )?),
            RedisKeyType::List => Some(value_integer(
                self.command(Some(db), &["LLEN", key]).await?,
            )?),
            RedisKeyType::Set => Some(value_integer(
                self.command(Some(db), &["SCARD", key]).await?,
            )?),
            RedisKeyType::ZSet => Some(value_integer(
                self.command(Some(db), &["ZCARD", key]).await?,
            )?),
            RedisKeyType::Hash => Some(value_integer(
                self.command(Some(db), &["HLEN", key]).await?,
            )?),
            RedisKeyType::Stream => Some(value_integer(
                self.command(Some(db), &["XLEN", key]).await?,
            )?),
            RedisKeyType::None => None,
        };
        Ok(KeyInfo {
            name: key.to_string(),
            key_type,
            ttl,
            size,
            memory_usage,
        })
    }
}

async fn list_push(
    connection: &IpcRedisConnection,
    database: Option<u8>,
    command: &str,
    key: &str,
    values: &[&str],
) -> Result<i64, RedisError> {
    collection_update(connection, database, command, key, values).await
}

async fn collection_update(
    connection: &IpcRedisConnection,
    database: Option<u8>,
    command: &str,
    key: &str,
    values: &[&str],
) -> Result<i64, RedisError> {
    let mut args = vec![command.to_string(), key.to_string()];
    args.extend(values.iter().map(|value| (*value).to_string()));
    value_integer(connection.command_owned(database, args).await?)
}

fn value_integer(value: RedisValue) -> Result<i64, RedisError> {
    value.as_integer().ok_or_else(|| RedisError::TypeMismatch {
        expected: "integer".into(),
        actual: value.to_display_string(),
    })
}

fn optional_integer(result: Result<RedisValue, RedisError>) -> Option<i64> {
    result.ok()?.as_integer()
}

fn value_string(value: RedisValue) -> Result<String, RedisError> {
    match value {
        RedisValue::String(value) | RedisValue::Status(value) => Ok(value),
        RedisValue::Binary(value) => {
            String::from_utf8(value).map_err(|error| RedisError::Serialization(error.to_string()))
        }
        RedisValue::Integer(value) => Ok(value.to_string()),
        RedisValue::Float(value) => Ok(value.to_string()),
        value => Err(RedisError::TypeMismatch {
            expected: "string".into(),
            actual: value.to_display_string(),
        }),
    }
}

fn value_bytes(value: RedisValue) -> Result<Vec<u8>, RedisError> {
    match value {
        RedisValue::String(value) | RedisValue::Status(value) => Ok(value.into_bytes()),
        RedisValue::Binary(value) => Ok(value),
        RedisValue::Integer(value) => Ok(value.to_string().into_bytes()),
        RedisValue::Float(value) => Ok(value.to_string().into_bytes()),
        value => Err(RedisError::TypeMismatch {
            expected: "byte string".into(),
            actual: value.to_display_string(),
        }),
    }
}

fn value_strings(value: RedisValue) -> Result<Vec<String>, RedisError> {
    match value {
        RedisValue::Bulk(values) => values.into_iter().map(value_string).collect(),
        value => Err(RedisError::TypeMismatch {
            expected: "array".into(),
            actual: value.to_display_string(),
        }),
    }
}

fn value_byte_strings(value: RedisValue) -> Result<Vec<Vec<u8>>, RedisError> {
    match value {
        RedisValue::Bulk(values) => values.into_iter().map(value_bytes).collect(),
        value => Err(RedisError::TypeMismatch {
            expected: "array".into(),
            actual: value.to_display_string(),
        }),
    }
}

fn parse_scan(value: RedisValue) -> Result<ScanResult, RedisError> {
    if let RedisValue::Error(message) = value {
        return Err(RedisError::command(message));
    }
    let RedisValue::Bulk(mut values) = value else {
        return Err(RedisError::TypeMismatch {
            expected: "scan tuple".into(),
            actual: format!(
                "invalid SCAN response from Redis native driver: {}",
                value.to_display_string()
            ),
        });
    };
    if values.len() != 2 {
        return Err(RedisError::Serialization(
            "invalid SCAN response from Redis native driver: expected cursor and keys".into(),
        ));
    }
    let keys = scan_key_strings(values.pop().expect("length checked"))?;
    let cursor = value_string(values.pop().expect("length checked"))?
        .parse::<u64>()
        .map_err(|error| {
            RedisError::Serialization(format!(
                "invalid SCAN cursor returned by Redis native driver: {error}"
            ))
        })?;
    Ok(ScanResult::new(cursor, keys))
}

fn scan_key_strings(value: RedisValue) -> Result<Vec<String>, RedisError> {
    let RedisValue::Bulk(values) = value else {
        return Err(RedisError::TypeMismatch {
            expected: "SCAN key array".into(),
            actual: format!(
                "invalid SCAN response from Redis native driver: {}",
                value.to_display_string()
            ),
        });
    };
    values
        .into_iter()
        .map(|value| match value {
            RedisValue::Binary(value) => String::from_utf8(value).map_err(|_| {
                RedisError::Serialization(
                    "SCAN returned a non-UTF-8 Redis key; binary Redis keys are not currently supported by the key tree"
                        .into(),
                )
            }),
            value => value_string(value),
        })
        .collect()
}

fn parse_key_type(value: &str) -> RedisKeyType {
    value.parse().unwrap_or(RedisKeyType::None)
}

fn parse_hash_fields(value: RedisValue) -> Result<Vec<HashField>, RedisError> {
    let values = value_byte_strings(value)?;
    if values.len() % 2 != 0 {
        return Err(RedisError::Serialization(
            "HGETALL response has an odd number of fields".into(),
        ));
    }
    Ok(values
        .chunks_exact(2)
        .map(|pair| HashField {
            field: pair[0].clone(),
            value: pair[1].clone(),
        })
        .collect())
}

fn parse_zset(value: RedisValue) -> Result<Vec<ZSetMember>, RedisError> {
    let RedisValue::Bulk(values) = value else {
        return Err(RedisError::TypeMismatch {
            expected: "array".into(),
            actual: value.to_display_string(),
        });
    };
    if values.len() % 2 != 0 {
        return Err(RedisError::Serialization(
            "ZRANGE WITHSCORES response has an odd number of fields".into(),
        ));
    }
    values
        .chunks_exact(2)
        .map(|pair| {
            Ok(ZSetMember {
                member: value_bytes(pair[0].clone())?,
                score: value_string(pair[1].clone())?
                    .parse::<f64>()
                    .map_err(|error| RedisError::Serialization(error.to_string()))?,
            })
        })
        .collect()
}

fn parse_stream_entries(value: RedisValue) -> Result<Vec<StreamEntry>, RedisError> {
    let RedisValue::Bulk(entries) = value else {
        return Err(RedisError::TypeMismatch {
            expected: "stream entries".into(),
            actual: value.to_display_string(),
        });
    };
    entries
        .into_iter()
        .map(|entry| {
            let RedisValue::Bulk(mut parts) = entry else {
                return Err(RedisError::Serialization("invalid stream entry".into()));
            };
            if parts.len() != 2 {
                return Err(RedisError::Serialization("invalid stream entry".into()));
            }
            let fields = value_byte_strings(parts.pop().expect("length checked"))?;
            if fields.len() % 2 != 0 {
                return Err(RedisError::Serialization(
                    "stream field list has odd length".into(),
                ));
            }
            let id = value_string(parts.pop().expect("length checked"))?;
            Ok(StreamEntry {
                id,
                fields: fields
                    .chunks_exact(2)
                    .map(|pair| HashField {
                        field: pair[0].clone(),
                        value: pair[1].clone(),
                    })
                    .collect(),
            })
        })
        .collect()
}

fn parse_info(info: &str) -> std::collections::HashMap<String, String> {
    info.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.trim().split_once(':'))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn parse_info_i64(values: &std::collections::HashMap<String, String>, key: &str) -> i64 {
    values
        .get(key)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn seconds_to_millis(seconds: u64) -> u32 {
    seconds.saturating_mul(1_000).min(u32::MAX as u64) as u32
}

fn wire_config_for_target(
    config: &RedisConnectionConfig,
    host: &str,
    port: u16,
) -> WireRedisConnectionConfig {
    WireRedisConnectionConfig {
        host: host.to_string(),
        port,
        username: config.username.clone(),
        password: config.password.clone(),
        database: config.db_index,
        use_tls: config.use_tls,
        connect_timeout_ms: Some(seconds_to_millis(config.timeout)),
    }
}

fn wire_bytes(bytes: Vec<u8>) -> WireBytes {
    match String::from_utf8(bytes) {
        Ok(value) => WireBytes::Utf8(value),
        Err(error) => {
            WireBytes::Base64(base64::engine::general_purpose::STANDARD.encode(error.into_bytes()))
        }
    }
}

fn domain_value(value: RedisRespValue) -> RedisValue {
    match value {
        RedisRespValue::Nil => RedisValue::Nil,
        RedisRespValue::Integer(value) => RedisValue::Integer(value),
        RedisRespValue::Double(value) => RedisValue::Float(value),
        RedisRespValue::Boolean(value) => RedisValue::Integer(i64::from(value)),
        RedisRespValue::Bytes(WireBytes::Utf8(value)) => RedisValue::String(value),
        RedisRespValue::Bytes(WireBytes::Base64(value)) => {
            match base64::engine::general_purpose::STANDARD.decode(value) {
                Ok(value) => RedisValue::Binary(value),
                Err(error) => RedisValue::Error(error.to_string()),
            }
        }
        RedisRespValue::SimpleString(value) => RedisValue::Status(value),
        RedisRespValue::Error(value) => RedisValue::Error(value),
        RedisRespValue::Array(values) | RedisRespValue::Set(values) => {
            RedisValue::Bulk(values.into_iter().map(domain_value).collect())
        }
        RedisRespValue::Map(values) => RedisValue::Bulk(
            values
                .into_iter()
                .map(|(key, value)| RedisValue::Bulk(vec![domain_value(key), domain_value(value)]))
                .collect(),
        ),
    }
}

fn native_driver_context(manifest: &NativeDriverManifest) -> String {
    let version = if manifest.version.trim().is_empty() {
        "unknown"
    } else {
        manifest.version.as_str()
    };
    format!(
        "driver={}, version={}, protocol={}, manifest={}",
        manifest.id,
        version,
        manifest.protocol_version,
        manifest.manifest_dir.display()
    )
}

fn host_connection_error(error: HostError, driver_context: &str) -> RedisError {
    RedisError::connection(format!("{error}; {driver_context}"))
}

fn redis_command_host_error(error: HostError, driver_context: &str) -> RedisError {
    match error {
        HostError::Protocol(error) if !error.is_connection_error() => {
            RedisError::command(format!("{}; {driver_context}", error.message))
        }
        error => host_connection_error(error, driver_context),
    }
}

fn serialization_error(error: impl std::fmt::Display) -> RedisError {
    RedisError::Serialization(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_protocol::error::{ProtocolError, error_codes};

    #[test]
    fn non_utf8_values_remain_binary() {
        let value = domain_value(RedisRespValue::Bytes(WireBytes::Base64("AP8=".into())));
        assert_eq!(RedisValue::Binary(vec![0, 0xff]), value);
    }

    #[test]
    fn scan_response_from_ipc_array_is_parsed() {
        let value = domain_value(RedisRespValue::Array(vec![
            RedisRespValue::Bytes(WireBytes::Utf8("17".into())),
            RedisRespValue::Array(vec![
                RedisRespValue::Bytes(WireBytes::Utf8("user:1".into())),
                RedisRespValue::Bytes(WireBytes::Utf8("user:2".into())),
            ]),
        ]));

        let result = parse_scan(value).expect("IPC SCAN response should follow Redis tuple shape");

        assert_eq!(17, result.cursor);
        assert_eq!(vec!["user:1", "user:2"], result.keys);
        assert!(!result.finished);
    }

    #[test]
    fn scan_command_error_preserves_redis_acl_message() {
        let error = parse_scan(RedisValue::Error(
            "NOPERM this user has no permissions to run the 'scan' command".into(),
        ))
        .expect_err("Redis command errors must not be reported as response shape errors");

        assert!(matches!(error, RedisError::Command { .. }));
        let message = error.to_string();
        assert!(message.contains("NOPERM"));
        assert!(message.contains("scan"));
    }

    #[test]
    fn scan_rejects_invalid_driver_response_shape_with_actionable_context() {
        let error = parse_scan(RedisValue::Bulk(vec![RedisValue::String("0".into())]))
            .expect_err("incomplete SCAN tuples must be rejected");

        let message = error.to_string();
        assert!(message.contains("invalid SCAN response"));
        assert!(message.contains("native driver"));
    }

    #[test]
    fn scan_reports_non_utf8_keys_without_lossy_conversion() {
        let error = parse_scan(RedisValue::Bulk(vec![
            RedisValue::String("0".into()),
            RedisValue::Bulk(vec![
                RedisValue::String("visible".into()),
                RedisValue::Binary(vec![0xff, 0x00]),
            ]),
        ]))
        .expect_err("binary Redis keys are not representable by the current String key contract");

        let message = error.to_string();
        assert!(message.contains("non-UTF-8 Redis key"));
        assert!(message.contains("not currently supported"));
    }

    #[test]
    fn sidecar_command_protocol_errors_remain_command_errors_with_driver_context() {
        let context = "driver=redis, version=0.1.1, protocol=1.0";
        let error = redis_command_host_error(
            extension_host::HostError::protocol(ProtocolError::new(
                error_codes::EXTENSION_CUSTOM_START,
                "NOPERM this user has no permissions to run the 'scan' command",
            )),
            context,
        );

        assert!(matches!(error, RedisError::Command { .. }));
        let message = error.to_string();
        assert!(message.contains("NOPERM"));
        assert!(message.contains(context));
    }

    #[test]
    fn sidecar_connection_protocol_errors_remain_connection_errors() {
        let error = redis_command_host_error(
            extension_host::HostError::protocol(ProtocolError::new(
                error_codes::IO_CONNECTION_REFUSED,
                "connection reset",
            )),
            "driver=redis, version=0.1.2, protocol=1.0",
        );

        assert!(matches!(error, RedisError::Connection { .. }));
    }

    #[test]
    fn native_driver_context_includes_version_protocol_and_manifest_path() {
        let mut manifest: NativeDriverManifest = serde_json::from_value(serde_json::json!({
            "id": "redis",
            "name": "Redis",
            "version": "0.1.2",
            "api": "redis",
            "protocol_version": "1.0",
            "entry": { "command": "./redis-driver" },
            "transport": { "name": "redis.sock" }
        }))
        .expect("test manifest should deserialize");
        manifest.manifest_dir = std::path::PathBuf::from("/tmp/database_drivers/redis");

        let context = native_driver_context(&manifest);

        assert!(context.contains("driver=redis"));
        assert!(context.contains("version=0.1.2"));
        assert!(context.contains("protocol=1.0"));
        assert!(context.contains("/tmp/database_drivers/redis"));
    }

    #[test]
    fn zset_members_preserve_non_utf8_bytes() {
        let bytes = vec![0x0b, 0xcf, 0xdb, 0xde, 0x01, 0x00];
        let members = parse_zset(RedisValue::Bulk(vec![
            RedisValue::Binary(bytes.clone()),
            RedisValue::String("42.5".into()),
        ]))
        .expect("binary ZSet members should remain readable");

        assert_eq!(bytes.as_slice(), members[0].member.as_slice());
        assert_eq!(42.5, members[0].score);
    }

    #[test]
    fn list_and_set_members_preserve_non_utf8_bytes() {
        let bytes = vec![0x0b, 0xcf, 0xdb, 0xde, 0x01, 0x00];
        let values = value_byte_strings(RedisValue::Bulk(vec![
            RedisValue::String("text".into()),
            RedisValue::Binary(bytes.clone()),
        ]))
        .expect("binary collection members should remain readable");

        assert_eq!(b"text", values[0].as_slice());
        assert_eq!(bytes, values[1]);
    }

    #[test]
    fn hash_fields_and_values_preserve_non_utf8_bytes() {
        let field = vec![0xff, 0x00, b'f'];
        let value = vec![0x0b, 0xcf, 0xdb, 0xde];
        let fields = parse_hash_fields(RedisValue::Bulk(vec![
            RedisValue::Binary(field.clone()),
            RedisValue::Binary(value.clone()),
        ]))
        .expect("binary Hash fields and values should remain readable");

        assert_eq!(field, fields[0].field);
        assert_eq!(value, fields[0].value);
    }

    #[test]
    fn stream_fields_and_values_preserve_non_utf8_bytes() {
        let field = vec![0xff, 0x00, b'f'];
        let value = vec![0x0b, 0xcf, 0xdb, 0xde];
        let entries = parse_stream_entries(RedisValue::Bulk(vec![RedisValue::Bulk(vec![
            RedisValue::String("1-0".into()),
            RedisValue::Bulk(vec![
                RedisValue::Binary(field.clone()),
                RedisValue::Binary(value.clone()),
            ]),
        ])]))
        .expect("binary Stream fields and values should remain readable");

        assert_eq!(field, entries[0].fields[0].field);
        assert_eq!(value, entries[0].fields[0].value);
    }

    #[test]
    fn maps_preserve_key_value_pairs() {
        let value = domain_value(RedisRespValue::Map(vec![(
            RedisRespValue::SimpleString("key".into()),
            RedisRespValue::Integer(1),
        )]));
        assert_eq!(
            RedisValue::Bulk(vec![RedisValue::Bulk(vec![
                RedisValue::Status("key".into()),
                RedisValue::Integer(1),
            ])]),
            value
        );
    }

    #[test]
    fn optional_integer_ignores_unsupported_metadata_commands() {
        let unsupported = Err(RedisError::command("unknown command 'MEMORY'"));

        assert_eq!(None, optional_integer(unsupported));
        assert_eq!(Some(128), optional_integer(Ok(RedisValue::Integer(128))));
    }

    #[test]
    fn wire_config_uses_the_resolved_tunnel_endpoint() {
        let config = RedisConnectionConfig {
            host: "redis.internal".into(),
            port: 6379,
            ..Default::default()
        };

        let wire = wire_config_for_target(&config, "127.0.0.1", 49152);

        assert_eq!("127.0.0.1", wire.host);
        assert_eq!(49152, wire.port);
    }
}
