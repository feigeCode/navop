#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use base64::Engine as _;
use extension_driver::{
    AsyncDriverConnection, AsyncNativeDriver, AsyncOpenedConnection, serve_async_from_env,
};
use extension_protocol::blob::WireBytes;
use extension_protocol::conn::{ConnOpenParams, ConnOpenResult};
use extension_protocol::error::{ProtocolError, error_codes};
use extension_protocol::event_stream::{
    EventCloseParams, EventOpenParams, EventOpenResult, EventReadParams, EventReadResult,
};
use extension_protocol::lifecycle::{Capability, InitResult};
use extension_protocol::method;
use extension_protocol::redis::{
    RedisCommandParams, RedisCommandResult, RedisConnectionConfig, RedisPipelineParams,
    RedisPipelineResult, RedisRespValue,
};
use futures::StreamExt;
use redis_client::aio::ConnectionManager;
use redis_client::{Client, Cmd, Value};
use serde_json::{Value as JsonValue, json};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{Duration, timeout};

const SOCKET_ENV: &str = "ONETCLI_EXT_SOCKET";

struct RedisDriver {
    next_conn_id: AtomicU64,
}

struct RedisConnection {
    config: RedisConnectionConfig,
    connections: HashMap<u8, ConnectionManager>,
    events: HashMap<String, RedisEvent>,
}

struct RedisEvent {
    commands: mpsc::Sender<PubSubCommand>,
    events: Arc<Mutex<mpsc::Receiver<JsonValue>>>,
    dropped: Arc<AtomicU64>,
    task: JoinHandle<()>,
}

enum PubSubCommand {
    Subscribe(String),
    PSubscribe(String),
    Unsubscribe(String),
    PUnsubscribe(String),
}

#[async_trait]
impl AsyncNativeDriver for RedisDriver {
    async fn init(&self, _params: &JsonValue) -> Result<JsonValue, ProtocolError> {
        let result = InitResult::new(env!("CARGO_PKG_VERSION"))
            .with_api("redis", extension_protocol::WIRE_PROTOCOL_VERSION)
            .with_feature(Capability::CANCEL_REQUEST)
            .with_method(method::REDIS_COMMAND)
            .with_method(method::REDIS_PIPELINE)
            .with_driver("redis");
        serde_json::to_value(result).map_err(internal_error)
    }

    async fn open_connection(
        &self,
        params: &JsonValue,
    ) -> Result<AsyncOpenedConnection, ProtocolError> {
        let open: ConnOpenParams =
            serde_json::from_value(params.clone()).map_err(invalid_params)?;
        let config: RedisConnectionConfig =
            serde_json::from_value(open.config).map_err(invalid_params)?;
        let client = Client::open(connection_url(&config).as_str()).map_err(connection_error)?;
        let manager = client
            .get_connection_manager()
            .await
            .map_err(connection_error)?;
        let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
        let mut connections = HashMap::new();
        connections.insert(config.database, manager);
        let result = ConnOpenResult {
            conn_id,
            server_info: None,
        };
        Ok(AsyncOpenedConnection {
            conn_id,
            open_result: serde_json::to_value(result).map_err(internal_error)?,
            connection: Box::new(RedisConnection {
                config,
                connections,
                events: HashMap::new(),
            }),
        })
    }

    async fn call_connless(
        &self,
        method_name: &str,
        _params: &JsonValue,
    ) -> Result<JsonValue, ProtocolError> {
        Err(ProtocolError::new(
            error_codes::METHOD_NOT_FOUND,
            format!("unsupported connless Redis method `{method_name}`"),
        ))
    }
}

#[async_trait]
impl AsyncDriverConnection for RedisConnection {
    async fn call(
        &mut self,
        method_name: &str,
        params: &JsonValue,
    ) -> Result<JsonValue, ProtocolError> {
        match method_name {
            method::REDIS_COMMAND => {
                let params: RedisCommandParams =
                    serde_json::from_value(params.clone()).map_err(invalid_params)?;
                let value = self.execute(params.database, params.args).await?;
                serde_json::to_value(RedisCommandResult { value }).map_err(internal_error)
            }
            method::REDIS_PIPELINE => {
                let params: RedisPipelineParams =
                    serde_json::from_value(params.clone()).map_err(invalid_params)?;
                params
                    .validate()
                    .map_err(|message| ProtocolError::new(error_codes::INVALID_PARAMS, message))?;
                let mut values = Vec::with_capacity(params.commands.len());
                for args in params.commands {
                    values.push(self.execute(params.database, args).await?);
                }
                serde_json::to_value(RedisPipelineResult { values }).map_err(internal_error)
            }
            method::EVENT_OPEN => self.open_event(params).await,
            method::EVENT_READ => self.read_event(params).await,
            method::EVENT_CLOSE => self.close_event(params).await,
            method::REDIS_PUBSUB_CONTROL => self.control_event(params).await,
            _ => Err(ProtocolError::new(
                error_codes::METHOD_NOT_FOUND,
                format!("unsupported Redis method `{method_name}`"),
            )),
        }
    }

    async fn close(&mut self) {
        self.connections.clear();
        for (_, event) in self.events.drain() {
            event.task.abort();
        }
    }
}

impl RedisConnection {
    async fn manager(
        &mut self,
        database: Option<u8>,
    ) -> Result<&mut ConnectionManager, ProtocolError> {
        let database = database.unwrap_or(self.config.database);
        if !self.connections.contains_key(&database) {
            let mut config = self.config.clone();
            config.database = database;
            let client =
                Client::open(connection_url(&config).as_str()).map_err(connection_error)?;
            let manager = client
                .get_connection_manager()
                .await
                .map_err(connection_error)?;
            self.connections.insert(database, manager);
        }
        self.connections.get_mut(&database).ok_or_else(|| {
            ProtocolError::new(error_codes::INTERNAL_ERROR, "Redis manager was not cached")
        })
    }

    async fn execute(
        &mut self,
        database: Option<u8>,
        args: Vec<WireBytes>,
    ) -> Result<RedisRespValue, ProtocolError> {
        let mut args = args.into_iter();
        let command = args.next().ok_or_else(|| {
            ProtocolError::new(error_codes::INVALID_PARAMS, "Redis command cannot be empty")
        })?;
        let command = decode_wire_bytes(command)?;
        let mut cmd = Cmd::new();
        cmd.arg(command);
        for arg in args {
            cmd.arg(decode_wire_bytes(arg)?);
        }
        let value: Value = cmd
            .query_async(self.manager(database).await?)
            .await
            .map_err(command_error)?;
        Ok(redis_value(value))
    }

    async fn open_event(&mut self, params: &JsonValue) -> Result<JsonValue, ProtocolError> {
        let params: EventOpenParams =
            serde_json::from_value(params.clone()).map_err(invalid_params)?;
        if params.kind != "redis_pubsub" {
            return Err(ProtocolError::new(
                error_codes::INVALID_PARAMS,
                "Redis sidecar only supports redis_pubsub event streams",
            ));
        }
        let capacity = params.capacity.unwrap_or(128).clamp(1, 1024) as usize;
        let client =
            Client::open(connection_url(&self.config).as_str()).map_err(connection_error)?;
        let pubsub = client.get_async_pubsub().await.map_err(connection_error)?;
        let stream_id = format!("redis-events-{}", EVENT_ID.fetch_add(1, Ordering::Relaxed));
        let (command_tx, mut command_rx) = mpsc::channel::<PubSubCommand>(32);
        let (event_tx, event_rx) = mpsc::channel::<JsonValue>(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let dropped_for_task = Arc::clone(&dropped);
        let task = tokio::spawn(async move {
            let (mut sink, mut stream) = pubsub.split();
            loop {
                tokio::select! {
                    command = command_rx.recv() => {
                        let Some(command) = command else { break; };
                        let result = match command {
                            PubSubCommand::Subscribe(channel) => sink.subscribe(channel).await,
                            PubSubCommand::PSubscribe(pattern) => sink.psubscribe(pattern).await,
                            PubSubCommand::Unsubscribe(channel) => sink.unsubscribe(channel).await,
                            PubSubCommand::PUnsubscribe(pattern) => sink.punsubscribe(pattern).await,
                        };
                        if result.is_err() { break; }
                    }
                    message = stream.next() => {
                        let Some(message) = message else { break; };
                        let payload = message.get_payload_bytes();
                        let payload = match String::from_utf8(payload.to_vec()) {
                            Ok(value) => json!({"encoding": "utf8", "value": value}),
                            Err(error) => json!({"encoding": "base64", "value": base64::engine::general_purpose::STANDARD.encode(error.into_bytes())}),
                        };
                        let event = json!({
                            "kind": if message.from_pattern() { "pmessage" } else { "message" },
                            "channel": message.get_channel_name(),
                            "pattern": message.get_pattern::<String>().ok(),
                            "payload": payload,
                        });
                        if event_tx.try_send(event).is_err() {
                            dropped_for_task.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
        });
        self.events.insert(
            stream_id.clone(),
            RedisEvent {
                commands: command_tx,
                events: Arc::new(Mutex::new(event_rx)),
                dropped,
                task,
            },
        );
        serde_json::to_value(EventOpenResult { stream_id }).map_err(internal_error)
    }

    async fn control_event(&mut self, params: &JsonValue) -> Result<JsonValue, ProtocolError> {
        let params: extension_protocol::redis::RedisPubSubControlParams =
            serde_json::from_value(params.clone()).map_err(invalid_params)?;
        let event = self.events.get(&params.stream_id).ok_or_else(|| {
            ProtocolError::new(error_codes::RESOURCE_CLOSED, "event stream is closed")
        })?;
        let command = match params.control {
            extension_protocol::redis::RedisPubSubControl::Subscribe(value) => {
                PubSubCommand::Subscribe(
                    String::from_utf8(decode_wire_bytes(value)?).map_err(invalid_params)?,
                )
            }
            extension_protocol::redis::RedisPubSubControl::PSubscribe(value) => {
                PubSubCommand::PSubscribe(
                    String::from_utf8(decode_wire_bytes(value)?).map_err(invalid_params)?,
                )
            }
            extension_protocol::redis::RedisPubSubControl::Unsubscribe(value) => {
                PubSubCommand::Unsubscribe(
                    String::from_utf8(decode_wire_bytes(value)?).map_err(invalid_params)?,
                )
            }
            extension_protocol::redis::RedisPubSubControl::PUnsubscribe(value) => {
                PubSubCommand::PUnsubscribe(
                    String::from_utf8(decode_wire_bytes(value)?).map_err(invalid_params)?,
                )
            }
        };
        event.commands.send(command).await.map_err(|_| {
            ProtocolError::new(error_codes::RESOURCE_CLOSED, "event stream is closed")
        })?;
        Ok(JsonValue::Null)
    }

    async fn read_event(&mut self, params: &JsonValue) -> Result<JsonValue, ProtocolError> {
        let params: EventReadParams =
            serde_json::from_value(params.clone()).map_err(invalid_params)?;
        let event = self.events.get(&params.stream_id).ok_or_else(|| {
            ProtocolError::new(error_codes::RESOURCE_CLOSED, "event stream is closed")
        })?;
        let max_events = params.effective_max_events() as usize;
        let mut receiver = event.events.lock().await;
        let mut values = Vec::with_capacity(max_events);
        if let Some(wait_ms) = params.wait_ms {
            if values.is_empty() && wait_ms > 0 {
                let first = timeout(Duration::from_millis(wait_ms as u64), receiver.recv()).await;
                if let Ok(Some(value)) = first {
                    values.push(value);
                }
            }
        }
        while values.len() < max_events {
            match receiver.try_recv() {
                Ok(value) => values.push(value),
                Err(_) => break,
            }
        }
        let dropped_count = event.dropped.swap(0, Ordering::Relaxed);
        serde_json::to_value(EventReadResult {
            events: values,
            closed: receiver.is_closed(),
            dropped_count,
        })
        .map_err(internal_error)
    }

    async fn close_event(&mut self, params: &JsonValue) -> Result<JsonValue, ProtocolError> {
        let params: EventCloseParams =
            serde_json::from_value(params.clone()).map_err(invalid_params)?;
        if let Some(event) = self.events.remove(&params.stream_id) {
            event.task.abort();
            Ok(JsonValue::Null)
        } else {
            Err(ProtocolError::new(
                error_codes::RESOURCE_CLOSED,
                "event stream is closed",
            ))
        }
    }
}

static EVENT_ID: AtomicU64 = AtomicU64::new(1);

fn decode_wire_bytes(value: WireBytes) -> Result<Vec<u8>, ProtocolError> {
    match value {
        WireBytes::Utf8(value) => Ok(value.into_bytes()),
        WireBytes::Base64(value) => base64::engine::general_purpose::STANDARD
            .decode(value)
            .map_err(|error| {
                ProtocolError::new(error_codes::DATA_INVALID_ENCODING, error.to_string())
            }),
    }
}

fn encode_bytes(bytes: Vec<u8>) -> WireBytes {
    match String::from_utf8(bytes) {
        Ok(value) => WireBytes::Utf8(value),
        Err(error) => {
            WireBytes::Base64(base64::engine::general_purpose::STANDARD.encode(error.into_bytes()))
        }
    }
}

fn redis_value(value: Value) -> RedisRespValue {
    match value {
        Value::Nil => RedisRespValue::Nil,
        Value::Int(value) => RedisRespValue::Integer(value),
        Value::BulkString(value) => RedisRespValue::Bytes(encode_bytes(value)),
        Value::Array(values) => {
            RedisRespValue::Array(values.into_iter().map(redis_value).collect())
        }
        Value::SimpleString(value) => RedisRespValue::SimpleString(value),
        Value::Okay => RedisRespValue::SimpleString("OK".into()),
        Value::Map(values) => RedisRespValue::Map(
            values
                .into_iter()
                .map(|(key, value)| (redis_value(key), redis_value(value)))
                .collect(),
        ),
        Value::Attribute { data, .. } => redis_value(*data),
        Value::Set(values) => RedisRespValue::Set(values.into_iter().map(redis_value).collect()),
        Value::Double(value) => RedisRespValue::Double(value),
        Value::Boolean(value) => RedisRespValue::Boolean(value),
        Value::VerbatimString { text, .. } => RedisRespValue::SimpleString(text),
        Value::BigNumber(value) => RedisRespValue::SimpleString(value.to_string()),
        Value::Push { data, .. } => {
            RedisRespValue::Array(data.into_iter().map(redis_value).collect())
        }
        Value::ServerError(error) => RedisRespValue::Error(format!("{error:?}")),
    }
}

fn connection_url(config: &RedisConnectionConfig) -> String {
    let scheme = if config.use_tls { "rediss" } else { "redis" };
    let auth = match (&config.username, &config.password) {
        (Some(username), Some(password)) => format!("{username}:{password}@"),
        (None, Some(password)) => format!("default:{password}@"),
        _ => String::new(),
    };
    format!(
        "{scheme}://{auth}{}:{}/{}",
        config.host, config.port, config.database
    )
}

fn invalid_params(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::INVALID_PARAMS, error.to_string())
}

fn connection_error(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::IO_CONNECTION_REFUSED, error.to_string())
}

fn command_error(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::EXTENSION_CUSTOM_START, error.to_string())
}

fn internal_error(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::new(error_codes::INTERNAL_ERROR, error.to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    serve_async_from_env(
        RedisDriver {
            next_conn_id: AtomicU64::new(1),
        },
        SOCKET_ENV,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_values_round_trip_through_base64_wire_form() {
        let bytes = vec![0, 0xff, 1];
        let wire = encode_bytes(bytes.clone());
        assert_eq!(bytes, decode_wire_bytes(wire).unwrap());
    }

    #[test]
    fn redis_arrays_preserve_nested_value_kinds() {
        assert_eq!(
            RedisRespValue::Array(vec![
                RedisRespValue::Integer(1),
                RedisRespValue::Bytes(WireBytes::Utf8("ok".into())),
            ]),
            redis_value(Value::Array(vec![
                Value::Int(1),
                Value::BulkString(b"ok".to_vec()),
            ]))
        );
    }
}
