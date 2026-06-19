//! v2 wire 客户端——把 `extension-host` 的 JSON-RPC 2.0 primitives 包装成
//! 业务层易用的「请求/响应」接口。
//!
//! 核心职责:
//!
//! - **进程管理**:spawn / kill_on_drop / socket 握手都委托给
//!   `extension_host::process`,本地只剩组合调用。
//! - **生命周期 init 化**:连上之后必须先 `init` 才能调业务方法,本 client 自动
//!   做握手,并把 capability 集合保留在 [`ExtensionSession`] 中供上层查询。
//!
//! 取消、超时、并发路由这些复杂部分全部下沉到 `extension-host::JsonRpcClient`;
//! 本层只需把 `HostError` 转成业务侧的 [`DbError`]。

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::connection::DbError;
use crate::ipc::registry::IpcDriverManifest;
use extension_host::client::{JsonRpcClient as HostClient, JsonRpcClientHandle, RequestOptions};
use extension_host::error::HostError;
use extension_host::negotiation::{ExtensionSession, NegotiationConfig, negotiate, shutdown};
use extension_host::process::{ProcessHandle, SpawnConfig, SpawnTransport, default_socket_name};
use extension_host::transport::FramedTransport;
use one_core::storage::DbConnectionConfig;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Mutex as StdMutex;
use tracing::warn;

/// 单次请求默认超时(毫秒)。
const REQUEST_TIMEOUT_MS: u64 = 30_000;

/// shutdown 优雅时长(毫秒)。
const SHUTDOWN_GRACE_MS: u32 = 5_000;

/// 与一个扩展子进程通讯的 v2 wire 客户端。
///
/// 内部组合:
/// - [`HostClient`](extension_host::JsonRpcClient):reader task 持有者,Drop 时 abort
/// - [`JsonRpcClientHandle`][]:轻量请求句柄(可 clone)
/// - [`ProcessHandle`][]:子进程持有者,kill_on_drop=true
/// - [`ExtensionSession`][]:init 握手后协商出的 capability 集合
///
/// 用法:
/// ```ignore
/// let client = JsonRpcClient::start(&driver).await?;
/// let result: serde_json::Value = client.request("conn/open", params).await?;
/// // ...
/// client.shutdown().await;
/// ```
pub struct JsonRpcClient {
    handle: JsonRpcClientHandle,
    session: ExtensionSession,
    // 用 std Mutex<Option<...>> 而不是 tokio Mutex,是因为 shutdown 时希望同步 take,
    // 之后再 await 各自的关闭流程(reader task / process kill)。
    inner: StdMutex<Option<ClientInner>>,
    // 写一次性 flag:reader 退出后,主动询问 is_closed 通过此提示——也可直接用
    // handle.is_closed(),保留为日后扩展(例如观测)
    driver_id: String,
}

struct ClientInner {
    owner: HostClient,
    process: ProcessHandle,
}

impl JsonRpcClient {
    /// 根据 driver manifest spawn 子进程、握手、返回可用客户端。
    ///
    /// 步骤:
    /// 1. spawn 子进程(用 `extension_host::process::spawn`)。
    /// 2. 把握得到的 stream split 成 reader/writer → 喂给 [`FramedTransport`]。
    /// 3. `JsonRpcClient::start` 启 reader task,得到 handle。
    /// 4. `negotiate` 做 init 握手,得到 ExtensionSession。
    pub async fn start(driver: &IpcDriverManifest) -> Result<Self, DbError> {
        Self::start_with_connection_config(driver, None).await
    }

    pub async fn start_with_connection_config(
        driver: &IpcDriverManifest,
        config: Option<&DbConnectionConfig>,
    ) -> Result<Self, DbError> {
        if driver.entry.command.trim().is_empty() {
            return Err(DbError::connection(format!(
                "external driver '{}' has empty command",
                driver.id
            )));
        }

        let spawn_config = build_spawn_config(driver, config);
        let mut process = extension_host::process::spawn(spawn_config)
            .await
            .map_err(host_error_to_db_error)?;

        let stream = process.take_stream().ok_or_else(|| {
            DbError::connection(format!(
                "external driver '{}' did not return a connected stream",
                driver.id
            ))
        })?;

        let (reader, writer) = tokio::io::split(stream);
        let transport = FramedTransport::new(reader, writer);
        let owner = HostClient::start(transport);
        let handle = owner.handle();

        // init 握手
        let negotiation = build_negotiation(driver);
        let session = negotiate(&handle, negotiation)
            .await
            .map_err(host_error_to_db_error)?;

        Ok(Self {
            handle,
            session,
            inner: StdMutex::new(Some(ClientInner { owner, process })),
            driver_id: driver.id.clone(),
        })
    }

    /// 调用 wire 方法并把 result 反序列化为 `T`。
    pub async fn request<T>(&self, method: &str, params: Value) -> Result<T, DbError>
    where
        T: DeserializeOwned,
    {
        let raw = self.request_value(method, params).await?;
        serde_json::from_value::<T>(raw)
            .map_err(|error| DbError::query_with_source("invalid external driver response", error))
    }

    /// 调用 wire 方法,返回 raw `serde_json::Value`(协议层场景用)。
    pub async fn request_value(&self, method: &str, params: Value) -> Result<Value, DbError> {
        let options =
            RequestOptions::default().with_timeout(Duration::from_millis(REQUEST_TIMEOUT_MS));
        self.handle
            .call_raw(method, params, options)
            .await
            .map_err(host_error_to_db_error)
    }

    /// 当前协商出的 capability 集合(`init` 之后冻结)。
    pub fn session(&self) -> &ExtensionSession {
        &self.session
    }

    /// driver 是否支持某项 capability。
    pub fn supports(&self, capability: &str) -> bool {
        self.session.has_feature(capability)
    }

    /// 是否已关闭(reader task 退出 / 用户调 `shutdown`)。
    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// 优雅关闭:先发 `shutdown` RPC 给扩展,再 abort reader,最后 kill child。
    pub async fn shutdown(&self) {
        // 1. 让扩展尝试 graceful shutdown(grace 时间内自行清理)。
        if let Err(error) = shutdown(&self.handle, SHUTDOWN_GRACE_MS).await {
            // 协议 shutdown 失败不致命(可能是子进程已经异常退出),warning 即可。
            warn!(
                driver = %self.driver_id,
                error = %error,
                "graceful shutdown failed; proceeding to abort reader and kill child"
            );
        }

        // 2. 取出 owner 与 process,owner.shutdown 内部 await reader task 退出,
        //    process 在 Drop 时 kill_on_drop。
        let inner = {
            let mut guard = self.inner.lock().expect("inner mutex poisoned");
            guard.take()
        };
        if let Some(ClientInner { owner, process }) = inner {
            owner.shutdown().await;
            drop(process);
        }
    }
}

impl Drop for JsonRpcClient {
    fn drop(&mut self) {
        // 兜底:确保 handle 标记关闭(让 in-flight caller 收 Closed),
        // owner 在 inner Drop 时也会 abort reader task,process 在 Drop 时 kill。
        self.handle.close();
    }
}

/// 把 driver manifest 翻译成 `SpawnConfig`。
fn build_spawn_config(
    driver: &IpcDriverManifest,
    connection_config: Option<&DbConnectionConfig>,
) -> SpawnConfig {
    let program = PathBuf::from(command_for_current_platform(driver));
    let cwd = command_working_dir(driver);
    let socket_name = default_socket_name();

    let mut config = SpawnConfig::new(program)
        .with_args(driver.entry.args.clone())
        .with_cwd(cwd)
        .with_transport(SpawnTransport::LocalSocket { name: socket_name });
    config = config.with_ready_timeout(Duration::from_millis(
        driver.transport.connect_timeout_ms().max(1_000),
    ));
    if let Some(connection_config) = connection_config {
        for (env_key, config_path) in &driver.entry.env_from_config {
            if let Some(value) = config_value(connection_config, config_path) {
                if !value.trim().is_empty() {
                    config = config.with_env(env_key.clone(), value);
                }
            }
        }
    }
    config
}

fn command_for_current_platform(driver: &IpcDriverManifest) -> &str {
    if cfg!(windows) {
        command_for_platform(driver, "windows")
    } else {
        command_for_platform(driver, "default")
    }
}

fn command_for_platform<'a>(driver: &'a IpcDriverManifest, platform: &str) -> &'a str {
    driver
        .entry
        .commands
        .get(platform)
        .or_else(|| driver.entry.commands.get("default"))
        .map(String::as_str)
        .unwrap_or(driver.entry.command.as_str())
}

fn config_value(config: &DbConnectionConfig, path: &str) -> Option<String> {
    match path {
        "id" => Some(config.id.clone()),
        "name" => Some(config.name.clone()),
        "host" => Some(config.host.clone()),
        "port" => Some(config.port.to_string()),
        "username" => Some(config.username.clone()),
        "password" => Some(config.password.clone()),
        "database" => config.database.clone(),
        "service_name" => config.service_name.clone(),
        "sid" => config.sid.clone(),
        "database_type" => Some(config.database_type.as_str().to_string()),
        path => path
            .strip_prefix("extra_params.")
            .and_then(|key| config.extra_params.get(key).cloned()),
    }
}

/// 解析 driver 的 working_dir;空 / 不存在则用 manifest 目录。
fn command_working_dir(driver: &IpcDriverManifest) -> PathBuf {
    match driver.entry.working_dir.as_deref() {
        Some(wd) if !wd.is_empty() => {
            let candidate = Path::new(wd);
            if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                driver.manifest_dir.join(candidate)
            }
        }
        _ => {
            if driver.manifest_dir.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                driver.manifest_dir.clone()
            }
        }
    }
}

/// 构造 init 握手参数。
fn build_negotiation(driver: &IpcDriverManifest) -> NegotiationConfig {
    let host_version = env!("CARGO_PKG_VERSION").to_string();
    let instance_id = uuid::Uuid::new_v4().to_string();
    let config = NegotiationConfig::new(host_version, instance_id)
        .offer_api("database", "1.0")
        // 宿主默认接受丰富错误,但不强制要求;driver 若没声明就只会给文本 message。
        .with_handshake_timeout(Duration::from_millis(
            driver.transport.connect_timeout_ms().max(5_000),
        ));
    // 暂不 require_capability;由 plugin 层在调用具体方法时 fallback。
    config
}

/// 把 [`HostError`] 翻译成 [`DbError`],尽量保留 protocol error 的语义。
pub(crate) fn host_error_to_db_error(error: HostError) -> DbError {
    match error {
        HostError::Io(io) => DbError::connection_with_source("external driver io error", io),
        HostError::Serde(serde) => {
            DbError::query_with_source("invalid external driver json", serde)
        }
        HostError::Protocol(pe) => {
            use extension_protocol::error::error_codes;
            let message = pe.message.clone();
            if pe.code == error_codes::METHOD_NOT_FOUND {
                DbError::NotSupported(message)
            } else if pe.is_connection_error() {
                DbError::connection(format!("external driver error: {message}"))
            } else if pe.is_sql_error() {
                DbError::query(format!("external driver sql error: {message}"))
            } else {
                DbError::query(format!(
                    "external driver error (code {}): {message}",
                    pe.code
                ))
            }
        }
        HostError::Timeout { method, timeout_ms } => DbError::query(format!(
            "external driver request `{method}` timed out after {timeout_ms}ms"
        )),
        HostError::Cancelled { method } => {
            DbError::query(format!("external driver request `{method}` was cancelled"))
        }
        HostError::Closed | HostError::NotInitialized => DbError::NotConnected,
        HostError::ProcessExited(reason) => {
            DbError::connection(format!("external driver process exited: {reason}"))
        }
        HostError::Config(reason) => {
            DbError::connection(format!("external driver spawn config invalid: {reason}"))
        }
        HostError::ProcessNotReady {
            deadline_ms,
            stderr_tail,
        } => {
            let mut message =
                format!("external driver did not become ready within {deadline_ms}ms");
            if let Some(stderr_tail) = stderr_tail.filter(|tail| !tail.trim().is_empty()) {
                message.push_str("\nrecent stderr:\n");
                message.push_str(&stderr_tail);
            }
            DbError::connection(message)
        }
        HostError::Incompatible(reason) => {
            DbError::connection(format!("external driver compatibility error: {reason}"))
        }
        HostError::NotImplemented(msg) => DbError::NotSupported(msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::registry::{IpcDriverEntry, IpcDriverTransport};
    use extension_protocol::error::{ProtocolError, error_codes};

    fn dummy_manifest(command: &str, working_dir: Option<&str>) -> IpcDriverManifest {
        IpcDriverManifest {
            id: "dummy".into(),
            name: "Dummy".into(),
            category: None,
            description: String::new(),
            version: String::new(),
            entry: IpcDriverEntry {
                command: command.into(),
                commands: Default::default(),
                args: Vec::new(),
                working_dir: working_dir.map(str::to_string),
                env_from_config: Default::default(),
            },
            transport: IpcDriverTransport::local_socket("dummy.sock"),
            dialect: Default::default(),
            capabilities: Default::default(),
            connection: Default::default(),
            methods: Vec::new(),
            ui: Default::default(),
            manifest_dir: PathBuf::from("/tmp/onetcli-test"),
        }
    }

    #[test]
    fn spawn_config_uses_manifest_dir_as_cwd() {
        let manifest = dummy_manifest("/usr/bin/true", None);
        let cfg = build_spawn_config(&manifest, None);
        assert!(cfg.env.is_empty());
        assert_eq!(cfg.cwd.as_deref(), Some(Path::new("/tmp/onetcli-test")));
    }

    #[test]
    fn spawn_config_maps_manifest_env_from_connection_config() {
        let mut manifest = dummy_manifest("/usr/bin/true", None);
        manifest
            .entry
            .env_from_config
            .insert("GBASE8S_JDK_HOME".into(), "extra_params.jdk_home".into());
        manifest
            .entry
            .env_from_config
            .insert("DB_HOST".into(), "host".into());
        let mut config = DbConnectionConfig {
            id: "conn-1".into(),
            database_type: one_core::storage::DatabaseType::external("gbase8s"),
            name: "GBase".into(),
            host: "127.0.0.1".into(),
            port: 11811,
            username: "gbasedbt".into(),
            password: "secret".into(),
            database: Some("stores".into()),
            service_name: None,
            sid: None,
            workspace_id: None,
            extra_params: Default::default(),
        };
        config
            .extra_params
            .insert("jdk_home".into(), "/opt/jdk-8".into());

        let cfg = build_spawn_config(&manifest, Some(&config));

        assert_eq!(
            cfg.env.get("GBASE8S_JDK_HOME"),
            Some(&"/opt/jdk-8".to_string())
        );
        assert_eq!(cfg.env.get("DB_HOST"), Some(&"127.0.0.1".to_string()));
    }

    #[test]
    fn command_for_platform_uses_platform_specific_entry_command() {
        let mut manifest = dummy_manifest("./driver", None);
        manifest
            .entry
            .commands
            .insert("default".into(), "./driver".into());
        manifest
            .entry
            .commands
            .insert("windows".into(), "./driver.cmd".into());

        assert_eq!(command_for_platform(&manifest, "windows"), "./driver.cmd");
        assert_eq!(command_for_platform(&manifest, "linux"), "./driver");
    }

    #[test]
    fn working_dir_falls_back_to_manifest_dir() {
        let manifest = dummy_manifest("/usr/bin/true", None);
        let wd = command_working_dir(&manifest);
        assert_eq!(wd, PathBuf::from("/tmp/onetcli-test"));
    }

    #[test]
    fn working_dir_resolves_relative_against_manifest_dir() {
        let manifest = dummy_manifest("/usr/bin/true", Some("bin"));
        let wd = command_working_dir(&manifest);
        assert_eq!(wd, PathBuf::from("/tmp/onetcli-test/bin"));
    }

    #[test]
    fn working_dir_uses_absolute_path_as_is() {
        let manifest = dummy_manifest("/usr/bin/true", Some("/opt/driver"));
        let wd = command_working_dir(&manifest);
        assert_eq!(wd, PathBuf::from("/opt/driver"));
    }

    #[test]
    fn method_not_found_maps_to_not_supported() {
        let err = host_error_to_db_error(HostError::Protocol(Box::new(ProtocolError::new(
            error_codes::METHOD_NOT_FOUND,
            "missing",
        ))));
        assert!(matches!(err, DbError::NotSupported(message) if message == "missing"));
    }

    #[test]
    fn closed_maps_to_not_connected() {
        let err = host_error_to_db_error(HostError::Closed);
        assert!(matches!(err, DbError::NotConnected));
    }

    #[test]
    fn timeout_maps_to_query_error() {
        let err = host_error_to_db_error(HostError::Timeout {
            method: "query/start".into(),
            timeout_ms: 1_000,
        });
        match err {
            DbError::Query { message, .. } => {
                assert!(message.contains("query/start"));
                assert!(message.contains("1000"));
            }
            other => panic!("expected Query, got {other:?}"),
        }
    }

    #[test]
    fn process_not_ready_maps_stderr_tail_to_connection_error() {
        let err = host_error_to_db_error(HostError::ProcessNotReady {
            deadline_ms: 500,
            stderr_tail: Some("driver boot failed".into()),
        });

        match err {
            DbError::Connection { message, .. } => {
                assert!(message.contains("500"));
                assert!(message.contains("driver boot failed"));
            }
            other => panic!("expected Connection, got {other:?}"),
        }
    }

    #[test]
    fn sql_error_maps_to_query_error() {
        let pe = ProtocolError::new(error_codes::SQL_SYNTAX_ERROR, "bad sql");
        let err = host_error_to_db_error(HostError::Protocol(Box::new(pe)));
        match err {
            DbError::Query { message, .. } => assert!(message.contains("bad sql")),
            other => panic!("expected Query, got {other:?}"),
        }
    }

    #[test]
    fn connection_refused_maps_to_connection_error() {
        let pe = ProtocolError::new(error_codes::IO_CONNECTION_REFUSED, "refused");
        let err = host_error_to_db_error(HostError::Protocol(Box::new(pe)));
        assert!(matches!(err, DbError::Connection { .. }));
    }

    #[tokio::test]
    async fn start_rejects_empty_command() {
        let manifest = dummy_manifest("", None);
        let result = JsonRpcClient::start(&manifest).await;
        assert!(matches!(result, Err(DbError::Connection { .. })));
    }
}
