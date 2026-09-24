use crate::cache::CacheContext;
use crate::cache_manager::{GlobalNodeCache, SchemaInvalidationPlan, SqlInvalidationContext};
use crate::clickhouse::ClickHousePlugin;
use crate::connection::{DbConnection, DbError, StreamingProgress};
use crate::connection_config_resolver::ConnectionConfigResolver;
#[cfg(feature = "builtin-duckdb")]
use crate::duckdb::DuckDbPlugin;
use crate::import_export::{
    ExportConfig, ExportProgressRequest, ExportResult, ImportConfig, ImportProgressRequest,
    ImportResult,
};
use crate::ipc::{ExternalDatabasePlugin, IpcDriverRegistry};
use crate::mssql::MsSqlPlugin;
use crate::mysql::MySqlPlugin;
use crate::oracle::OraclePlugin;
use crate::plugin::DatabasePlugin;
use crate::plugin_manifest::DatabaseCapabilities;
use crate::postgresql::PostgresPlugin;
use crate::runtime_contract::require_tokio_runtime;
use crate::sqlite::SqlitePlugin;
use crate::{
    DbNode, DbNodeType, ExecOptions, SqlErrorInfo, SqlResult, SqlSource, TableDesign,
    TableObjectType, TableSaveResponse,
};
use dashmap::DashMap;
use gpui::{AppContext, AsyncApp, Global, Task};
use one_core::connection_notifier::{ConnectionDataEvent, GlobalConnectionNotifier};
use one_core::gpui_tokio::Tokio;
use one_core::storage::{ConnectionRepository, DatabaseType, DbConnectionConfig};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex};

type ExternalRegistryReloader = dyn Fn() -> IpcDriverRegistry + Send + Sync;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, RwLock, mpsc};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

const BUSY_CLOSE_ON_RELEASE_RETRY_DELAY: Duration = Duration::from_millis(10);

/// Macro to reduce boilerplate for plugin operations with session management
macro_rules! with_plugin_session {
    ($self:expr, $cx:expr, $connection_id:expr, |$plugin:ident, $conn:ident| $body:expr) => {{
        let config = $self.get_config(&$connection_id);
        if config.is_none() {
            error!(
                "with_plugin_session: Connection not found: {}",
                $connection_id
            );
        }
        let config =
            config.ok_or_else(|| anyhow::anyhow!("Connection not found: {}", $connection_id))?;

        let clone_self = $self.clone();
        Tokio::spawn_result($cx, async move {
            let $plugin = clone_self.get_plugin(&config.database_type)?;
            info!(
                "with_plugin_session: creating session for config_id={}",
                config.id
            );
            let session_id = clone_self
                .connection_manager
                .create_session(config.clone(), &clone_self.db_manager)
                .await?;
            info!("with_plugin_session: session created: {}", session_id);

            let result = {
                let mut guard = clone_self
                    .connection_manager
                    .get_session_connection(&session_id)
                    .await?;
                let $conn = guard
                    .connection()
                    .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
                $body.map_err(|e| anyhow::anyhow!("{}", e))
            };

            clone_self
                .connection_manager
                .release_session(&session_id)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            result
        })
        .await
    }};
}

/// Macro with database parameter for PostgreSQL and other databases that require connection-level database selection
macro_rules! with_plugin_session_db {
    ($self:expr, $cx:expr, $connection_id:expr, $database:expr, |$plugin:ident, $conn:ident| $body:expr) => {{
        let config = $self.get_config(&$connection_id);
        if config.is_none() {
            error!(
                "with_plugin_session_db: Connection not found: {}",
                $connection_id
            );
        }
        let mut config = config
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", $connection_id))?
            .clone();
        config.database = Some($database.to_string());

        let clone_self = $self.clone();
        Tokio::spawn_result($cx, async move {
            let $plugin = clone_self.get_plugin(&config.database_type)?;
            let session_id = clone_self
                .connection_manager
                .create_session(config, &clone_self.db_manager)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            let result = {
                let mut guard = clone_self
                    .connection_manager
                    .get_session_connection(&session_id)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                let $conn = guard
                    .connection()
                    .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
                $body.map_err(|e| anyhow::anyhow!("{}", e))
            };

            clone_self
                .connection_manager
                .release_session(&session_id)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            result
        })
        .await
    }};
}

async fn cached_foreign_keys(
    cx: &mut AsyncApp,
    cache: GlobalNodeCache,
    connection_id: &str,
    database: &str,
    schema: Option<&str>,
    table: &str,
) -> anyhow::Result<Vec<crate::types::ForeignKeyDefinition>> {
    let conn_id = connection_id.to_string();
    let db = database.to_string();
    let sch = schema.map(str::to_string);
    let tbl = table.to_string();
    Tokio::spawn_result(cx, async move {
        cache
            .get_foreign_keys(&conn_id, &db, sch.as_deref(), &tbl)
            .await
            .ok_or_else(|| anyhow::anyhow!("Cache miss"))
    })
    .await
}

/// TDengine 经外部 IPC 驱动(driver_id "tdengine")提供实现,
/// 主仓不再内置原生插件;历史连接的 `DatabaseType::TDengine` 在
/// `get_plugin` 中路由到同一外部驱动解析路径。
const TDENGINE_IPC_DRIVER_ID: &str = "tdengine";

/// Database manager - creates database plugins
pub struct DbManager {
    mysql: Arc<dyn DatabasePlugin>,
    postgresql: Arc<dyn DatabasePlugin>,
    sqlite: Arc<dyn DatabasePlugin>,
    duckdb: Arc<dyn DatabasePlugin>,
    clickhouse: Arc<dyn DatabasePlugin>,
    mssql: Arc<dyn DatabasePlugin>,
    oracle: Arc<dyn DatabasePlugin>,
    external_drivers: HashMap<String, Arc<dyn DatabasePlugin>>,
    external_registry_reloader: Arc<ExternalRegistryReloader>,
}

impl DbManager {
    pub fn new() -> Self {
        Self::with_external_registry(IpcDriverRegistry::load_default())
    }

    fn with_external_registry(registry: IpcDriverRegistry) -> Self {
        Self::with_external_registry_reloader(registry, Arc::new(IpcDriverRegistry::load_default))
    }

    fn with_external_registry_reloader(
        registry: IpcDriverRegistry,
        external_registry_reloader: Arc<ExternalRegistryReloader>,
    ) -> Self {
        let external_drivers = registry
            .drivers()
            .iter()
            .map(|driver| {
                (
                    driver.id.clone(),
                    Arc::new(ExternalDatabasePlugin::for_driver(driver.clone()))
                        as Arc<dyn DatabasePlugin>,
                )
            })
            .collect();

        Self {
            mysql: Arc::new(MySqlPlugin::new()),
            postgresql: Arc::new(PostgresPlugin::new()),
            sqlite: Arc::new(SqlitePlugin::new()),
            duckdb: default_duckdb_plugin(&registry),
            clickhouse: Arc::new(ClickHousePlugin::new()),
            mssql: Arc::new(MsSqlPlugin::new()),
            oracle: Arc::new(OraclePlugin::new()),
            external_drivers,
            external_registry_reloader,
        }
    }

    pub fn get_plugin(&self, db_type: &DatabaseType) -> Result<Arc<dyn DatabasePlugin>, DbError> {
        match db_type {
            DatabaseType::MySQL => Ok(Arc::clone(&self.mysql)),
            DatabaseType::PostgreSQL => Ok(Arc::clone(&self.postgresql)),
            DatabaseType::SQLite => Ok(Arc::clone(&self.sqlite)),
            DatabaseType::DuckDB => Ok(Arc::clone(&self.duckdb)),
            DatabaseType::ClickHouse => Ok(Arc::clone(&self.clickhouse)),
            // TDengine 无原生插件,与 External 一致按 driver_id 解析 IPC 外部驱动。
            DatabaseType::TDengine => self.resolve_external_plugin(TDENGINE_IPC_DRIVER_ID),
            DatabaseType::MSSQL => Ok(Arc::clone(&self.mssql)),
            DatabaseType::Oracle => Ok(Arc::clone(&self.oracle)),
            DatabaseType::External { driver_id } => self.resolve_external_plugin(driver_id),
        }
    }

    /// 按 driver_id 解析外部 IPC 驱动插件:优先实时重载注册表,回退构造时快照。
    fn resolve_external_plugin(&self, driver_id: &str) -> Result<Arc<dyn DatabasePlugin>, DbError> {
        if let Some(driver) = (self.external_registry_reloader)().find(driver_id) {
            return Ok(
                Arc::new(ExternalDatabasePlugin::for_driver(driver)) as Arc<dyn DatabasePlugin>
            );
        }
        if let Some(plugin) = self.external_drivers.get(driver_id).cloned() {
            return Ok(plugin);
        }
        Err(DbError::connection(format!(
            "external driver '{driver_id}' not found"
        )))
    }
}

#[cfg(feature = "builtin-duckdb")]
fn default_duckdb_plugin(_registry: &IpcDriverRegistry) -> Arc<dyn DatabasePlugin> {
    Arc::new(DuckDbPlugin::new())
}

#[cfg(not(feature = "builtin-duckdb"))]
fn default_duckdb_plugin(registry: &IpcDriverRegistry) -> Arc<dyn DatabasePlugin> {
    Arc::new(ExternalDatabasePlugin::with_registry_reloader(
        registry.clone(),
        Arc::new(IpcDriverRegistry::load_default),
    ))
}

impl Default for DbManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for DbManager {
    fn clone(&self) -> Self {
        Self {
            mysql: Arc::clone(&self.mysql),
            postgresql: Arc::clone(&self.postgresql),
            sqlite: Arc::clone(&self.sqlite),
            duckdb: Arc::clone(&self.duckdb),
            clickhouse: Arc::clone(&self.clickhouse),
            mssql: Arc::clone(&self.mssql),
            oracle: Arc::clone(&self.oracle),
            external_drivers: self.external_drivers.clone(),
            external_registry_reloader: Arc::clone(&self.external_registry_reloader),
        }
    }
}

/// Connection session - represents a single database connection
///
/// The physical connection lives behind its own mutex, so a running statement only
/// locks the connection it uses. Bookkeeping lives in [`SessionState`] instead of the
/// session itself so status reads never wait for an in-flight statement. Reuse checks
/// read the cached [`DatabaseIdentity`] for the same reason: picking a candidate must
/// not wait for a session that is busy with a statement.
struct ConnectionSession {
    connection: Arc<AsyncMutex<Box<dyn DbConnection + Send + Sync>>>,
    close_on_release: bool,
    created_at: Instant,
    session_id: String,
    state: StdMutex<SessionState>,
    identity: StdMutex<DatabaseIdentity>,
}

/// Mutable per-session bookkeeping kept separate from the connection lock.
struct SessionState {
    /// Statements currently holding this session.
    ///
    /// A bool would lose a claim: a release that finishes after the next statement
    /// already took the session would mark a busy session idle, and the cleanup task
    /// could then recycle the connection of a running statement.
    in_flight: usize,
    /// Set when a session is handed to a caller that has not taken the connection yet
    /// (`create_session`). The matching `get_session_connection` consumes it, so one
    /// statement is never counted twice while the hand-off stays protected against
    /// concurrent reuse.
    reserved: bool,
    last_active: Instant,
}

/// Identity fields that decide whether a session can serve a request.
///
/// Cached on the session so the pool can pick a reuse candidate without waiting for
/// the connection lock. `verify_and_sync_database` is the only place that rewrites
/// `DbConnectionConfig::database`, and it refreshes this snapshot, so the cache cannot
/// silently drift from the live connection.
struct DatabaseIdentity {
    database_type: DatabaseType,
    database: Option<String>,
    sid: Option<String>,
    service_name: Option<String>,
}

impl DatabaseIdentity {
    fn from_config(config: &DbConnectionConfig) -> Self {
        Self {
            database_type: config.database_type.clone(),
            database: config.database.clone(),
            sid: config.sid.clone(),
            service_name: config.service_name.clone(),
        }
    }

    /// Whether a session with this identity can serve `config`.
    fn matches(&self, config: &DbConnectionConfig) -> bool {
        match self.database_type {
            DatabaseType::Oracle => {
                (self.sid.is_some() && self.sid == config.sid)
                    || (self.service_name.is_some() && self.service_name == config.service_name)
            }
            _ => self.database.is_some() && self.database == config.database,
        }
    }
}

/// 空闲期被中间设备丢弃的 TCP 连接不会回包：任何验证/关闭网络调用都必须有界，
/// 否则复用或退出路径会永久挂起（issue：长时间挂机后查询转圈、关不掉）。
pub(crate) mod network_deadline {
    use std::future::Future;
    use std::time::Duration;

    use crate::connection::DbError;

    /// 复用前 ping 的上限；超时即判死，直接丢弃会话。
    pub(crate) const VALIDATE_PING_TIMEOUT: Duration = Duration::from_secs(10);
    /// 关闭连接的上限；超时放弃优雅断开，等 socket drop 强制回收。
    pub(crate) const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(5);

    /// 包装一次空闲期网络调用，超时映射为连接错误而不是永久挂起。
    pub(crate) async fn bound(
        what: &'static str,
        timeout: Duration,
        fut: impl Future<Output = Result<(), DbError>>,
    ) -> Result<(), DbError> {
        match tokio::time::timeout(timeout, fut).await {
            Ok(result) => result,
            Err(_) => Err(DbError::connection(format!(
                "{what} timed out after {}s (connection likely dropped while idle)",
                timeout.as_secs()
            ))),
        }
    }
}

impl ConnectionSession {
    fn new(
        connection: Box<dyn DbConnection + Send + Sync>,
        session_id: String,
        close_on_release: bool,
    ) -> Self {
        let identity = DatabaseIdentity::from_config(connection.config());
        let now = Instant::now();
        Self {
            connection: Arc::new(AsyncMutex::new(connection)),
            close_on_release,
            created_at: now,
            session_id,
            state: StdMutex::new(SessionState {
                in_flight: 0,
                reserved: false,
                last_active: now,
            }),
            identity: StdMutex::new(identity),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, SessionState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Cached identity used to pick reuse candidates without the connection lock.
    fn identity(&self) -> std::sync::MutexGuard<'_, DatabaseIdentity> {
        self.identity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Refresh the cached identity from the live connection config.
    fn refresh_identity(&self, connection: &(dyn DbConnection + Send + Sync)) {
        *self.identity() = DatabaseIdentity::from_config(connection.config());
    }

    /// Whether the cached identity says this session can serve `config`.
    fn can_serve(&self, config: &DbConnectionConfig) -> bool {
        self.identity().matches(config)
    }

    /// Hand the session to a statement that is about to take its connection.
    fn reserve(&self) {
        let mut state = self.state();
        state.reserved = true;
        state.last_active = Instant::now();
    }

    /// Claim the session for the statement that is taking its connection now.
    ///
    /// Consumes a reservation made by [`Self::reserve`], so create_session and the
    /// following statement do not count the same use twice.
    fn mark_in_use(&self) {
        let mut state = self.state();
        state.reserved = false;
        state.in_flight += 1;
        state.last_active = Instant::now();
    }

    /// Release one claim (or undo one reservation).
    fn release(&self) {
        let mut state = self.state();
        state.reserved = false;
        state.in_flight = state.in_flight.saturating_sub(1);
        state.last_active = Instant::now();
    }

    fn in_use(&self) -> bool {
        let state = self.state();
        state.in_flight > 0 || state.reserved
    }

    fn last_active(&self) -> Instant {
        self.state().last_active
    }

    /// Idle long enough to be recycled.
    ///
    /// Ignores `reserved` on purpose: a reservation that is never consumed (its caller
    /// died in between) must not pin the session forever. A live reservation is young,
    /// so it is still protected by the timeout.
    fn is_expired(&self, timeout: Duration) -> bool {
        let state = self.state();
        state.in_flight == 0 && state.last_active.elapsed() > timeout
    }

    fn is_lifetime_expired(&self, max_lifetime: Duration) -> bool {
        self.created_at.elapsed() > max_lifetime
    }

    /// Lock the physical connection for one exclusive statement.
    async fn lock_connection(&self) -> OwnedMutexGuard<Box<dyn DbConnection + Send + Sync>> {
        Arc::clone(&self.connection).lock_owned().await
    }

    /// Validate that this session can serve `config` before it is reused.
    ///
    /// Takes the connection lock, so the caller must not hold the session table lock.
    /// Returns `Ok(false)` when the live database no longer matches `config`, in which
    /// case the refreshed identity is kept so the next loop can pick another candidate.
    ///
    /// 空闲期死连接的 ping 永远不回包，必须按超时判死，由调用方丢弃会话。
    async fn validate_reuse(&self, config: &DbConnectionConfig) -> Result<bool, DbError> {
        let connection = self.lock_connection().await;
        self.refresh_identity(&**connection);
        if !self.can_serve(config) {
            return Ok(false);
        }
        network_deadline::bound(
            "reuse ping",
            network_deadline::VALIDATE_PING_TIMEOUT,
            connection.ping(),
        )
        .await?;
        Ok(true)
    }

    /// Check if current database matches config database
    /// Returns Ok(true) if consistent, Ok(false) if updated config, Err if check failed
    async fn verify_and_sync_database(&self) -> Result<bool, DbError> {
        let mut connection = self.lock_connection().await;
        // Skip check for databases that don't support switching
        if !connection.supports_database_switch() {
            return Ok(true);
        }

        let config_db = connection.config().database.clone();
        let current_db = connection.current_database().await?;

        if config_db == current_db {
            Ok(true)
        } else {
            // Database changed: update the config together with the cached identity
            connection.set_config_database(current_db.clone());
            self.refresh_identity(&**connection);
            info!(
                "Session {} database changed: {:?} -> {:?}",
                self.session_id, config_db, current_db
            );
            Ok(false)
        }
    }

    async fn close(&self) {
        let mut connection = self.lock_connection().await;
        let disconnect = connection.disconnect();
        let outcome =
            tokio::time::timeout(network_deadline::DISCONNECT_TIMEOUT, disconnect).await;
        match outcome {
            Ok(Ok(())) => info!("Closed session: {}", self.session_id),
            Ok(Err(e)) => error!("Failed to disconnect session {}: {}", self.session_id, e),
            // 死连接的优雅断开不会回包：放弃等待，锁随 guard drop 释放，
            // socket 由驱动在 drop 时强制回收。
            Err(_) => warn!(
                "Disconnect of session {} timed out after {}s; abandoning graceful close",
                self.session_id,
                network_deadline::DISCONNECT_TIMEOUT.as_secs()
            ),
        }
    }
}

/// Connection manager - manages database connections for a client application
pub struct ConnectionManager {
    /// config_id -> list of sessions for that config
    ///
    /// The map lock only guards the pool itself. A running statement locks its own
    /// session's connection instead, so sessions never serialize each other.
    sessions: Arc<RwLock<HashMap<String, Vec<Arc<ConnectionSession>>>>>,
    /// Per file-backed database lock used to serialize physical opens before a session is visible.
    physical_open_locks: Arc<AsyncMutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
    /// Converts stored connection configs into runtime configs before opening sessions.
    config_resolver: ConnectionConfigResolver,
    /// Connection idle timeout (default: 5 minutes)
    idle_timeout: Duration,
    /// Maximum connection lifetime (default: 30 minutes)
    max_lifetime: Duration,
    /// Session counter for generating unique IDs
    session_counter: Arc<tokio::sync::Mutex<u64>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self::with_config_resolver(ConnectionConfigResolver::default())
    }

    pub fn with_config_resolver(config_resolver: ConnectionConfigResolver) -> Self {
        Self::with_config_and_resolver(
            Duration::from_secs(300),
            Duration::from_secs(1800),
            config_resolver,
        )
    }

    pub fn with_config(idle_timeout: Duration, max_lifetime: Duration) -> Self {
        Self::with_config_and_resolver(
            idle_timeout,
            max_lifetime,
            ConnectionConfigResolver::default(),
        )
    }

    fn with_config_and_resolver(
        idle_timeout: Duration,
        max_lifetime: Duration,
        config_resolver: ConnectionConfigResolver,
    ) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            physical_open_locks: Arc::new(AsyncMutex::new(HashMap::new())),
            config_resolver,
            idle_timeout,
            max_lifetime,
            session_counter: Arc::new(tokio::sync::Mutex::new(0)),
        }
    }

    /// Generate unique session ID
    async fn generate_session_id(&self, config_id: &str) -> String {
        let mut counter = self.session_counter.lock().await;
        *counter += 1;
        format!("{}:session:{}", config_id, *counter)
    }

    /// Create a new connection session
    pub async fn create_session(
        &self,
        config: DbConnectionConfig,
        db_manager: &DbManager,
    ) -> Result<String, DbError> {
        let session_started = Instant::now();
        let config = self.config_resolver.resolve(config)?;
        let plugin = db_manager.get_plugin(&config.database_type)?;
        let lifecycle = plugin.connection_lifecycle(&config);
        let _physical_open_guard = self
            .physical_open_guard(lifecycle.physical_open_lock_key.as_deref())
            .await;
        let config_id = config.id.clone();
        let database_type = config.database_type.clone();
        let database = config.database.clone();

        // Try to acquire an existing session and switch database if needed
        if let Some(session_id) = self.try_acquire_session(&config).await? {
            debug!(
                "[DB][Timing] create_session reused config_id={} database_type={:?} database={:?} session_id={} elapsed={}ms",
                config_id,
                database_type,
                database,
                session_id,
                session_started.elapsed().as_millis()
            );
            return Ok(session_id);
        }

        let session_id = self.generate_session_id(&config_id).await;

        // Create new connection
        let connect_started = Instant::now();
        let connection = plugin.create_connection(config.clone()).await?;
        info!(
            "[DB][Timing] create_session connect config_id={} database_type={:?} database={:?} session_id={} elapsed={}ms",
            config_id,
            database_type,
            database,
            session_id,
            connect_started.elapsed().as_millis()
        );
        info!(
            "Created new session: {} (database: {:?})",
            session_id, config.database
        );

        // Store session, reserved for the caller that is about to take its connection.
        // The matching `get_session_connection` consumes the reservation, so every
        // statement adds exactly one claim and its release brings the counter back to
        // zero, while two concurrent create_session calls never share a session.
        let session =
            ConnectionSession::new(connection, session_id.clone(), lifecycle.close_on_release);
        session.reserve();

        let mut sessions = self.sessions.write().await;
        sessions
            .entry(config_id)
            .or_insert_with(Vec::new)
            .push(Arc::new(session));

        info!(
            "[DB][Timing] create_session total database_type={:?} database={:?} session_id={} elapsed={}ms",
            database_type,
            database,
            session_id,
            session_started.elapsed().as_millis()
        );
        Ok(session_id)
    }

    async fn physical_open_guard(&self, key: Option<&str>) -> Option<OwnedMutexGuard<()>> {
        let key = key?;
        let lock = {
            let mut locks = self.physical_open_locks.lock().await;
            Arc::clone(
                locks
                    .entry(key.to_string())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        };

        Some(lock.lock_owned().await)
    }

    /// Get exclusive access to a session's connection for one statement
    ///
    /// The pool lock is only held long enough to locate the session and claim it;
    /// the returned guard holds that session's connection lock, so another tab on
    /// another connection keeps running in parallel.
    pub async fn get_session_connection(
        &self,
        session_id: &str,
    ) -> Result<SessionConnectionGuard, DbError> {
        let session = {
            let sessions = self.sessions.read().await;
            let session = sessions
                .values()
                .flat_map(|list| list.iter())
                .find(|session| session.session_id == session_id)
                .cloned()
                .ok_or_else(|| DbError::Internal(format!("session not found: {}", session_id)))?;
            // Claim it before dropping the pool lock so cleanup cannot close it underneath.
            session.mark_in_use();
            session
        };

        // The session may have been detached (and its connection closed) while we queued
        // behind a running statement; never hand out a connection the pool lost.
        //
        // This nests the connection lock and the session table lock. It cannot deadlock:
        // no path holds the session table lock while awaiting a connection lock
        // (`try_acquire_session` releases it before validating a candidate).
        let connection = session.lock_connection().await;
        if !self.is_pooled(session_id).await {
            session.release();
            return Err(DbError::Internal(format!(
                "session not found: {}",
                session_id
            )));
        }

        Ok(SessionConnectionGuard { connection })
    }

    /// Clone the shared handle of one session without holding the pool lock.
    async fn find_session(&self, session_id: &str) -> Option<Arc<ConnectionSession>> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .flat_map(|list| list.iter())
            .find(|session| session.session_id == session_id)
            .cloned()
    }

    /// Remove one session from the pool so no new statement can claim it.
    ///
    /// The caller owns closing the returned session outside the pool lock.
    async fn detach_session(&self, session_id: &str) -> Option<Arc<ConnectionSession>> {
        let mut sessions = self.sessions.write().await;
        let mut detached = None;
        for session_list in sessions.values_mut() {
            if let Some(index) = session_list
                .iter()
                .position(|session| session.session_id == session_id)
            {
                detached = Some(session_list.remove(index));
                break;
            }
        }
        if detached.is_some() {
            sessions.retain(|_, session_list| !session_list.is_empty());
        }
        detached
    }

    /// Try to acquire an existing idle session with matching database
    ///
    /// The session table lock is held only to pick and claim a candidate: every wait —
    /// the live database check and the ping — happens after it is released. A session
    /// busy with a long statement therefore never stalls the pool for other connections.
    async fn try_acquire_session(
        &self,
        config: &DbConnectionConfig,
    ) -> Result<Option<String>, DbError> {
        // Candidates already rejected by the out-of-lock checks below.
        let mut rejected: HashSet<String> = HashSet::new();

        loop {
            let (candidate, waiting_for_busy_session) = {
                let mut sessions = self.sessions.write().await;
                let Some(session_list) = sessions.get_mut(&config.id) else {
                    return Ok(None);
                };

                let mut candidate: Option<Arc<ConnectionSession>> = None;
                let mut waiting_for_busy_session = false;

                for session in session_list.iter() {
                    if rejected.contains(&session.session_id) || !session.can_serve(config) {
                        continue;
                    }
                    if session.in_use() {
                        waiting_for_busy_session |= session.close_on_release;
                        continue;
                    }
                    candidate = Some(Arc::clone(session));
                    break;
                }

                if let Some(session) = &candidate {
                    session.reserve();
                }

                let empty = session_list.is_empty();
                if empty {
                    sessions.remove(&config.id);
                }

                (candidate, waiting_for_busy_session)
            };

            let Some(session) = candidate else {
                if waiting_for_busy_session {
                    sleep(BUSY_CLOSE_ON_RELEASE_RETRY_DELAY).await;
                    continue;
                }
                return Ok(None);
            };

            let session_id = session.session_id.clone();

            match session.validate_reuse(config).await {
                Ok(true) => {}
                Ok(false) => {
                    // The live database drifted from the request; the refreshed identity
                    // stays cached, so look for another candidate.
                    session.release();
                    rejected.insert(session_id);
                    continue;
                }
                Err(error) => {
                    warn!(
                        "Discarding stale session {} before reuse: {}",
                        session_id, error
                    );
                    session.release();
                    self.detach_and_close(&session_id).await;
                    continue;
                }
            }

            if !self.is_pooled(&session_id).await {
                // Removed while we were waiting for the connection; never hand it out.
                session.release();
                continue;
            }

            debug!(
                "Reusing session: {} (database: {:?})",
                session_id, config.database
            );
            return Ok(Some(session_id));
        }
    }

    /// Whether a session is still part of the pool.
    ///
    /// Borrows the session table only, so a caller holding a connection lock can use it
    /// to re-check ownership before handing the connection out.
    async fn is_pooled(&self, session_id: &str) -> bool {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .any(|list| list.iter().any(|session| session.session_id == session_id))
    }

    /// Detach a session from the pool and close it.
    ///
    /// Returns `false` when another task detached it first: that task owns the close, so
    /// the physical connection is disconnected exactly once.
    async fn detach_and_close(&self, session_id: &str) -> bool {
        let Some(session) = self.detach_session(session_id).await else {
            return false;
        };
        session.close().await;
        true
    }
}

/// Guard that locks one session's connection for the duration of a statement
pub struct SessionConnectionGuard {
    connection: OwnedMutexGuard<Box<dyn DbConnection + Send + Sync>>,
}

impl SessionConnectionGuard {
    /// Get mutable reference to the connection
    pub fn connection(&mut self) -> Option<&mut (dyn DbConnection + Send + Sync)> {
        Some(&mut **self.connection)
    }
}

impl ConnectionManager {
    /// Get session config
    pub async fn get_session_config(&self, session_id: &str) -> Option<DbConnectionConfig> {
        let session = self.find_session(session_id).await?;
        Some(session.lock_connection().await.config().clone())
    }

    pub async fn release_session(&self, session_id: &str) -> Result<(), DbError> {
        self.release_session_internal(session_id, true).await
    }

    async fn release_session_for_reuse(&self, session_id: &str) -> Result<(), DbError> {
        self.release_session_internal(session_id, false).await
    }

    async fn release_session_internal(
        &self,
        session_id: &str,
        close_idle_file_connection: bool,
    ) -> Result<(), DbError> {
        let Some(session) = self.find_session(session_id).await else {
            return Err(DbError::Internal(format!(
                "session not found: {}",
                session_id
            )));
        };

        let should_close = match session.verify_and_sync_database().await {
            Ok(_) => close_idle_file_connection && session.close_on_release,
            Err(e) => {
                // Check failed, mark for closing
                warn!(
                    "Session {} database check failed: {}, closing connection",
                    session_id, e
                );
                true
            }
        };

        let detached = should_close && self.detach_session(session_id).await.is_some();

        if detached {
            // This task removed the session from the pool, so it owns the disconnect.
            session.release();
            session.close().await;
            return Ok(());
        }

        session.release();
        if should_close {
            debug!("Session {} was already detached", session_id);
        } else {
            debug!("Session {} released", session_id);
        }
        Ok(())
    }

    /// Close a specific session
    pub async fn close_session(&self, session_id: &str) -> Result<(), DbError> {
        let Some(session) = self.detach_session(session_id).await else {
            return Err(DbError::Internal(format!(
                "session not found: {}",
                session_id
            )));
        };

        session.release();
        session.close().await;
        Ok(())
    }

    /// Remove all sessions for a connection config
    pub async fn remove_all_sessions(&self, config_id: &str) {
        let session_list = {
            let mut sessions = self.sessions.write().await;
            sessions.remove(config_id)
        };

        let Some(session_list) = session_list else {
            return;
        };

        info!(
            "Closing {} sessions for config: {}",
            session_list.len(),
            config_id
        );

        for session in session_list.iter() {
            session.close().await;
        }
    }

    /// Clean up expired sessions
    async fn cleanup_expired_sessions(&self) {
        let idle_timeout = self.idle_timeout;
        let max_lifetime = self.max_lifetime;

        let expired = {
            let mut sessions = self.sessions.write().await;
            let mut expired = Vec::new();

            for (config_id, session_list) in sessions.iter_mut() {
                let mut i = 0;
                while i < session_list.len() {
                    let session = &session_list[i];
                    let should_remove = session.is_expired(idle_timeout)
                        || session.is_lifetime_expired(max_lifetime);

                    if should_remove {
                        let session = session_list.remove(i);
                        warn!(
                            "Closing expired session {} for config {} (in_use: {}, idle: {}s, lifetime: {}s)",
                            session.session_id,
                            config_id,
                            session.in_use(),
                            session.last_active().elapsed().as_secs(),
                            session.created_at.elapsed().as_secs()
                        );
                        expired.push(session);
                    } else {
                        i += 1;
                    }
                }
            }

            // Remove empty config entries
            sessions.retain(|_, list| !list.is_empty());
            expired
        };

        for session in expired {
            session.close().await;
        }
    }

    /// Get connection statistics
    pub async fn stats(&self) -> ConnectionStats {
        let sessions = self.sessions.read().await;
        let mut total = 0;
        let mut in_use_count = 0;

        for session_list in sessions.values() {
            total += session_list.len();
            in_use_count += session_list.iter().filter(|s| s.in_use()).count();
        }

        ConnectionStats {
            total_sessions: total,
            active_sessions: in_use_count,
            configs_with_sessions: sessions.len(),
        }
    }

    /// List all sessions for a config
    pub async fn list_sessions(&self, config_id: &str) -> Vec<SessionInfo> {
        let session_list = {
            let sessions = self.sessions.read().await;
            sessions.get(config_id).cloned().unwrap_or_default()
        };

        let mut infos = Vec::with_capacity(session_list.len());
        for session in session_list {
            let last_active = session.last_active();
            infos.push(SessionInfo {
                session_id: session.session_id.clone(),
                // Cached identity instead of the live config: listing sessions must not
                // wait for a session that is busy with a statement.
                database: session.identity().database.clone(),
                in_use: session.in_use(),
                idle_time: last_active.elapsed(),
                lifetime: session.created_at.elapsed(),
            });
        }
        infos
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ConnectionManager {
    fn clone(&self) -> Self {
        Self {
            sessions: Arc::clone(&self.sessions),
            physical_open_locks: Arc::clone(&self.physical_open_locks),
            config_resolver: self.config_resolver.clone(),
            idle_timeout: self.idle_timeout,
            max_lifetime: self.max_lifetime,
            session_counter: Arc::clone(&self.session_counter),
        }
    }
}

/// Connection statistics
#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub configs_with_sessions: usize,
}

/// Session information
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session_id: String,
    pub database: Option<String>,
    pub in_use: bool,
    pub idle_time: Duration,
    pub lifetime: Duration,
}

/// Safe connection metadata for extension and UI callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbConnectionSummary {
    pub id: String,
    pub name: String,
    pub database_type: DatabaseType,
    pub database: Option<String>,
}

/// Global database state - stores DbManager and ConnectionManager
#[derive(Clone)]
pub struct GlobalDbState {
    pub db_manager: DbManager,
    pub connection_manager: ConnectionManager,
    /// connection_id -> config mapping
    connections: Arc<DashMap<String, DbConnectionConfig>>,
}

struct StreamingExecutionRequest {
    state: GlobalDbState,
    config: DbConnectionConfig,
    source: Option<SqlSource>,
    /// Keep the original source available after `execute_on_session` consumes
    /// the execution source. Script sources can use precise DDL invalidation;
    /// file sources must retain the conservative connection-wide behavior.
    invalidation_source: SqlSource,
    schema: Option<String>,
    opts: ExecOptions,
    tx: mpsc::Sender<StreamingProgress>,
    cache: Option<GlobalNodeCache>,
    cancellation: CancellationToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamingExecutionOutcome {
    Success,
    Error,
    Cancelled,
}

impl StreamingExecutionRequest {
    async fn run(mut self) -> Vec<(String, String, Option<String>)> {
        let total_size = self
            .source
            .as_ref()
            .and_then(SqlSource::file_size)
            .unwrap_or(0);
        let plugin = match self.state.get_plugin(&self.config.database_type) {
            Ok(plugin) => plugin,
            Err(error) => {
                send_streaming_error(
                    &self.tx,
                    format!("Failed to get database plugin: {error}"),
                    total_size,
                )
                .await;
                return Vec::new();
            }
        };
        let session_id = match self
            .state
            .connection_manager
            .create_session(self.config.clone(), &self.state.db_manager)
            .await
        {
            Ok(session_id) => session_id,
            Err(error) => {
                send_streaming_error(
                    &self.tx,
                    format!("Failed to create session: {error}"),
                    total_size,
                )
                .await;
                return Vec::new();
            }
        };

        let source = match self.source.take() {
            Some(source) => source,
            None => {
                send_streaming_error(
                    &self.tx,
                    "Streaming source already consumed".to_string(),
                    total_size,
                )
                .await;
                return Vec::new();
            }
        };
        let (driver_tx, mut driver_rx) = mpsc::channel::<StreamingProgress>(100);
        let cancellation = self.cancellation.clone();
        let mut execution =
            Box::pin(self.execute_on_session(&session_id, plugin.as_ref(), source, driver_tx));
        let mut exec_result = None;
        let mut outcome: StreamingExecutionOutcome;
        let mut confirmed_plan = SchemaInvalidationPlan::default();
        let mut had_successful_progress = false;
        let mut had_error_progress = false;
        let mut driver_progress_open = true;

        loop {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    outcome = StreamingExecutionOutcome::Cancelled;
                    break;
                }
                progress = driver_rx.recv(), if driver_progress_open => {
                    let Some(progress) = progress else {
                        driver_progress_open = false;
                        continue;
                    };
                    self.observe_streaming_progress(
                        &progress,
                        &mut confirmed_plan,
                        &mut had_successful_progress,
                        &mut had_error_progress,
                    );
                    let forwarded = tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => None,
                        result = self.tx.send(progress) => Some(result),
                    };
                    match forwarded {
                        Some(Ok(())) => {}
                        Some(Err(_)) | None => {
                            outcome = StreamingExecutionOutcome::Cancelled;
                            break;
                        }
                    }
                }
                result = &mut execution => {
                    outcome = if result.is_err() || had_error_progress {
                        StreamingExecutionOutcome::Error
                    } else {
                        StreamingExecutionOutcome::Success
                    };
                    exec_result = Some(result);
                    break;
                }
            }
        }
        drop(execution);

        if exec_result.is_some() {
            let mut forward_progress = outcome != StreamingExecutionOutcome::Cancelled;
            while let Some(progress) = driver_rx.recv().await {
                self.observe_streaming_progress(
                    &progress,
                    &mut confirmed_plan,
                    &mut had_successful_progress,
                    &mut had_error_progress,
                );
                if forward_progress {
                    let forwarded = tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => None,
                        result = self.tx.send(progress) => Some(result),
                    };
                    if !matches!(forwarded, Some(Ok(()))) {
                        forward_progress = false;
                    }
                }
            }
        } else {
            while let Ok(progress) = driver_rx.try_recv() {
                self.observe_streaming_progress(
                    &progress,
                    &mut confirmed_plan,
                    &mut had_successful_progress,
                    &mut had_error_progress,
                );
            }
        }
        if outcome == StreamingExecutionOutcome::Success && had_error_progress {
            outcome = StreamingExecutionOutcome::Error;
        }

        if let Err(error) = self
            .state
            .connection_manager
            .close_session(&session_id)
            .await
        {
            warn!("Failed to close streaming session {session_id}: {error}");
        }

        if let Some(Err(error)) = exec_result {
            error!("Streaming execution error: {error}");
            send_streaming_error(&self.tx, error.to_string(), total_size).await;
        }

        self.apply_streaming_invalidation(outcome, confirmed_plan, had_successful_progress)
            .await
    }

    async fn execute_on_session(
        &self,
        session_id: &str,
        plugin: &dyn DatabasePlugin,
        source: SqlSource,
        tx: mpsc::Sender<StreamingProgress>,
    ) -> anyhow::Result<()> {
        let mut guard = self
            .state
            .connection_manager
            .get_session_connection(session_id)
            .await?;
        let conn = guard
            .connection()
            .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;

        if let Some(schema) = &self.schema {
            conn.switch_schema(schema)
                .await
                .map_err(|error| anyhow::anyhow!("Failed to switch schema: {error}"))?;
        }

        conn.execute_streaming(plugin, source, self.opts.clone(), tx)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))
    }

    async fn apply_streaming_invalidation(
        &self,
        outcome: StreamingExecutionOutcome,
        confirmed_plan: SchemaInvalidationPlan,
        had_successful_progress: bool,
    ) -> Vec<(String, String, Option<String>)> {
        let Some(cache) = self.cache.as_ref() else {
            return Vec::new();
        };
        let conservative = should_conservatively_invalidate_streaming(
            &self.invalidation_source,
            self.opts.transactional,
            outcome,
            had_successful_progress,
        );
        let plan = if conservative {
            self.state.conservative_sql_cache_invalidation_plan(
                &self.config.id,
                self.config.database.as_deref(),
                self.schema.as_deref(),
            )
        } else {
            confirmed_plan
        };
        self.state
            .apply_sql_cache_invalidation_plan(cache, &self.config.id, &plan)
            .await
    }

    fn observe_streaming_progress(
        &self,
        progress: &StreamingProgress,
        confirmed_plan: &mut SchemaInvalidationPlan,
        had_successful_progress: &mut bool,
        had_error_progress: &mut bool,
    ) {
        if matches!(progress.result, SqlResult::Error(_)) {
            *had_error_progress = true;
            return;
        }
        let Some(sql) = successful_streaming_sql(&progress.result) else {
            return;
        };
        *had_successful_progress = true;
        if let (Some(cache), SqlSource::Script(_)) =
            (self.cache.as_ref(), &self.invalidation_source)
        {
            confirmed_plan.merge(self.state.plan_sql_cache_invalidation(
                cache,
                &self.config.id,
                sql,
                self.config.database.as_deref(),
                self.schema.as_deref(),
            ));
        }
    }
}

fn successful_streaming_sql(result: &SqlResult) -> Option<&str> {
    match result {
        SqlResult::Query(result) => Some(result.sql.as_str()),
        SqlResult::Exec(result) => Some(result.sql.as_str()),
        SqlResult::Error(_) => None,
    }
    .filter(|sql| !sql.trim().is_empty())
}

/// Streaming is mandatory for file sources because the executor reads them
/// incrementally instead of loading the whole file into memory.
fn streaming_exec_opts(source: &SqlSource, opts: Option<ExecOptions>) -> ExecOptions {
    let mut opts = opts.unwrap_or_default();
    if source.is_file() {
        opts.streaming = true;
    }
    opts
}

fn should_conservatively_invalidate_streaming(
    source: &SqlSource,
    transactional: bool,
    outcome: StreamingExecutionOutcome,
    had_successful_progress: bool,
) -> bool {
    match source {
        SqlSource::File(_) => {
            outcome == StreamingExecutionOutcome::Cancelled || had_successful_progress
        }
        SqlSource::Script(_) if !transactional => false,
        SqlSource::Script(_) => match outcome {
            StreamingExecutionOutcome::Success => false,
            StreamingExecutionOutcome::Error => had_successful_progress,
            StreamingExecutionOutcome::Cancelled => true,
        },
    }
}

async fn send_streaming_error(
    tx: &mpsc::Sender<StreamingProgress>,
    message: String,
    total_size: u64,
) {
    let progress = StreamingProgress::with_file_progress(
        0,
        SqlResult::Error(SqlErrorInfo {
            sql: String::new(),
            message,
        }),
        0,
        total_size,
    );
    let _ = tx.send(progress).await;
}

impl GlobalDbState {
    /// Build a reusable schema invalidation plan without mutating caches.
    ///
    /// Callers that own a longer-lived session (for example a manual
    /// transaction) can accumulate plans and apply them at the correct
    /// transaction boundary.
    pub fn plan_sql_cache_invalidation(
        &self,
        cache: &GlobalNodeCache,
        connection_id: &str,
        script: &str,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> SchemaInvalidationPlan {
        let Some(config) = self.get_config(connection_id) else {
            return SchemaInvalidationPlan::default();
        };
        let current_database = database.or(config.database.as_deref()).unwrap_or_default();
        cache.plan_sql_invalidation(
            script,
            SqlInvalidationContext {
                database: current_database,
                schema,
                database_type: &config.database_type,
            },
        )
    }

    /// Build a connection-wide fallback plan for an execution whose final
    /// transaction state cannot be determined.
    pub fn conservative_sql_cache_invalidation_plan(
        &self,
        connection_id: &str,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> SchemaInvalidationPlan {
        let current_database = database
            .map(str::to_string)
            .or_else(|| {
                self.get_config(connection_id)
                    .and_then(|config| config.database)
            })
            .unwrap_or_default();
        SchemaInvalidationPlan::conservative_connection_wide(
            current_database,
            schema.map(str::to_string),
        )
    }

    /// Apply a previously built schema invalidation plan and return every scope
    /// that should emit a `SchemaChanged` notification.
    pub async fn apply_sql_cache_invalidation_plan(
        &self,
        cache: &GlobalNodeCache,
        connection_id: &str,
        plan: &SchemaInvalidationPlan,
    ) -> Vec<(String, String, Option<String>)> {
        let cache_ctx = self
            .get_config(connection_id)
            .map(|config| CacheContext::from_config(&config));
        cache
            .apply_invalidation_plan(connection_id, plan, cache_ctx.as_ref())
            .await
            .into_iter()
            .map(|scope| (connection_id.to_string(), scope.database, scope.schema))
            .collect()
    }

    /// Invalidate metadata for SQL executed on a caller-owned session.
    ///
    /// Session-bound execution cannot use `execute_with_session_internal` because
    /// that method owns the session lifecycle. Keep cache invalidation separate so
    /// manual-transaction callers can use the same DDL parser and invalidator.
    pub async fn invalidate_sql_cache(
        &self,
        cache: &GlobalNodeCache,
        connection_id: &str,
        script: &str,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> Option<(String, String, Option<String>)> {
        let plan = self.plan_sql_cache_invalidation(cache, connection_id, script, database, schema);
        self.apply_sql_cache_invalidation_plan(cache, connection_id, &plan)
            .await
            .into_iter()
            .next()
    }

    async fn finish_direct_metadata_session<T>(
        &self,
        session_id: &str,
        result: anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let cleanup = self.connection_manager.release_session(session_id).await;
        match (result, cleanup) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(anyhow::anyhow!(
                "Failed to release metadata session: {error}"
            )),
            (Err(error), Err(cleanup_error)) => {
                tracing::warn!(
                    "Failed to release metadata session after metadata query error: {cleanup_error}"
                );
                Err(error)
            }
        }
    }

    pub fn new() -> Self {
        Self::with_config_resolver(ConnectionConfigResolver::default())
    }

    pub fn with_connection_repository(connection_repo: Option<Arc<ConnectionRepository>>) -> Self {
        Self::with_config_resolver(ConnectionConfigResolver::new(connection_repo))
    }

    fn with_config_resolver(config_resolver: ConnectionConfigResolver) -> Self {
        let manager = ConnectionManager::with_config_resolver(config_resolver);
        let db_manager = DbManager::new();

        Self {
            db_manager: db_manager.clone(),
            connection_manager: manager,
            connections: Arc::new(DashMap::new()),
        }
    }

    /// Start the cleanup task (should be called after Tokio runtime is available)
    pub fn start_cleanup_task<C>(&self, cx: &mut C)
    where
        C: AppContext,
    {
        let manager = Arc::new(self.connection_manager.clone());
        let _ = Tokio::spawn(cx, async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                manager.cleanup_expired_sessions().await;
            }
        });
    }

    /// Internal method for get_config
    pub fn get_config(&self, connection_id: &str) -> Option<DbConnectionConfig> {
        let config_ref = self.connections.get(connection_id);
        if let Some(config) = config_ref {
            return Some(config.value().clone());
        }
        None
    }

    pub fn list_connection_summaries(&self) -> Vec<DbConnectionSummary> {
        let mut summaries = self
            .connections
            .iter()
            .map(|entry| {
                let config = entry.value();
                DbConnectionSummary {
                    id: config.id.clone(),
                    name: config.name.clone(),
                    database_type: config.database_type.clone(),
                    database: config.database.clone(),
                }
            })
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| left.id.cmp(&right.id));
        summaries
    }

    pub fn get_plugin(
        &self,
        database_type: &DatabaseType,
    ) -> Result<Arc<dyn DatabasePlugin>, DbError> {
        self.db_manager.get_plugin(database_type)
    }

    fn wrapper_result(result: Vec<SqlResult>) -> anyhow::Result<SqlResult> {
        match result.into_iter().next() {
            Some(re) => Ok(re),
            None => Err(anyhow::anyhow!("No result returned")),
        }
    }

    /// Convert statement-level SQL errors into operation errors for schema actions.
    ///
    /// Interactive query execution intentionally keeps `SqlResult::Error` as a
    /// successful transport result so the editor can render it. Schema actions,
    /// however, use the outer `Result` to decide whether to show a success toast.
    fn wrapper_operation_result(result: Vec<SqlResult>) -> anyhow::Result<SqlResult> {
        match Self::wrapper_result(result)? {
            SqlResult::Error(error) => Err(anyhow::anyhow!(error.message)),
            result => Ok(result),
        }
    }

    pub async fn drop_database(
        &self,
        cx: &mut AsyncApp,
        config_id: String,
        database_name: String,
    ) -> anyhow::Result<SqlResult> {
        let config = self
            .get_config(&config_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", config_id))?;
        let plugin = self.get_plugin(&config.database_type)?;
        let sql = plugin.drop_database_async(&database_name).await?;

        let result = self.execute_with_session(cx, config, sql, None).await?;

        Self::wrapper_operation_result(result)
    }

    /// Drop table
    pub async fn drop_table(
        &self,
        cx: &mut AsyncApp,
        config_id: String,
        database: String,
        schema: Option<String>,
        table_name: String,
    ) -> anyhow::Result<SqlResult> {
        self.drop_table_like(
            cx,
            config_id,
            database,
            schema,
            table_name,
            TableObjectType::Table,
        )
        .await
    }

    /// 删除「表」目录下的对象。
    ///
    /// 外部表必须使用 `DROP FOREIGN TABLE`，否则 PostgreSQL 会拒绝执行。
    pub async fn drop_table_like(
        &self,
        cx: &mut AsyncApp,
        config_id: String,
        database: String,
        schema: Option<String>,
        table_name: String,
        object_type: TableObjectType,
    ) -> anyhow::Result<SqlResult> {
        let mut config = self
            .get_config(&config_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", config_id))?;
        let plugin = self.get_plugin(&config.database_type)?;
        let sql = match object_type {
            TableObjectType::ForeignTable => {
                plugin.drop_foreign_table(&database, schema.as_deref(), &table_name)
            }
            _ => plugin.drop_table(&database, schema.as_deref(), &table_name),
        };

        // For non-Oracle databases, modify config.database to switch database
        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database);
        }

        // Pass schema to switch before executing
        let result = self
            .execute_with_session_internal(cx, config, sql, None, schema)
            .await?;

        Self::wrapper_operation_result(result)
    }

    /// Truncate table
    pub async fn truncate_table(
        &self,
        cx: &mut AsyncApp,
        config_id: String,
        database: String,
        table_name: String,
    ) -> anyhow::Result<SqlResult> {
        self.truncate_table_with_schema(cx, config_id, database, None, table_name)
            .await
    }

    /// Truncate table with an optional schema.
    pub async fn truncate_table_with_schema(
        &self,
        cx: &mut AsyncApp,
        config_id: String,
        database: String,
        schema: Option<String>,
        table_name: String,
    ) -> anyhow::Result<SqlResult> {
        let mut config = self
            .get_config(&config_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", config_id))?;
        let plugin = self.get_plugin(&config.database_type)?;
        let sql = plugin.truncate_table_with_schema(&database, schema.as_deref(), &table_name);

        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database);
        }

        let result = self
            .execute_with_session_internal(cx, config, sql, None, schema)
            .await?;

        Self::wrapper_operation_result(result)
    }

    /// Rename table
    pub async fn rename_table(
        &self,
        cx: &mut AsyncApp,
        config_id: String,
        database: String,
        old_name: String,
        new_name: String,
    ) -> anyhow::Result<SqlResult> {
        self.rename_table_like(
            cx,
            config_id,
            database,
            None,
            old_name,
            new_name,
            TableObjectType::Table,
        )
        .await
    }

    /// 重命名「表」目录下的对象（外部表需 `ALTER FOREIGN TABLE`）。
    pub async fn rename_table_like(
        &self,
        cx: &mut AsyncApp,
        config_id: String,
        database: String,
        schema: Option<String>,
        old_name: String,
        new_name: String,
        object_type: TableObjectType,
    ) -> anyhow::Result<SqlResult> {
        let mut config = self
            .get_config(&config_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", config_id))?;
        let plugin = self.get_plugin(&config.database_type)?;
        let sql = match object_type {
            TableObjectType::ForeignTable => {
                plugin.rename_foreign_table(&database, schema.as_deref(), &old_name, &new_name)
            }
            _ => plugin.rename_table(&database, &old_name, &new_name),
        };

        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database);
        }

        let result = self.execute_with_session(cx, config, sql, None).await?;

        Self::wrapper_operation_result(result)
    }

    /// 删除物化视图（`DROP MATERIALIZED VIEW`）。
    pub async fn drop_materialized_view(
        &self,
        cx: &mut AsyncApp,
        config_id: String,
        database: String,
        schema: Option<String>,
        view_name: String,
    ) -> anyhow::Result<SqlResult> {
        let mut config = self
            .get_config(&config_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", config_id))?;
        let plugin = self.get_plugin(&config.database_type)?;
        let sql = plugin.drop_materialized_view(&database, schema.as_deref(), &view_name);

        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database);
        }

        let result = self.execute_with_session(cx, config, sql, None).await?;

        Self::wrapper_operation_result(result)
    }

    /// Drop view
    pub async fn drop_view(
        &self,
        cx: &mut AsyncApp,
        config_id: String,
        database: String,
        view_name: String,
    ) -> anyhow::Result<SqlResult> {
        let config = self
            .get_config(&config_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", config_id))?;
        let plugin = self.get_plugin(&config.database_type)?;
        let sql = plugin.drop_view(&database, &view_name);

        let result = self.execute_with_session(cx, config, sql, None).await?;

        Self::wrapper_operation_result(result)
    }

    /// Register a connection configuration
    pub fn register_connection(&mut self, config: DbConnectionConfig) {
        self.connections.insert(config.id.clone(), config);
    }

    pub async fn update_connection(
        &mut self,
        cx: &mut AsyncApp,
        config: DbConnectionConfig,
    ) -> anyhow::Result<()> {
        self.unregister_connection(cx, config.id.clone()).await?;
        self.register_connection(config);
        Ok(())
    }

    /// Unregister a connection configuration
    pub async fn unregister_connection(
        &mut self,
        cx: &mut AsyncApp,
        connection_id: String,
    ) -> anyhow::Result<()> {
        self.connections.remove(&connection_id);
        let clone_self = self.clone();
        // Remove from registry
        Tokio::spawn_result(cx, async move {
            // Close all sessions for this connection
            clone_self
                .connection_manager
                .remove_all_sessions(&connection_id)
                .await;
            Ok(())
        })
        .await
    }

    /// Create a new session for executing queries
    pub async fn create_session(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: Option<String>,
    ) -> anyhow::Result<String> {
        let clone_self = self.clone();
        let mut config = self
            .get_config(&connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;

        // Override database if specified
        if let Some(db) = database {
            config.database = Some(db);
        }
        Tokio::spawn_result(cx, async move {
            clone_self
                .connection_manager
                .create_session(config, &clone_self.db_manager)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        })
        .await
    }

    pub async fn create_session_direct(
        &self,
        connection_id: String,
        database: Option<String>,
    ) -> anyhow::Result<String> {
        require_tokio_runtime("database session creation")?;
        let mut config = self
            .get_config(&connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;
        if let Some(database) = database {
            config.database = Some(database);
        }
        self.connection_manager
            .create_session(config, &self.db_manager)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Execute SQL  (simplified - creates session per execution)
    pub async fn execute_single(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        script: String,
        database: Option<String>,
        opts: Option<ExecOptions>,
    ) -> anyhow::Result<SqlResult> {
        let result = self
            .execute_script(cx, connection_id, script, database, None, opts)
            .await?;
        Self::wrapper_result(result)
    }

    /// Execute SQL script (simplified - creates session per execution)
    pub async fn execute_script(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        script: String,
        database: Option<String>,
        schema: Option<String>,
        opts: Option<ExecOptions>,
    ) -> anyhow::Result<Vec<SqlResult>> {
        //  Get config
        let mut config = self
            .get_config(&connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;

        // Schema to switch before executing
        let schema_to_switch = schema;

        // For non-Oracle databases, modify config.database to switch database
        if config.database_type != DatabaseType::Oracle {
            if let Some(db) = database {
                config.database = Some(db);
            }
        }

        self.execute_with_session_internal(cx, config, script, opts, schema_to_switch)
            .await
    }

    /// Execute script with existing session (for transaction scenarios)
    pub async fn execute_with_session(
        &self,
        cx: &mut AsyncApp,
        config: DbConnectionConfig,
        script: String,
        opts: Option<ExecOptions>,
    ) -> anyhow::Result<Vec<SqlResult>> {
        self.execute_with_session_internal(cx, config, script, opts, None)
            .await
    }

    pub async fn execute_session(
        &self,
        session_id: String,
        script: String,
        opts: Option<ExecOptions>,
    ) -> anyhow::Result<Vec<SqlResult>> {
        require_tokio_runtime("database session execution")?;
        let config = self
            .connection_manager
            .get_session_config(&session_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;
        let plugin = self.get_plugin(&config.database_type)?;
        let mut guard = self
            .connection_manager
            .get_session_connection(&session_id)
            .await?;
        let conn = guard
            .connection()
            .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
        conn.execute(plugin.as_ref(), &script, opts.unwrap_or_default())
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Execute SQL on an existing session from a GPUI async task.
    ///
    /// Session connections may use Tokio I/O internally, so callers running on
    /// GPUI's executor must cross the Tokio runtime boundary explicitly.
    pub async fn execute_session_on_runtime(
        &self,
        cx: &mut AsyncApp,
        session_id: String,
        script: String,
        opts: Option<ExecOptions>,
    ) -> anyhow::Result<Vec<SqlResult>> {
        let clone_self = self.clone();
        Tokio::spawn_result(cx, async move {
            clone_self.execute_session(session_id, script, opts).await
        })
        .await
    }

    /// Query table data on an already-acquired session.
    ///
    /// Unlike `query_table_data`, this does not create or release a session.
    /// Callers that need connection-scoped state such as a read transaction can
    /// therefore keep COUNT, page queries, and terminal probes on one physical
    /// connection.
    pub async fn query_table_data_session(
        &self,
        session_id: &str,
        request: crate::types::TableDataRequest,
    ) -> anyhow::Result<crate::types::TableDataResponse> {
        require_tokio_runtime("database session query")?;
        let config = self
            .connection_manager
            .get_session_config(session_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;
        let plugin = self.get_plugin(&config.database_type)?;
        let mut guard = self
            .connection_manager
            .get_session_connection(session_id)
            .await?;
        let conn = guard
            .connection()
            .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
        plugin
            .query_table_data(&*conn, request)
            .await
            .map_err(|error| anyhow::anyhow!("{}", error))
    }

    /// Query table data on an existing session from a GPUI async task.
    ///
    /// Keeping this wrapper separate preserves the direct API for callers that
    /// already execute inside a Tokio runtime.
    pub async fn query_table_data_session_on_runtime(
        &self,
        cx: &mut AsyncApp,
        session_id: String,
        request: crate::types::TableDataRequest,
    ) -> anyhow::Result<crate::types::TableDataResponse> {
        let clone_self = self.clone();
        Tokio::spawn_result(cx, async move {
            clone_self
                .query_table_data_session(&session_id, request)
                .await
        })
        .await
    }

    pub async fn switch_session_schema(
        &self,
        session_id: String,
        schema: String,
    ) -> anyhow::Result<()> {
        require_tokio_runtime("database session schema switch")?;
        let mut guard = self
            .connection_manager
            .get_session_connection(&session_id)
            .await?;
        let conn = guard
            .connection()
            .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
        conn.switch_schema(&schema)
            .await
            .map_err(|error| anyhow::anyhow!("{}", error))
    }

    pub async fn list_databases_direct(
        &self,
        connection_id: String,
    ) -> anyhow::Result<Vec<String>> {
        require_tokio_runtime("database metadata query")?;
        let config = self
            .get_config(&connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;
        let plugin = self.get_plugin(&config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(config, &self.db_manager)
            .await?;
        let result = async {
            let mut guard = self
                .connection_manager
                .get_session_connection(&session_id)
                .await?;
            let conn = guard
                .connection()
                .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
            plugin
                .list_databases(&*conn)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        }
        .await;
        self.finish_direct_metadata_session(&session_id, result)
            .await
    }

    pub async fn list_schemas_direct(
        &self,
        connection_id: String,
        database: String,
    ) -> anyhow::Result<Vec<String>> {
        require_tokio_runtime("database metadata query")?;
        let mut config = self
            .get_config(&connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;
        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database.clone());
        }
        let plugin = self.get_plugin(&config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(config, &self.db_manager)
            .await?;
        let result = async {
            let mut guard = self
                .connection_manager
                .get_session_connection(&session_id)
                .await?;
            let conn = guard
                .connection()
                .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
            plugin
                .list_schemas(&*conn, &database)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        }
        .await;
        self.finish_direct_metadata_session(&session_id, result)
            .await
    }

    /// Build table-designer DDL SQL on an async path.
    ///
    /// This keeps synchronous preview builders local, while allowing external IPC
    /// drivers to provide dialect-specific DDL through `ddl/build_*` without
    /// blocking the UI thread.
    pub async fn build_table_design_sql(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        schema: Option<String>,
        original: Option<TableDesign>,
        design: TableDesign,
        column_renames: Vec<(String, String)>,
    ) -> anyhow::Result<String> {
        let mut config = self
            .get_config(&connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;

        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database);
        }

        let clone_self = self.clone();
        Tokio::spawn_result(cx, async move {
            let plugin = clone_self.get_plugin(&config.database_type)?;
            let session_id = clone_self
                .connection_manager
                .create_session(config, &clone_self.db_manager)
                .await?;

            let result = async {
                let mut guard = clone_self
                    .connection_manager
                    .get_session_connection(&session_id)
                    .await?;
                let conn = guard
                    .connection()
                    .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;

                if let Some(schema) = &schema {
                    conn.switch_schema(schema)
                        .await
                        .map_err(|error| anyhow::anyhow!("Failed to switch schema: {}", error))?;
                }

                match original.as_ref() {
                    Some(original) => {
                        plugin
                            .build_alter_table_sql_with_schema_async(
                                conn,
                                schema.as_deref(),
                                original,
                                &design,
                                &column_renames,
                            )
                            .await
                    }
                    None => {
                        plugin
                            .build_create_table_sql_with_schema_async(
                                conn,
                                schema.as_deref(),
                                &design,
                            )
                            .await
                    }
                }
            }
            .await;

            let close_result = clone_self
                .connection_manager
                .close_session(&session_id)
                .await;
            close_result?;
            result
        })
        .await
    }

    async fn execute_with_session_internal(
        &self,
        cx: &mut AsyncApp,
        config: DbConnectionConfig,
        script: String,
        opts: Option<ExecOptions>,
        schema_to_switch: Option<String>,
    ) -> anyhow::Result<Vec<SqlResult>> {
        // Access the cache used for DDL invalidation.
        let cache = cx.update(|cx| cx.try_global::<GlobalNodeCache>().cloned());

        let cache_ctx = cx.update(|cx| {
            cx.try_global::<GlobalDbState>()
                .and_then(|state| state.get_config(&config.id))
                .map(|cfg| CacheContext::from_config(&cfg))
        });

        let notifier = cx.update(|cx| cx.try_global::<GlobalConnectionNotifier>().cloned());

        let clone_self = self.clone();
        let config_id = config.id.clone();
        let current_database = config.database.clone().unwrap_or_default();
        let current_schema = schema_to_switch.clone();
        let database_type = config.database_type.clone();
        let script_for_ddl = script.clone();

        let result = Tokio::spawn_result(cx, async move {
            // Create session
            let session_id = clone_self
                .connection_manager
                .create_session(config.clone(), &clone_self.db_manager)
                .await?;

            // Execute query on session
            let opts = opts.unwrap_or_default();
            let is_transactional = opts.transactional;

            let plugin = clone_self.get_plugin(&config.database_type)?;

            let result = {
                let mut guard = clone_self
                    .connection_manager
                    .get_session_connection(&session_id)
                    .await?;
                let conn = guard
                    .connection()
                    .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;

                // Switch schema before executing
                if let Some(schema) = &schema_to_switch {
                    conn.switch_schema(schema)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to switch schema: {}", e))?;
                }

                conn.execute(plugin.as_ref(), &script, opts).await?
            };

            // Determine if session should stay open based on script content
            let upper_script = script.to_uppercase();
            let has_begin =
                upper_script.contains("BEGIN") || upper_script.contains("START TRANSACTION");
            let has_commit = upper_script.contains("COMMIT");
            let has_rollback = upper_script.contains("ROLLBACK");

            // Keep session open if: in transactional mode, or has BEGIN without COMMIT/ROLLBACK
            let keep_session = is_transactional || (has_begin && !has_commit && !has_rollback);

            if keep_session {
                // Release but don't close - session can be reused later
                clone_self
                    .connection_manager
                    .release_session_for_reuse(&session_id)
                    .await?;
            } else {
                // Close session completely
                clone_self
                    .connection_manager
                    .close_session(&session_id)
                    .await?;
            }

            Ok(result)
        })
        .await?;

        // Process DDL cache invalidation after successful execution.
        if let Some(cache) = cache {
            let ddl_info = Tokio::spawn_result(cx, async move {
                Ok(cache
                    .process_sql_for_invalidation(
                        &config_id,
                        &script_for_ddl,
                        &current_database,
                        current_schema.as_deref(),
                        &database_type,
                        cache_ctx.as_ref(),
                    )
                    .await)
            })
            .await;

            // Emit a SchemaChanged event when DDL changes are detected.
            if let Ok(Some((conn_id, database, schema))) = ddl_info {
                if let Some(notifier) = notifier {
                    cx.update(|cx| {
                        notifier.0.update(cx, |_, cx| {
                            cx.emit(ConnectionDataEvent::SchemaChanged {
                                connection_id: conn_id,
                                database,
                                schema,
                            });
                        });
                    });
                }
            }
        }

        Ok(result)
    }

    /// Execute SQL with streaming progress (supports both script string and file)
    /// Returns a receiver that will receive progress updates for each statement
    /// For file source, the file is read incrementally to avoid loading the entire file into memory
    pub fn execute_streaming(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        source: SqlSource,
        database: Option<String>,
        schema: Option<String>,
        opts: Option<ExecOptions>,
    ) -> anyhow::Result<mpsc::Receiver<StreamingProgress>> {
        self.execute_streaming_cancellable(
            cx,
            connection_id,
            source,
            database,
            schema,
            opts,
            CancellationToken::new(),
        )
    }

    /// Execute SQL with streaming progress and an externally controlled cancellation token.
    ///
    /// Cancelling the token drops the in-flight driver future, closes the temporary session,
    /// and then closes the progress channel.
    pub fn execute_streaming_cancellable(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        source: SqlSource,
        database: Option<String>,
        schema: Option<String>,
        opts: Option<ExecOptions>,
        cancellation: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamingProgress>> {
        let (tx, rx) = mpsc::channel::<StreamingProgress>(100);
        let mut config = self
            .get_config(&connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;

        if config.database_type != DatabaseType::Oracle {
            if let Some(db) = database {
                config.database = Some(db);
            }
        }

        let opts = streaming_exec_opts(&source, opts);

        let cache = cx.update(|cx| cx.try_global::<GlobalNodeCache>().cloned());
        let notifier = cx.update(|cx| cx.try_global::<GlobalConnectionNotifier>().cloned());
        let invalidation_source = source.clone();
        let request = StreamingExecutionRequest {
            state: self.clone(),
            config,
            source: Some(source),
            invalidation_source,
            schema,
            opts,
            tx,
            cache,
            cancellation,
        };
        let execution_task = Tokio::spawn(cx, request.run());

        cx.spawn(async move |cx: &mut AsyncApp| {
            if let Ok(scopes) = execution_task.await {
                if let Some(notifier) = notifier {
                    cx.update(|cx| {
                        notifier.0.update(cx, |_, cx| {
                            for (connection_id, database, schema) in scopes {
                                cx.emit(ConnectionDataEvent::SchemaChanged {
                                    connection_id,
                                    database,
                                    schema,
                                });
                            }
                        });
                    });
                }
            }
        })
        .detach();

        Ok(rx)
    }

    pub async fn with_session_connection<R, F>(
        &self,
        cx: &mut AsyncApp,
        config: DbConnectionConfig,
        f: F,
    ) -> anyhow::Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&dyn DatabasePlugin, &mut (dyn DbConnection + Send + Sync)) -> anyhow::Result<R>
            + Send
            + 'static,
    {
        let clone_self = self.clone();
        Tokio::spawn_result(cx, async move {
            let plugin = clone_self.get_plugin(&config.database_type)?;
            let session_id = clone_self
                .connection_manager
                .create_session(config.clone(), &clone_self.db_manager)
                .await?;

            let result = {
                let mut guard = clone_self
                    .connection_manager
                    .get_session_connection(&session_id)
                    .await?;
                let conn = guard
                    .connection()
                    .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
                f(&*plugin, conn)
            };

            clone_self
                .connection_manager
                .close_session(&session_id)
                .await?;

            result
        })
        .await
    }

    /// Get connection statistics
    pub async fn stats(&self, cx: &mut AsyncApp) -> anyhow::Result<ConnectionStats> {
        let clone_self = self.clone();
        Tokio::spawn_result(cx, async move {
            Ok(clone_self.connection_manager.stats().await)
        })
        .await
    }

    /// List all sessions for a connection
    pub async fn list_sessions(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
    ) -> anyhow::Result<Vec<SessionInfo>> {
        let clone_self = self.clone();
        Tokio::spawn_result(cx, async move {
            Ok(clone_self
                .connection_manager
                .list_sessions(&connection_id)
                .await)
        })
        .await
    }

    /// Close a specific session
    pub async fn close_session(&self, cx: &mut AsyncApp, session_id: String) -> anyhow::Result<()> {
        let clone_self = self.clone();
        Tokio::spawn_result(cx, async move {
            clone_self
                .connection_manager
                .close_session(&session_id)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        })
        .await
    }

    pub async fn close_session_direct(&self, session_id: &str) -> anyhow::Result<()> {
        require_tokio_runtime("database session close")?;
        self.connection_manager
            .close_session(session_id)
            .await
            .map_err(|error| anyhow::anyhow!("{}", error))
    }

    /// Disconnect all sessions for a connection
    pub async fn disconnect_all(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
    ) -> anyhow::Result<()> {
        let clone_self = self.clone();
        Tokio::spawn_result(cx, async move {
            clone_self
                .connection_manager
                .remove_all_sessions(&connection_id)
                .await;
            Ok(())
        })
        .await
    }

    /// Query table data
    pub async fn query_table_data(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        request: crate::types::TableDataRequest,
    ) -> anyhow::Result<crate::types::TableDataResponse> {
        info!("query_table_data: connection_id={}", connection_id);
        let database = request.database.clone();
        with_plugin_session_db!(self, cx, connection_id, database, |plugin, conn| {
            plugin.query_table_data(&*conn, request).await
        })
    }

    fn cached_children_ready(cached: &DbNode) -> bool {
        cached.children_loaded
    }

    /// Load node children for tree view
    pub async fn load_node_children(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        node: DbNode,
    ) -> anyhow::Result<Vec<DbNode>> {
        let load_started = Instant::now();
        // Resolve the connection config for the current node.
        let mut config = self
            .get_config(&connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?
            .clone();

        // Build the cache context up front for cache lookup and write-back.
        let cache_ctx = CacheContext::from_config(&config);

        // Access the global node cache if it is available.
        let cache = cx.update(|cx| cx.try_global::<GlobalNodeCache>().cloned());

        // For Database and Schema nodes, we need to connect to the specific database
        // This is especially important for PostgreSQL which doesn't support database switching
        let target_database = node.get_database_name();

        if let Some(db) = target_database {
            config.database = Some(db);
        }

        let clone_self = self.clone();
        let node_clone = node.clone();
        let connection_id_for_ui = connection_id.clone();
        let node_for_ui = node.clone();

        let result = Tokio::spawn_result(cx, async move {
            let async_started = Instant::now();
            // Try cache first to avoid unnecessary session creation.
            if let Some(ref cache) = cache {
                if let Some(cached) = cache.get_node(&cache_ctx, &node_clone.id).await {
                    if Self::cached_children_ready(&cached) {
                        debug!("Cache hit for node: {}", node_clone.id);
                        info!(
                            "[DB][Timing] load_node_children cache_hit connection_id={} node_id={} node_type={:?} children={} elapsed={}ms",
                            connection_id,
                            node_clone.id,
                            node_clone.node_type,
                            cached.children.len(),
                            async_started.elapsed().as_millis()
                        );
                        return Ok(cached.children);
                    }
                }
            }

            // Cache miss. Load children from the database.
            debug!(
                "Cache miss for node: {}, loading from database",
                node_clone.id
            );

            let plugin = clone_self.get_plugin(&config.database_type)?;
            let session_started = Instant::now();
            let session_id = clone_self
                .connection_manager
                .create_session(config.clone(), &clone_self.db_manager)
                .await?;
            info!(
                "[DB][Timing] load_node_children create_session connection_id={} node_id={} node_type={:?} session_id={} elapsed={}ms",
                connection_id,
                node_clone.id,
                node_clone.node_type,
                session_id,
                session_started.elapsed().as_millis()
            );

            let fetch_started = Instant::now();
            let result = {
                let mut guard = clone_self
                    .connection_manager
                    .get_session_connection(&session_id)
                    .await?;
                let conn = guard
                    .connection()
                    .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
                plugin
                    .load_node_children(&*conn, &node_clone)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))
            };
            info!(
                "[DB][Timing] load_node_children fetch connection_id={} node_id={} node_type={:?} session_id={} elapsed={}ms",
                connection_id,
                node_clone.id,
                node_clone.node_type,
                session_id,
                fetch_started.elapsed().as_millis()
            );

            let release_started = Instant::now();
            if let Err(e) = clone_self
                .connection_manager
                .release_session(&session_id)
                .await
            {
                warn!("Failed to release session {}: {}", session_id, e);
            } else {
                info!(
                    "[DB][Timing] load_node_children release_session connection_id={} node_id={} session_id={} elapsed={}ms",
                    connection_id,
                    node_clone.id,
                    session_id,
                    release_started.elapsed().as_millis()
                );
            }

            // Persist successful results back to the cache.
            if let Ok(ref children) = result {
                if let Some(ref cache) = cache {
                    let mut node_with_children = node_clone.clone();
                    node_with_children.children = children.clone();
                    node_with_children.children_loaded = true;

                    cache
                        .cache_node(&cache_ctx, &node_with_children.id, &node_with_children)
                        .await;
                    debug!(
                        "Cached node: {} with {} children",
                        node_with_children.id,
                        children.len()
                    );
                }
                info!(
                    "[DB][Timing] load_node_children total connection_id={} node_id={} node_type={:?} children={} elapsed={}ms",
                    connection_id,
                    node_clone.id,
                    node_clone.node_type,
                    children.len(),
                    async_started.elapsed().as_millis()
                );
            } else if let Err(ref error) = result {
                warn!(
                    "[DB][Timing] load_node_children failed connection_id={} node_id={} node_type={:?} elapsed={}ms error={}",
                    connection_id,
                    node_clone.id,
                    node_clone.node_type,
                    async_started.elapsed().as_millis(),
                    error
                );
            }

            result
        })
        .await;

        if let Ok(children) = &result {
            info!(
                "[DB][Timing] load_node_children ui_total connection_id={} node_id={} node_type={:?} children={} elapsed={}ms",
                connection_id_for_ui,
                node_for_ui.id,
                node_for_ui.node_type,
                children.len(),
                load_started.elapsed().as_millis()
            );
        }

        result
    }

    /// Apply table changes
    pub async fn apply_table_changes(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        request: crate::types::TableSaveRequest,
    ) -> anyhow::Result<TableSaveResponse> {
        let database = request.database.clone();
        with_plugin_session_db!(self, cx, connection_id, database, |plugin, conn| {
            let mut success_count = 0;
            let mut errors = Vec::new();

            for change in &request.changes {
                let Some(sql) = plugin.build_table_change_sql(&request, change) else {
                    continue;
                };

                match conn
                    .execute(plugin.as_ref(), &sql, ExecOptions::default())
                    .await
                {
                    Ok(results) => {
                        for result in results {
                            match result {
                                SqlResult::Exec(_) => {
                                    success_count += 1;
                                }
                                SqlResult::Error(err) => {
                                    errors.push(err.message);
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(e.to_string());
                    }
                }
            }

            anyhow::Ok(TableSaveResponse {
                success_count,
                errors,
            })
        })
    }

    /// List databases (with caching)
    pub async fn list_databases(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
    ) -> anyhow::Result<Vec<String>> {
        // Access the cache instance.
        let cache = cx.update(|cx| cx.try_global::<GlobalNodeCache>().cloned());

        // Try the cache first.
        if let Some(cache) = cache.clone() {
            let conn_id = connection_id.clone();
            let result = Tokio::spawn_result(cx, async move {
                if let Some(databases) = cache.get_databases(&conn_id).await {
                    debug!("Cache hit for databases: {}", conn_id);
                    return Ok(databases);
                }
                Err(anyhow::anyhow!("Cache miss"))
            })
            .await;

            if let Ok(databases) = result {
                return Ok(databases);
            }
        }

        // Cache miss. Query the database.
        let conn_id = connection_id.clone();
        let databases = with_plugin_session!(self, cx, connection_id, |plugin, conn| {
            plugin.list_databases(&*conn).await
        })?;

        // Persist the result in cache.
        if let Some(cache) = cache {
            let databases_clone = databases.clone();
            Tokio::spawn(cx, async move {
                cache.cache_databases(&conn_id, databases_clone).await;
                debug!("Cached databases for: {}", conn_id);
            })
            .detach();
        }

        Ok(databases)
    }

    /// List databases view
    pub async fn list_databases_view(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
    ) -> anyhow::Result<crate::types::ObjectView> {
        with_plugin_session!(self, cx, connection_id, |plugin, conn| {
            plugin.list_databases_view(&*conn).await
        })
    }

    pub async fn list_users_view(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: Option<String>,
    ) -> anyhow::Result<crate::types::ObjectView> {
        if let Some(database) = database {
            return with_plugin_session_db!(
                self,
                cx,
                connection_id,
                database.clone(),
                |plugin, conn| { plugin.list_users_view(&*conn, Some(&database)).await }
            );
        }
        with_plugin_session!(self, cx, connection_id, |plugin, conn| {
            plugin.list_users_view(&*conn, None).await
        })
    }

    pub fn capabilities(&self, database_type: &DatabaseType) -> DatabaseCapabilities {
        self.db_manager
            .get_plugin(database_type)
            .map(|plugin| plugin.capabilities())
            .unwrap_or_default()
    }

    /// List schemas in a database (with caching)
    pub async fn list_schemas(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
    ) -> anyhow::Result<Vec<String>> {
        // Access the cache instance.
        let cache = cx.update(|cx| cx.try_global::<GlobalNodeCache>().cloned());

        // Try the cache first.
        if let Some(cache) = cache.clone() {
            let conn_id = connection_id.clone();
            let db = database.clone();
            let result = Tokio::spawn_result(cx, async move {
                if let Some(schemas) = cache.get_schemas(&conn_id, &db).await {
                    debug!("Cache hit for schemas: {}:{}", conn_id, db);
                    return Ok(schemas);
                }
                Err(anyhow::anyhow!("Cache miss"))
            })
            .await;

            if let Ok(schemas) = result {
                return Ok(schemas);
            }
        }

        // Cache miss. Query the database.
        let conn_id = connection_id.clone();
        let db = database.clone();
        let schemas =
            with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
                plugin.list_schemas(&*conn, &database).await
            })?;

        // Persist the result in cache.
        if let Some(cache) = cache {
            let schemas_clone = schemas.clone();
            Tokio::spawn(cx, async move {
                cache.cache_schemas(&conn_id, &db, schemas_clone).await;
                debug!("Cached schemas for: {}:{}", conn_id, db);
            })
            .detach();
        }

        Ok(schemas)
    }

    /// List tables (with caching)
    pub async fn list_tables(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        schema: Option<String>,
    ) -> anyhow::Result<Vec<crate::types::TableInfo>> {
        // Access the cache instance.
        let cache = cx.update(|cx| cx.try_global::<GlobalNodeCache>().cloned());

        // Try the cache first.
        if let Some(cache) = cache.clone() {
            let conn_id = connection_id.clone();
            let db = database.clone();
            let sch = schema.clone();
            let result = Tokio::spawn_result(cx, async move {
                if let Some(tables) = cache.get_tables(&conn_id, &db, sch.as_deref()).await {
                    debug!("Cache hit for tables: {}:{}:{:?}", conn_id, db, sch);
                    return Ok(tables);
                }
                Err(anyhow::anyhow!("Cache miss"))
            })
            .await;

            if let Ok(tables) = result {
                return Ok(tables);
            }
        }

        // Cache miss. Query the database.
        let conn_id = connection_id.clone();
        let db = database.clone();
        let sch = schema.clone();
        let tables =
            with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
                plugin.list_tables(&*conn, &database, schema).await
            })?;

        // Persist the result in cache.
        if let Some(cache) = cache {
            let tables_clone = tables.clone();
            Tokio::spawn(cx, async move {
                cache
                    .cache_tables(&conn_id, &db, sch.as_deref(), tables_clone)
                    .await;
                debug!("Cached tables for: {}:{}:{:?}", conn_id, db, sch);
            })
            .detach();
        }

        Ok(tables)
    }

    /// List tables view
    pub async fn list_tables_view(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        schema: Option<String>,
    ) -> anyhow::Result<crate::types::ObjectView> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin.list_tables_view(&*conn, &database, schema).await
        })
    }

    /// List columns (with caching)
    pub async fn list_columns(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        schema: Option<String>,
        table: String,
    ) -> anyhow::Result<Vec<crate::types::ColumnInfo>> {
        // Access the cache instance.
        let cache = cx.update(|cx| cx.try_global::<GlobalNodeCache>().cloned());

        // Try the cache first.
        if let Some(cache) = cache.clone() {
            let conn_id = connection_id.clone();
            let db = database.clone();
            let sch = schema.clone();
            let tbl = table.clone();
            let result = Tokio::spawn_result(cx, async move {
                if let Some(columns) = cache.get_columns(&conn_id, &db, sch.as_deref(), &tbl).await
                {
                    debug!(
                        "Cache hit for columns: {}:{}:{:?}:{}",
                        conn_id, db, sch, tbl
                    );
                    return Ok(columns);
                }
                Err(anyhow::anyhow!("Cache miss"))
            })
            .await;

            if let Ok(columns) = result {
                return Ok(columns);
            }
        }

        // Cache miss. Query the database.
        let conn_id = connection_id.clone();
        let db = database.clone();
        let sch = schema.clone();
        let tbl = table.clone();
        let columns =
            with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
                plugin.list_columns(&*conn, &database, schema, &table).await
            })?;

        // Persist the result in cache.
        if let Some(cache) = cache {
            let columns_clone = columns.clone();
            Tokio::spawn(cx, async move {
                cache
                    .cache_columns(&conn_id, &db, sch.as_deref(), &tbl, columns_clone)
                    .await;
                debug!("Cached columns for: {}:{}:{:?}:{}", conn_id, db, sch, tbl);
            })
            .detach();
        }

        Ok(columns)
    }

    /// List columns view
    pub async fn list_columns_view(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        schema: Option<String>,
        table: String,
    ) -> anyhow::Result<crate::types::ObjectView> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin
                .list_columns_view(&*conn, &database, schema, &table)
                .await
        })
    }

    /// List indexes (with caching)
    pub async fn list_indexes(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        schema: Option<String>,
        table: String,
    ) -> anyhow::Result<Vec<crate::types::IndexInfo>> {
        // Access the cache instance.
        let cache = cx.update(|cx| cx.try_global::<GlobalNodeCache>().cloned());

        // Try the cache first.
        if let Some(cache) = cache.clone() {
            let conn_id = connection_id.clone();
            let db = database.clone();
            let sch = schema.clone();
            let tbl = table.clone();
            let result = Tokio::spawn_result(cx, async move {
                if let Some(indexes) = cache.get_indexes(&conn_id, &db, sch.as_deref(), &tbl).await
                {
                    debug!(
                        "Cache hit for indexes: {}:{}:{:?}:{}",
                        conn_id, db, sch, tbl
                    );
                    return Ok(indexes);
                }
                Err(anyhow::anyhow!("Cache miss"))
            })
            .await;

            if let Ok(indexes) = result {
                return Ok(indexes);
            }
        }

        // Cache miss. Query the database.
        let conn_id = connection_id.clone();
        let db = database.clone();
        let sch = schema.clone();
        let tbl = table.clone();
        let indexes =
            with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
                plugin.list_indexes(&*conn, &database, schema, &table).await
            })?;

        // Persist the result in cache.
        if let Some(cache) = cache {
            let indexes_clone = indexes.clone();
            Tokio::spawn(cx, async move {
                cache
                    .cache_indexes(&conn_id, &db, sch.as_deref(), &tbl, indexes_clone)
                    .await;
                debug!("Cached indexes for: {}:{}:{:?}:{}", conn_id, db, sch, tbl);
            })
            .detach();
        }

        Ok(indexes)
    }

    /// List foreign keys (with caching)
    pub async fn list_foreign_keys(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        schema: Option<String>,
        table: String,
    ) -> anyhow::Result<Vec<crate::types::ForeignKeyDefinition>> {
        let cache = cx.update(|cx| cx.try_global::<GlobalNodeCache>().cloned());
        if let Some(cache) = cache.clone() {
            let result = cached_foreign_keys(
                cx,
                cache,
                &connection_id,
                &database,
                schema.as_deref(),
                &table,
            )
            .await;
            if let Ok(foreign_keys) = result {
                return Ok(foreign_keys);
            }
        }

        let conn_id = connection_id.clone();
        let db = database.clone();
        let sch = schema.clone();
        let tbl = table.clone();
        let foreign_keys =
            with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
                plugin
                    .list_foreign_keys(&*conn, &database, schema, &table)
                    .await
            })?;

        if let Some(cache) = cache {
            let foreign_keys_clone = foreign_keys.clone();
            Tokio::spawn(cx, async move {
                cache
                    .cache_foreign_keys(&conn_id, &db, sch.as_deref(), &tbl, foreign_keys_clone)
                    .await;
            })
            .detach();
        }

        Ok(foreign_keys)
    }

    /// List views
    pub async fn list_views_view(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
    ) -> anyhow::Result<crate::types::ObjectView> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin.list_views_view(&*conn, &database).await
        })
    }

    /// List materialized views（仅 PostgreSQL 等支持物化视图的数据库）
    pub async fn list_materialized_views_view(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        schema: Option<String>,
    ) -> anyhow::Result<crate::types::ObjectView> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin
                .list_materialized_views_view(&*conn, &database, schema)
                .await
        })
    }

    /// List functions view
    pub async fn list_functions_view(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
    ) -> anyhow::Result<crate::types::ObjectView> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin.list_functions_view(&*conn, &database).await
        })
    }

    /// List functions
    pub async fn list_functions(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
    ) -> anyhow::Result<Vec<crate::types::FunctionInfo>> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin.list_functions(&*conn, &database).await
        })
    }

    /// List functions in a database schema.
    pub async fn list_functions_in_schema(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        schema: Option<String>,
    ) -> anyhow::Result<Vec<crate::types::FunctionInfo>> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin
                .list_functions_in_schema(&*conn, &database, schema)
                .await
        })
    }

    /// Load a stored function's CREATE statement
    pub async fn get_function_definition(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        function: String,
    ) -> anyhow::Result<String> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin
                .get_function_definition(&*conn, &database, &function)
                .await
        })
    }

    /// Load a stored function's database-specific edit script.
    pub async fn get_function_edit_script(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        routine: crate::types::RoutineIdentity,
    ) -> anyhow::Result<String> {
        let database = routine.database.clone();
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin.get_function_edit_script(&*conn, &routine).await
        })
    }

    /// List procedures view
    pub async fn list_procedures_view(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
    ) -> anyhow::Result<crate::types::ObjectView> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin.list_procedures_view(&*conn, &database).await
        })
    }

    /// List procedures in a database schema.
    pub async fn list_procedures_in_schema(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        schema: Option<String>,
    ) -> anyhow::Result<Vec<crate::types::FunctionInfo>> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin
                .list_procedures_in_schema(&*conn, &database, schema)
                .await
        })
    }

    /// Load a stored procedure's CREATE statement
    pub async fn get_procedure_definition(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        procedure: String,
    ) -> anyhow::Result<String> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin
                .get_procedure_definition(&*conn, &database, &procedure)
                .await
        })
    }

    /// Load a stored procedure's database-specific edit script.
    pub async fn get_procedure_edit_script(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        routine: crate::types::RoutineIdentity,
    ) -> anyhow::Result<String> {
        let database = routine.database.clone();
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin.get_procedure_edit_script(&*conn, &routine).await
        })
    }

    /// List triggers view
    pub async fn list_triggers_view(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
    ) -> anyhow::Result<crate::types::ObjectView> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin.list_triggers_view(&*conn, &database).await
        })
    }

    /// List triggers for a database/schema scope.
    pub async fn list_triggers(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
    ) -> anyhow::Result<Vec<crate::types::TriggerInfo>> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin.list_triggers(&*conn, &database).await
        })
    }

    /// List triggers in a database schema.
    pub async fn list_triggers_in_schema(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
        schema: Option<String>,
    ) -> anyhow::Result<Vec<crate::types::TriggerInfo>> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin
                .list_triggers_in_schema(&*conn, &database, schema)
                .await
        })
    }

    /// List sequences view
    pub async fn list_sequences_view(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
    ) -> anyhow::Result<crate::types::ObjectView> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin.list_sequences_view(&*conn, &database).await
        })
    }

    /// List schemas view
    pub async fn list_schemas_view(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        database: String,
    ) -> anyhow::Result<crate::types::ObjectView> {
        with_plugin_session_db!(self, cx, connection_id, database.clone(), |plugin, conn| {
            plugin.list_schemas_view(&*conn, &database).await
        })
    }

    /// Load object view based on node type
    pub async fn load_object_view(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        node: DbNode,
    ) -> anyhow::Result<Option<crate::types::ObjectView>> {
        if node.node_type == DbNodeType::Connection && !node.children_loaded {
            info!(
                "[DB][Timing] load_object_view skipped connection_id={} node_id={} reason=connection_children_not_loaded",
                connection_id, node.id
            );
            return Ok(None);
        }

        let mut config = self
            .get_config(&connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?
            .clone();

        let target_database = node.get_database_name();
        if let Some(db) = target_database {
            config.database = Some(db);
        }

        let database = config.database.clone().unwrap_or_default();
        let schema = node.get_schema_name();
        let table = node.get_table_name().unwrap_or_default();
        let clone_self = self.clone();
        Tokio::spawn_result(cx, async move {
            let plugin = clone_self.get_plugin(&config.database_type)?;
            let session_id = clone_self
                .connection_manager
                .create_session(config.clone(), &clone_self.db_manager)
                .await?;

            let result = {
                let mut guard = clone_self
                    .connection_manager
                    .get_session_connection(&session_id)
                    .await?;
                let conn = guard
                    .connection()
                    .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
                let view = match node.node_type {
                    DbNodeType::Connection => {
                        if node.children_loaded {
                            if plugin.capabilities().uses_schema_as_database {
                                plugin.list_schemas_view(&*conn, &database).await.ok()
                            } else {
                                plugin.list_databases_view(&*conn).await.ok()
                            }
                        } else {
                            None
                        }
                    }
                    DbNodeType::Database => {
                        if plugin.capabilities().supports_schema {
                            plugin.list_schemas_view(&*conn, &database).await.ok()
                        } else {
                            plugin.list_tables_view(&*conn, &database, None).await.ok()
                        }
                    }
                    DbNodeType::TablesFolder => plugin
                        .list_tables_view(&*conn, &database, schema)
                        .await
                        .ok(),
                    DbNodeType::Schema => plugin
                        .list_tables_view(&*conn, &database, schema)
                        .await
                        .ok(),
                    DbNodeType::Table | DbNodeType::ForeignTable | DbNodeType::ColumnsFolder => {
                        plugin
                            .list_columns_view(&*conn, &database, schema, &table)
                            .await
                            .ok()
                    }
                    DbNodeType::ViewsFolder => plugin.list_views_view(&*conn, &database).await.ok(),
                    DbNodeType::MaterializedViewsFolder => plugin
                        .list_materialized_views_view(&*conn, &database, schema)
                        .await
                        .ok(),
                    DbNodeType::FunctionsFolder => plugin
                        .list_functions_view_in_schema(&*conn, &database, schema)
                        .await
                        .ok(),
                    DbNodeType::ProceduresFolder => plugin
                        .list_procedures_view_in_schema(&*conn, &database, schema)
                        .await
                        .ok(),
                    DbNodeType::TriggersFolder => {
                        plugin.list_triggers_view(&*conn, &database).await.ok()
                    }
                    DbNodeType::SequencesFolder => plugin
                        .list_sequences_view_in_schema(&*conn, &database, schema)
                        .await
                        .ok(),
                    _ => None,
                };
                Ok::<_, anyhow::Error>(view)
            };

            if let Err(e) = clone_self
                .connection_manager
                .release_session(&session_id)
                .await
            {
                warn!("Failed to release session {}: {}", session_id, e);
            }

            result
        })
        .await
    }

    /// Get completion info
    pub fn get_completion_info(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
    ) -> anyhow::Result<crate::plugin::SqlCompletionInfo> {
        let _ = cx;
        if let Some(config) = self.get_config(&connection_id) {
            match self.get_plugin(&config.database_type) {
                Ok(plugin) => Ok(plugin.get_completion_info()),
                Err(_) => Ok(crate::plugin::SqlCompletionInfo::default()),
            }
        } else {
            Ok(crate::plugin::SqlCompletionInfo::default())
        }
    }

    /// Export data
    pub async fn export_data(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        config: ExportConfig,
    ) -> anyhow::Result<ExportResult> {
        self.export_data_with_progress(
            cx,
            ExportProgressRequest {
                connection_id,
                config,
                progress_tx: None,
            },
        )
        .await
    }

    /// Export data with progress callback on the application Tokio runtime.
    pub fn export_data_with_progress<C: AppContext>(
        &self,
        cx: &C,
        request: ExportProgressRequest,
    ) -> Task<anyhow::Result<ExportResult>> {
        let clone_self = self.clone();
        Tokio::spawn_result(cx, async move {
            clone_self.export_data_with_progress_on_tokio(request).await
        })
    }

    async fn export_data_with_progress_on_tokio(
        &self,
        request: ExportProgressRequest,
    ) -> anyhow::Result<ExportResult> {
        require_tokio_runtime("database export")?;
        let mut db_config = self
            .get_config(&request.connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", request.connection_id))?;

        // Some databases (notably PostgreSQL) cannot switch to another database
        // after connecting. When the export config identifies a database selected
        // in the UI, open the export session directly against that database.
        // Preserve the connection's default database when the export database is
        // unspecified.
        if !request.config.database.trim().is_empty() {
            db_config.database = Some(request.config.database.clone());
        }

        let plugin = self.get_plugin(&db_config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(db_config.clone(), &self.db_manager)
            .await?;

        let result = {
            let mut guard = self
                .connection_manager
                .get_session_connection(&session_id)
                .await?;
            let conn = guard
                .connection()
                .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
            plugin
                .export_data_with_progress(conn, &request.config, request.progress_tx)
                .await
                .map_err(|error| anyhow::anyhow!("{}", error))
        };

        self.connection_manager
            .release_session(&session_id)
            .await
            .map_err(|error| anyhow::anyhow!("{}", error))?;

        result
    }

    /// Import data
    pub async fn import_data(
        &self,
        cx: &mut AsyncApp,
        connection_id: String,
        config: ImportConfig,
        data: String,
    ) -> anyhow::Result<ImportResult> {
        self.import_data_with_progress(
            cx,
            ImportProgressRequest {
                connection_id,
                config,
                data,
                file_name: String::new(),
                progress_tx: None,
            },
        )
        .await
    }

    /// Import data with progress callback on the application Tokio runtime.
    pub fn import_data_with_progress<C: AppContext>(
        &self,
        cx: &C,
        request: ImportProgressRequest,
    ) -> Task<anyhow::Result<ImportResult>> {
        let clone_self = self.clone();
        Tokio::spawn_result(cx, async move {
            clone_self.import_data_with_progress_on_tokio(request).await
        })
    }

    async fn import_data_with_progress_on_tokio(
        &self,
        request: ImportProgressRequest,
    ) -> anyhow::Result<ImportResult> {
        require_tokio_runtime("database import")?;
        let db_config = self
            .get_config(&request.connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", request.connection_id))?;

        let plugin = self.get_plugin(&db_config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(db_config.clone(), &self.db_manager)
            .await?;

        let result = {
            let mut guard = self
                .connection_manager
                .get_session_connection(&session_id)
                .await?;
            let conn = guard
                .connection()
                .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
            plugin
                .import_data_with_progress(
                    conn,
                    &request.config,
                    &request.data,
                    &request.file_name,
                    request.progress_tx,
                )
                .await
                .map_err(|error| anyhow::anyhow!("{}", error))
        };

        self.connection_manager
            .release_session(&session_id)
            .await
            .map_err(|error| anyhow::anyhow!("{}", error))?;

        result
    }

    /// Pure async version of `list_tables` — can be called from any tokio context
    /// without `AsyncApp`. Skips `GlobalNodeCache`.
    pub async fn list_tables_direct(
        &self,
        connection_id: &str,
        database: &str,
        schema: Option<String>,
    ) -> anyhow::Result<Vec<crate::types::TableInfo>> {
        require_tokio_runtime("database metadata query")?;
        let config = self
            .get_config(connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;
        let mut config = config.clone();
        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database.to_string());
        }

        let plugin = self.get_plugin(&config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(config, &self.db_manager)
            .await?;

        let result = async {
            let mut guard = self
                .connection_manager
                .get_session_connection(&session_id)
                .await?;
            let conn = guard
                .connection()
                .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
            plugin
                .list_tables(conn, database, schema)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        }
        .await;

        self.finish_direct_metadata_session(&session_id, result)
            .await
    }

    /// Pure async version of `list_columns` — can be called from any tokio context
    /// without `AsyncApp`. Skips `GlobalNodeCache`.
    pub async fn list_columns_direct(
        &self,
        connection_id: &str,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> anyhow::Result<Vec<crate::types::ColumnInfo>> {
        require_tokio_runtime("database metadata query")?;
        let config = self
            .get_config(connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;
        let mut config = config.clone();
        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database.to_string());
        }

        let plugin = self.get_plugin(&config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(config, &self.db_manager)
            .await?;

        let result = async {
            let mut guard = self
                .connection_manager
                .get_session_connection(&session_id)
                .await?;
            let conn = guard
                .connection()
                .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
            plugin
                .list_columns(conn, database, schema, table)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        }
        .await;

        self.finish_direct_metadata_session(&session_id, result)
            .await
    }

    /// Pure async version of `list_indexes` — can be called from any tokio context
    /// without `AsyncApp`. Skips `GlobalNodeCache`.
    pub async fn list_indexes_direct(
        &self,
        connection_id: &str,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> anyhow::Result<Vec<crate::types::IndexInfo>> {
        require_tokio_runtime("database metadata query")?;
        let config = self
            .get_config(connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;
        let mut config = config.clone();
        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database.to_string());
        }

        let plugin = self.get_plugin(&config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(config, &self.db_manager)
            .await?;

        let result = async {
            let mut guard = self
                .connection_manager
                .get_session_connection(&session_id)
                .await?;
            let conn = guard
                .connection()
                .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
            plugin
                .list_indexes(conn, database, schema, table)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        }
        .await;

        self.finish_direct_metadata_session(&session_id, result)
            .await
    }

    /// Pure async version of `list_foreign_keys` — can be called from any tokio
    /// context without `AsyncApp`. Skips `GlobalNodeCache`.
    pub async fn list_foreign_keys_direct(
        &self,
        connection_id: &str,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> anyhow::Result<Vec<crate::types::ForeignKeyDefinition>> {
        require_tokio_runtime("database metadata query")?;
        let config = self
            .get_config(connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;
        let mut config = config.clone();
        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database.to_string());
        }

        let plugin = self.get_plugin(&config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(config, &self.db_manager)
            .await?;

        let result = async {
            let mut guard = self
                .connection_manager
                .get_session_connection(&session_id)
                .await?;
            let conn = guard
                .connection()
                .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
            plugin
                .list_foreign_keys(conn, database, schema, table)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        }
        .await;

        self.finish_direct_metadata_session(&session_id, result)
            .await
    }

    /// Loads schema-compare metadata using one session and one connection.
    pub async fn load_table_metadata_direct(
        &self,
        request: crate::types::DirectTableMetadataRequest,
    ) -> anyhow::Result<crate::types::DirectTableMetadata> {
        require_tokio_runtime("database metadata query")?;
        let mut config = self
            .get_config(&request.connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", request.connection_id))?
            .clone();
        if config.database_type != DatabaseType::Oracle {
            config.database = Some(request.database.clone());
        }
        let plugin = self.get_plugin(&config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(config, &self.db_manager)
            .await?;
        let result = self
            .query_table_metadata_session(&session_id, plugin.as_ref(), &request)
            .await;
        self.finish_direct_metadata_session(&session_id, result)
            .await
    }

    async fn query_table_metadata_session(
        &self,
        session_id: &str,
        plugin: &dyn DatabasePlugin,
        request: &crate::types::DirectTableMetadataRequest,
    ) -> anyhow::Result<crate::types::DirectTableMetadata> {
        let mut guard = self
            .connection_manager
            .get_session_connection(session_id)
            .await?;
        let connection = guard
            .connection()
            .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
        let columns = plugin
            .list_columns(
                connection,
                &request.database,
                request.schema.clone(),
                &request.table,
            )
            .await?;
        if !request.include_table_metadata {
            return Ok(crate::types::DirectTableMetadata {
                columns,
                ..Default::default()
            });
        }
        let indexes = plugin
            .list_indexes(
                connection,
                &request.database,
                request.schema.clone(),
                &request.table,
            )
            .await?;
        let foreign_keys = plugin
            .list_foreign_keys(
                connection,
                &request.database,
                request.schema.clone(),
                &request.table,
            )
            .await?;
        Ok(crate::types::DirectTableMetadata {
            columns,
            indexes,
            foreign_keys,
        })
    }

    /// Pure async version of `list_functions_in_schema` — skips all metadata
    /// cache reads and writes.
    pub async fn list_functions_in_schema_direct(
        &self,
        connection_id: &str,
        database: &str,
        schema: Option<String>,
    ) -> anyhow::Result<Vec<crate::types::FunctionInfo>> {
        require_tokio_runtime("database metadata query")?;
        let config = self
            .get_config(connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;
        let mut config = config.clone();
        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database.to_string());
        }

        let plugin = self.get_plugin(&config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(config, &self.db_manager)
            .await?;

        let result = async {
            let mut guard = self
                .connection_manager
                .get_session_connection(&session_id)
                .await?;
            let conn = guard
                .connection()
                .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
            plugin
                .list_functions_in_schema(conn, database, schema)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        }
        .await;

        self.finish_direct_metadata_session(&session_id, result)
            .await
    }

    /// Pure async version of `list_procedures_in_schema` — skips all metadata
    /// cache reads and writes.
    pub async fn list_procedures_in_schema_direct(
        &self,
        connection_id: &str,
        database: &str,
        schema: Option<String>,
    ) -> anyhow::Result<Vec<crate::types::FunctionInfo>> {
        require_tokio_runtime("database metadata query")?;
        let config = self
            .get_config(connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;
        let mut config = config.clone();
        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database.to_string());
        }

        let plugin = self.get_plugin(&config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(config, &self.db_manager)
            .await?;

        let result = async {
            let mut guard = self
                .connection_manager
                .get_session_connection(&session_id)
                .await?;
            let conn = guard
                .connection()
                .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
            plugin
                .list_procedures_in_schema(conn, database, schema)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        }
        .await;

        self.finish_direct_metadata_session(&session_id, result)
            .await
    }

    /// Pure async version of `list_triggers_in_schema` — skips all metadata
    /// cache reads and writes.
    pub async fn list_triggers_in_schema_direct(
        &self,
        connection_id: &str,
        database: &str,
        schema: Option<String>,
    ) -> anyhow::Result<Vec<crate::types::TriggerInfo>> {
        require_tokio_runtime("database metadata query")?;
        let config = self
            .get_config(connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;
        let mut config = config.clone();
        if config.database_type != DatabaseType::Oracle {
            config.database = Some(database.to_string());
        }

        let plugin = self.get_plugin(&config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(config, &self.db_manager)
            .await?;

        let result = async {
            let mut guard = self
                .connection_manager
                .get_session_connection(&session_id)
                .await?;
            let conn = guard
                .connection()
                .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;
            plugin
                .list_triggers_in_schema(conn, database, schema)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        }
        .await;

        self.finish_direct_metadata_session(&session_id, result)
            .await
    }

    /// Pure async SQL execution version — can be called from any tokio context
    /// without `AsyncApp`. Skips cache invalidation and notifier side effects.
    pub async fn execute_script_direct(
        &self,
        connection_id: &str,
        script: &str,
        database: Option<String>,
        schema: Option<String>,
        opts: Option<ExecOptions>,
    ) -> anyhow::Result<Vec<SqlResult>> {
        require_tokio_runtime("database script execution")?;
        let mut config = self
            .get_config(connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?
            .clone();

        // For non-Oracle databases, switch database through config override.
        if config.database_type != DatabaseType::Oracle {
            if let Some(db) = database {
                config.database = Some(db);
            }
        }

        let plugin = self.get_plugin(&config.database_type)?;
        let session_id = self
            .connection_manager
            .create_session(config, &self.db_manager)
            .await?;

        let result = async {
            let mut guard = self
                .connection_manager
                .get_session_connection(&session_id)
                .await?;
            let conn = guard
                .connection()
                .ok_or_else(|| anyhow::anyhow!("Session connection not found"))?;

            if let Some(schema) = &schema {
                conn.switch_schema(schema)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to switch schema: {}", e))?;
            }

            conn.execute(plugin.as_ref(), script, opts.unwrap_or_default())
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
        }
        .await;

        self.finish_direct_metadata_session(&session_id, result)
            .await
    }

    /// 获取数据库的比较能力
    pub fn get_compare_capabilities(
        &self,
        database_type: &DatabaseType,
    ) -> crate::compare::CompareCapabilities {
        use crate::compare::CompareCapabilities;
        match database_type {
            DatabaseType::PostgreSQL => CompareCapabilities::postgresql(),
            DatabaseType::MySQL => CompareCapabilities::mysql(),
            DatabaseType::SQLite => CompareCapabilities::sqlite(),
            DatabaseType::MSSQL => CompareCapabilities::sqlserver(),
            DatabaseType::ClickHouse => CompareCapabilities::clickhouse(),
            _ => CompareCapabilities::default(),
        }
    }
}

impl Default for GlobalDbState {
    fn default() -> Self {
        Self::new()
    }
}

impl Global for GlobalDbState {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::DbConnection;
    use crate::executor::{ExecOptions, ExecResult, SqlErrorInfo, SqlSource};
    use crate::ipc::{IpcDriverManifest, IpcDriverRegistry};
    use crate::plugin::ConnectionLifecycle;
    use crate::types::*;
    use crate::{DatabaseOperationRequest, ExportProgressSender, ImportProgressSender};
    use async_trait::async_trait;
    use one_core::storage::DatabaseType;
    use sqlparser::dialect::{Dialect, GenericDialect};
    use std::path::PathBuf;
    use std::sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use tokio::sync::mpsc;

    struct StreamingDropMarker(Arc<AtomicBool>);

    impl Drop for StreamingDropMarker {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    struct MockConnection {
        config: DbConnectionConfig,
        healthy: bool,
        disconnect_count: Arc<AtomicUsize>,
        executed_sql: Option<Arc<StdMutex<Vec<String>>>>,
        switched_schemas: Option<Arc<StdMutex<Vec<String>>>>,
        execution_started: Option<Arc<AtomicBool>>,
        execution_dropped: Option<Arc<AtomicBool>>,
        streaming_started: Option<Arc<AtomicBool>>,
        streaming_dropped: Option<Arc<AtomicBool>>,
        streaming_results: Option<Vec<SqlResult>>,
        /// 模拟空闲期被中间设备丢弃的 TCP 连接：任何网络调用永远挂起。
        dead_socket: bool,
    }

    impl MockConnection {
        fn new(config: DbConnectionConfig, healthy: bool) -> Self {
            Self {
                config,
                healthy,
                disconnect_count: Arc::new(AtomicUsize::new(0)),
                executed_sql: None,
                switched_schemas: None,
                execution_started: None,
                execution_dropped: None,
                streaming_started: None,
                streaming_dropped: None,
                streaming_results: None,
                dead_socket: false,
            }
        }

        fn with_disconnect_count(
            config: DbConnectionConfig,
            healthy: bool,
            disconnect_count: Arc<AtomicUsize>,
        ) -> Self {
            Self {
                config,
                healthy,
                disconnect_count,
                executed_sql: None,
                switched_schemas: None,
                execution_started: None,
                execution_dropped: None,
                streaming_started: None,
                streaming_dropped: None,
                streaming_results: None,
                dead_socket: false,
            }
        }

        fn with_dead_socket(config: DbConnectionConfig) -> Self {
            Self {
                dead_socket: true,
                ..Self::new(config, true)
            }
        }

        fn with_executed_sql(
            config: DbConnectionConfig,
            executed_sql: Arc<StdMutex<Vec<String>>>,
        ) -> Self {
            Self {
                config,
                healthy: true,
                disconnect_count: Arc::new(AtomicUsize::new(0)),
                executed_sql: Some(executed_sql),
                switched_schemas: None,
                execution_started: None,
                execution_dropped: None,
                streaming_started: None,
                streaming_dropped: None,
                streaming_results: None,
                dead_socket: false,
            }
        }

        fn with_switched_schemas(
            config: DbConnectionConfig,
            switched_schemas: Arc<StdMutex<Vec<String>>>,
        ) -> Self {
            Self {
                config,
                healthy: true,
                disconnect_count: Arc::new(AtomicUsize::new(0)),
                executed_sql: None,
                switched_schemas: Some(switched_schemas),
                execution_started: None,
                execution_dropped: None,
                streaming_started: None,
                streaming_dropped: None,
                streaming_results: None,
                dead_socket: false,
            }
        }

        fn with_blocking_execution(
            config: DbConnectionConfig,
            disconnect_count: Arc<AtomicUsize>,
            execution_started: Arc<AtomicBool>,
            execution_dropped: Arc<AtomicBool>,
        ) -> Self {
            Self {
                config,
                healthy: true,
                disconnect_count,
                executed_sql: None,
                switched_schemas: None,
                execution_started: Some(execution_started),
                execution_dropped: Some(execution_dropped),
                streaming_started: None,
                streaming_dropped: None,
                streaming_results: None,
                dead_socket: false,
            }
        }

        fn with_blocking_streaming(
            config: DbConnectionConfig,
            disconnect_count: Arc<AtomicUsize>,
            streaming_started: Arc<AtomicBool>,
            streaming_dropped: Arc<AtomicBool>,
        ) -> Self {
            Self {
                config,
                healthy: true,
                disconnect_count,
                executed_sql: None,
                switched_schemas: None,
                execution_started: None,
                execution_dropped: None,
                streaming_started: Some(streaming_started),
                streaming_dropped: Some(streaming_dropped),
                streaming_results: None,
                dead_socket: false,
            }
        }

        fn with_streaming_results(
            config: DbConnectionConfig,
            streaming_results: Vec<SqlResult>,
        ) -> Self {
            Self {
                config,
                healthy: true,
                disconnect_count: Arc::new(AtomicUsize::new(0)),
                executed_sql: None,
                switched_schemas: None,
                execution_started: None,
                execution_dropped: None,
                streaming_started: None,
                streaming_dropped: None,
                streaming_results: Some(streaming_results),
                dead_socket: false,
            }
        }
    }

    struct SlowOpenPlugin {
        database_type: DatabaseType,
        lifecycle: ConnectionLifecycle,
        active_opens: Arc<AtomicUsize>,
        max_active_opens: Arc<AtomicUsize>,
        open_started: Arc<tokio::sync::Notify>,
        created_configs: Option<Arc<StdMutex<Vec<DbConnectionConfig>>>>,
        metadata_calls: Option<Arc<StdMutex<Vec<&'static str>>>>,
    }

    impl SlowOpenPlugin {
        fn new(
            active_opens: Arc<AtomicUsize>,
            max_active_opens: Arc<AtomicUsize>,
            open_started: Arc<tokio::sync::Notify>,
        ) -> Self {
            Self::with_lifecycle(
                DatabaseType::DuckDB,
                ConnectionLifecycle {
                    close_on_release: true,
                    physical_open_lock_key: Some(
                        "duckdb:/tmp/duckdb-concurrent-open.duckdb".to_string(),
                    ),
                },
                active_opens,
                max_active_opens,
                open_started,
            )
        }

        fn with_lifecycle(
            database_type: DatabaseType,
            lifecycle: ConnectionLifecycle,
            active_opens: Arc<AtomicUsize>,
            max_active_opens: Arc<AtomicUsize>,
            open_started: Arc<tokio::sync::Notify>,
        ) -> Self {
            Self {
                database_type,
                lifecycle,
                active_opens,
                max_active_opens,
                open_started,
                created_configs: None,
                metadata_calls: None,
            }
        }

        fn recording_postgres(created_configs: Arc<StdMutex<Vec<DbConnectionConfig>>>) -> Self {
            let mut plugin = Self::with_lifecycle(
                DatabaseType::PostgreSQL,
                ConnectionLifecycle::default(),
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicUsize::new(0)),
                Arc::new(tokio::sync::Notify::new()),
            );
            plugin.created_configs = Some(created_configs);
            plugin
        }

        fn recording_table_metadata(
            created_configs: Arc<StdMutex<Vec<DbConnectionConfig>>>,
            metadata_calls: Arc<StdMutex<Vec<&'static str>>>,
        ) -> Self {
            let mut plugin = Self::recording_postgres(created_configs);
            plugin.metadata_calls = Some(metadata_calls);
            plugin
        }

        fn record_metadata_call(&self, operation: &'static str) {
            if let Some(calls) = &self.metadata_calls {
                calls.lock().unwrap().push(operation);
            }
        }
    }

    #[async_trait]
    impl DatabasePlugin for SlowOpenPlugin {
        fn name(&self) -> DatabaseType {
            self.database_type.clone()
        }

        fn quote_identifier(&self, identifier: &str) -> String {
            format!("\"{}\"", identifier.replace('"', "\"\""))
        }

        async fn create_connection(
            &self,
            config: DbConnectionConfig,
        ) -> Result<Box<dyn DbConnection + Send + Sync>, DbError> {
            if let Some(created_configs) = &self.created_configs {
                created_configs.lock().unwrap().push(config.clone());
            }
            let active = self.active_opens.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active_opens.fetch_max(active, Ordering::SeqCst);
            self.open_started.notify_waiters();
            sleep(Duration::from_millis(100)).await;
            self.active_opens.fetch_sub(1, Ordering::SeqCst);
            Ok(Box::new(MockConnection::new(config, true)))
        }

        fn connection_lifecycle(&self, _config: &DbConnectionConfig) -> ConnectionLifecycle {
            self.lifecycle.clone()
        }

        async fn list_databases(
            &self,
            _connection: &dyn DbConnection,
        ) -> anyhow::Result<Vec<String>> {
            Ok(Vec::new())
        }

        async fn list_databases_view(
            &self,
            _connection: &dyn DbConnection,
        ) -> anyhow::Result<ObjectView> {
            Ok(ObjectView::default())
        }

        async fn list_databases_detailed(
            &self,
            _connection: &dyn DbConnection,
        ) -> anyhow::Result<Vec<DatabaseInfo>> {
            Ok(Vec::new())
        }

        fn sql_dialect(&self) -> Box<dyn Dialect> {
            Box::new(GenericDialect {})
        }

        async fn list_tables(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
            _schema: Option<String>,
        ) -> anyhow::Result<Vec<TableInfo>> {
            Ok(Vec::new())
        }

        async fn list_tables_view(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
            _schema: Option<String>,
        ) -> anyhow::Result<ObjectView> {
            Ok(ObjectView::default())
        }

        async fn list_columns(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
            _schema: Option<String>,
            _table: &str,
        ) -> anyhow::Result<Vec<ColumnInfo>> {
            self.record_metadata_call("columns");
            Ok(Vec::new())
        }

        async fn list_columns_view(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
            _schema: Option<String>,
            _table: &str,
        ) -> anyhow::Result<ObjectView> {
            Ok(ObjectView::default())
        }

        async fn list_indexes(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
            _schema: Option<String>,
            _table: &str,
        ) -> anyhow::Result<Vec<IndexInfo>> {
            self.record_metadata_call("indexes");
            Ok(Vec::new())
        }

        async fn list_foreign_keys(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
            _schema: Option<String>,
            _table: &str,
        ) -> anyhow::Result<Vec<ForeignKeyDefinition>> {
            self.record_metadata_call("foreign_keys");
            Ok(Vec::new())
        }

        async fn list_indexes_view(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
            _schema: Option<&str>,
            _table: &str,
        ) -> anyhow::Result<ObjectView> {
            Ok(ObjectView::default())
        }

        async fn list_views(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
            _schema: Option<String>,
        ) -> anyhow::Result<Vec<ViewInfo>> {
            Ok(Vec::new())
        }

        async fn list_views_view(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
        ) -> anyhow::Result<ObjectView> {
            Ok(ObjectView::default())
        }

        async fn list_functions(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
        ) -> anyhow::Result<Vec<FunctionInfo>> {
            Ok(Vec::new())
        }

        async fn list_functions_view(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
        ) -> anyhow::Result<ObjectView> {
            Ok(ObjectView::default())
        }

        async fn list_procedures(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
        ) -> anyhow::Result<Vec<FunctionInfo>> {
            Ok(Vec::new())
        }

        async fn list_procedures_view(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
        ) -> anyhow::Result<ObjectView> {
            Ok(ObjectView::default())
        }

        async fn list_triggers(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
        ) -> anyhow::Result<Vec<TriggerInfo>> {
            Ok(Vec::new())
        }

        async fn list_triggers_view(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
        ) -> anyhow::Result<ObjectView> {
            Ok(ObjectView::default())
        }

        async fn list_sequences(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
            _schema: Option<String>,
        ) -> anyhow::Result<Vec<SequenceInfo>> {
            Ok(Vec::new())
        }

        async fn list_sequences_view(
            &self,
            _connection: &dyn DbConnection,
            _database: &str,
        ) -> anyhow::Result<ObjectView> {
            Ok(ObjectView::default())
        }

        fn build_column_definition(&self, column: &ColumnInfo, include_name: bool) -> String {
            if include_name {
                format!(
                    "{} {}",
                    self.quote_identifier(&column.name),
                    column.data_type
                )
            } else {
                column.data_type.clone()
            }
        }

        fn build_create_database_sql(&self, request: &DatabaseOperationRequest) -> String {
            format!(
                "CREATE DATABASE {}",
                self.quote_identifier(&request.database_name)
            )
        }

        fn build_modify_database_sql(&self, _request: &DatabaseOperationRequest) -> String {
            String::new()
        }

        fn build_drop_database_sql(&self, database_name: &str) -> String {
            format!("DROP DATABASE {}", self.quote_identifier(database_name))
        }

        fn build_limit_clause(&self) -> String {
            "LIMIT ? OFFSET ?".to_string()
        }

        fn build_where_and_limit_clause(
            &self,
            _request: &TableSaveRequest,
            _original_data: &[TableCellValue],
        ) -> (String, String) {
            (String::new(), self.build_limit_clause())
        }

        fn rename_table(&self, _database: &str, old_name: &str, new_name: &str) -> String {
            format!(
                "ALTER TABLE {} RENAME TO {}",
                self.quote_identifier(old_name),
                self.quote_identifier(new_name)
            )
        }

        fn build_column_def(&self, col: &ColumnDefinition) -> String {
            format!("{} {}", self.quote_identifier(&col.name), col.data_type)
        }

        fn build_create_table_sql(&self, design: &TableDesign) -> String {
            format!("CREATE TABLE {}", self.quote_identifier(&design.table_name))
        }

        fn build_alter_table_sql(&self, _original: &TableDesign, _new: &TableDesign) -> String {
            String::new()
        }

        async fn import_data_with_progress(
            &self,
            _connection: &dyn DbConnection,
            _config: &ImportConfig,
            _data: &str,
            _file_name: &str,
            _progress_tx: Option<ImportProgressSender>,
        ) -> anyhow::Result<ImportResult> {
            Ok(ImportResult {
                success: true,
                rows_imported: 0,
                errors: Vec::new(),
                elapsed_ms: 0,
            })
        }

        async fn export_data_with_progress(
            &self,
            _connection: &dyn DbConnection,
            _config: &ExportConfig,
            _progress_tx: Option<ExportProgressSender>,
        ) -> anyhow::Result<ExportResult> {
            Ok(ExportResult {
                success: true,
                output: String::new(),
                rows_exported: 0,
                elapsed_ms: 0,
            })
        }
    }

    #[async_trait]
    impl DbConnection for MockConnection {
        fn config(&self) -> &DbConnectionConfig {
            &self.config
        }

        fn set_config_database(&mut self, database: Option<String>) {
            self.config.database = database;
        }

        async fn connect(&mut self) -> Result<(), DbError> {
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<(), DbError> {
            if self.dead_socket {
                std::future::pending::<()>().await;
                unreachable!("dead socket disconnect must be timed out by the caller");
            }
            self.disconnect_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn execute(
            &self,
            _plugin: &dyn DatabasePlugin,
            script: &str,
            _options: ExecOptions,
        ) -> Result<Vec<SqlResult>, DbError> {
            if let (Some(execution_started), Some(execution_dropped)) =
                (&self.execution_started, &self.execution_dropped)
            {
                let _drop_marker = StreamingDropMarker(execution_dropped.clone());
                execution_started.store(true, Ordering::SeqCst);
                std::future::pending::<()>().await;
                unreachable!("blocking execution mock must be cancelled");
            }
            if let Some(executed_sql) = &self.executed_sql {
                executed_sql.lock().unwrap().push(script.to_string());
            }
            Ok(vec![SqlResult::Exec(ExecResult {
                sql: script.to_string(),
                rows_affected: 0,
                elapsed_ms: 0,
                message: None,
            })])
        }

        async fn query(&self, query: &str) -> Result<SqlResult, DbError> {
            if self.dead_socket {
                std::future::pending::<()>().await;
                unreachable!("dead socket query must be timed out by the caller");
            }
            if self.healthy {
                Ok(SqlResult::Exec(ExecResult {
                    sql: query.to_string(),
                    rows_affected: 0,
                    elapsed_ms: 0,
                    message: None,
                }))
            } else {
                Ok(SqlResult::Error(SqlErrorInfo {
                    sql: query.to_string(),
                    message: "connection closed".to_string(),
                }))
            }
        }

        async fn current_database(&self) -> Result<Option<String>, DbError> {
            Ok(self.config.database.clone())
        }

        async fn switch_database(&self, _database: &str) -> Result<(), DbError> {
            Ok(())
        }

        async fn switch_schema(&self, schema: &str) -> Result<(), DbError> {
            if let Some(switched_schemas) = &self.switched_schemas {
                switched_schemas.lock().unwrap().push(schema.to_string());
            }
            Ok(())
        }

        async fn execute_streaming(
            &self,
            _plugin: &dyn DatabasePlugin,
            source: SqlSource,
            _options: ExecOptions,
            sender: mpsc::Sender<StreamingProgress>,
        ) -> Result<(), DbError> {
            if let Some(results) = &self.streaming_results {
                let total = results.len();
                for (index, result) in results.iter().cloned().enumerate() {
                    if sender
                        .send(StreamingProgress::new(index + 1, total, result))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                return Ok(());
            }

            if let (Some(streaming_started), Some(streaming_dropped)) =
                (&self.streaming_started, &self.streaming_dropped)
            {
                let _drop_marker = StreamingDropMarker(streaming_dropped.clone());
                streaming_started.store(true, Ordering::SeqCst);
                std::future::pending::<()>().await;
                unreachable!("blocking streaming mock must be cancelled");
            }

            let (sql, is_file) = match source {
                SqlSource::Script(sql) => (sql, false),
                SqlSource::File(_) => ("CREATE TABLE widgets (id INT)".to_string(), true),
            };
            let result = SqlResult::Exec(ExecResult {
                sql,
                rows_affected: 0,
                elapsed_ms: 0,
                message: None,
            });
            let progress = if is_file {
                StreamingProgress::with_file_progress(1, result, 1, 1)
            } else {
                StreamingProgress::new(1, 1, result)
            };
            let _ = sender.send(progress).await;
            Ok(())
        }
    }

    fn test_config(id: &str) -> DbConnectionConfig {
        DbConnectionConfig {
            id: id.to_string(),
            database_type: DatabaseType::PostgreSQL,
            name: "test".to_string(),
            host: "localhost".to_string(),
            port: 5432,
            username: "user".to_string(),
            password: "password".to_string(),
            database: Some("postgres".to_string()),
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            credential_reference: None,
            extra_params: Default::default(),
        }
    }

    struct StreamingCacheTestContext {
        state: GlobalDbState,
        cache: GlobalNodeCache,
        config: DbConnectionConfig,
    }

    /// Streaming cache tests run on a plain Tokio runtime instead of the
    /// deterministic GPUI test scheduler: the production path hands the request
    /// to Tokio and waits for its join handle on the GPUI background executor,
    /// so a worker thread completes the request and wakes that executor, which
    /// the test scheduler reports as non-deterministic activity. Building the
    /// request directly keeps the cache assertions deterministic.
    async fn setup_streaming_cache_test(connection_id: &str) -> StreamingCacheTestContext {
        setup_streaming_cache_test_with_connection(connection_id, |config| {
            MockConnection::new(config, true)
        })
        .await
    }

    async fn setup_streaming_cache_test_with_connection(
        connection_id: &str,
        build_connection: impl FnOnce(DbConnectionConfig) -> MockConnection,
    ) -> StreamingCacheTestContext {
        let cache = GlobalNodeCache::with_config(crate::metadata_cache::MetadataCacheConfig {
            enable_file_cache: false,
            ..Default::default()
        })
        .unwrap();
        let mut state = GlobalDbState::new();
        let config = test_config(connection_id);
        state.register_connection(config.clone());

        let session = ConnectionSession::new(
            Box::new(build_connection(config.clone())),
            format!("{}:session:1", config.id),
            false,
        );
        state
            .connection_manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::new(session));

        cache
            .cache_tables(connection_id, "postgres", Some("public"), Vec::new())
            .await;

        StreamingCacheTestContext {
            state,
            cache,
            config,
        }
    }

    async fn run_streaming_source(test: &StreamingCacheTestContext, source: SqlSource) -> bool {
        run_streaming_source_with_options(test, source, None).await
    }

    /// Mirrors `GlobalDbState::execute_streaming_cancellable` without the GPUI
    /// layer: same request fields, same progress channel, same Tokio runtime.
    async fn run_streaming_source_with_options(
        test: &StreamingCacheTestContext,
        source: SqlSource,
        options: Option<ExecOptions>,
    ) -> bool {
        let invalidation_source = source.clone();
        let opts = streaming_exec_opts(&source, options);
        let (tx, mut progress) = mpsc::channel::<StreamingProgress>(100);
        let request = StreamingExecutionRequest {
            state: test.state.clone(),
            config: test.config.clone(),
            source: Some(source),
            invalidation_source,
            schema: Some("public".to_string()),
            opts,
            tx,
            cache: Some(test.cache.clone()),
            cancellation: CancellationToken::new(),
        };
        let execution = tokio::spawn(request.run());
        while progress.recv().await.is_some() {}
        execution
            .await
            .expect("streaming execution task must not panic");

        test.cache
            .get_tables(&test.config.id, "postgres", Some("public"))
            .await
            .is_some()
    }

    #[test]
    fn wrapper_operation_result_surfaces_statement_errors() {
        let result =
            GlobalDbState::wrapper_operation_result(vec![SqlResult::Error(SqlErrorInfo {
                sql: "RENAME TABLE `old` TO `new`".to_string(),
                message: "table does not exist".to_string(),
            })]);

        assert_eq!("table does not exist", result.unwrap_err().to_string());
    }

    #[test]
    fn wrapper_result_preserves_statement_errors_for_query_execution() {
        let result = GlobalDbState::wrapper_result(vec![SqlResult::Error(SqlErrorInfo {
            sql: "SELECT missing_column".to_string(),
            message: "unknown column".to_string(),
        })])
        .unwrap();

        assert!(matches!(result, SqlResult::Error(_)));
    }

    fn external_driver_manifest(id: &str, quote: &str, supports_schema: bool) -> IpcDriverManifest {
        let mut driver: IpcDriverManifest = serde_json::from_str(&format!(
            r#"{{
                "id":"{id}",
                "name":"{id}",
                "entry":{{"command":"driver"}},
                "transport":{{"name":"{id}.sock"}},
                "capabilities":{{"supports_schema":{supports_schema}}}
            }}"#
        ))
        .unwrap();
        driver.dialect.identifier_quote_left = quote.to_string();
        driver.manifest_dir = PathBuf::from(format!("/drivers/{id}"));
        driver
    }

    #[cfg(feature = "builtin-duckdb")]
    #[test]
    fn test_db_manager_registers_duckdb_plugin() {
        let plugin = DbManager::default()
            .get_plugin(&DatabaseType::DuckDB)
            .expect("DuckDB plugin should be registered");

        assert_eq!(plugin.name(), DatabaseType::DuckDB);
    }

    #[cfg(not(feature = "builtin-duckdb"))]
    #[test]
    fn default_db_manager_uses_external_plugin_for_duckdb() {
        let plugin = DbManager::default()
            .get_plugin(&DatabaseType::DuckDB)
            .expect("DuckDB plugin should be available through external IPC");

        assert_eq!(plugin.name(), DatabaseType::external("duckdb"));
    }

    #[cfg(not(feature = "builtin-duckdb"))]
    #[tokio::test]
    async fn external_duckdb_plugin_uses_ipc_driver_id_without_external_param() {
        let mut config = test_config("duckdb-ipc");
        config.database_type = DatabaseType::DuckDB;
        config.host = tempfile::tempdir()
            .unwrap()
            .path()
            .join("duckdb-ipc.duckdb")
            .to_string_lossy()
            .to_string();
        config.database = Some("main".to_string());

        let plugin = ExternalDatabasePlugin::with_registry(IpcDriverRegistry::empty());

        let error = plugin.test_connection(config).await.unwrap_err();

        assert!(
            format!("{error}").contains("external driver 'duckdb' not found"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn db_manager_resolves_external_plugin_by_connection_driver_id() {
        let manager = DbManager::with_external_registry(IpcDriverRegistry::from_drivers(vec![
            external_driver_manifest("alpha", "`", true),
            external_driver_manifest("beta", "\"", false),
        ]));

        let alpha = manager
            .get_plugin(&DatabaseType::external("alpha"))
            .unwrap();
        let beta = manager.get_plugin(&DatabaseType::external("beta")).unwrap();

        assert_eq!("`a``b`", alpha.quote_identifier("a`b"));
        assert!(alpha.capabilities().supports_schema);
        assert_eq!("\"a\"\"b\"", beta.quote_identifier("a\"b"));
        assert!(!beta.capabilities().supports_schema);
    }

    #[test]
    fn db_manager_reloads_external_driver_added_after_manager_creation() {
        let manager = DbManager::with_external_registry_reloader(
            IpcDriverRegistry::empty(),
            Arc::new(|| {
                IpcDriverRegistry::from_drivers(vec![external_driver_manifest("dm", "\"", true)])
            }),
        );

        let plugin = manager.get_plugin(&DatabaseType::external("dm")).unwrap();

        assert_eq!(DatabaseType::external("dm"), plugin.name());
        assert!(plugin.capabilities().supports_schema);
    }

    #[test]
    fn db_manager_reloads_updated_external_driver_manifest_for_existing_id() {
        let mut stale = external_driver_manifest("oracle-go", "\"", true);
        stale
            .capabilities
            .as_mut()
            .expect("test manifest has capabilities")
            .uses_schema_as_database = false;
        let mut fresh = stale.clone();
        fresh
            .capabilities
            .as_mut()
            .expect("test manifest has capabilities")
            .uses_schema_as_database = true;
        fresh.dialect.uses_schema_as_database = true;

        let manager = DbManager::with_external_registry_reloader(
            IpcDriverRegistry::from_drivers(vec![stale]),
            Arc::new(move || IpcDriverRegistry::from_drivers(vec![fresh.clone()])),
        );

        let plugin = manager
            .get_plugin(&DatabaseType::external("oracle-go"))
            .unwrap();

        assert!(plugin.capabilities().uses_schema_as_database);
    }

    #[test]
    fn test_cached_children_ready_allows_empty_children() {
        let node = DbNode::new(
            "node-id",
            "node",
            DbNodeType::Table,
            "conn-id".to_string(),
            DatabaseType::SQLite,
        )
        .with_children_loaded(true);

        assert!(GlobalDbState::cached_children_ready(&node));
    }

    #[test]
    fn list_connection_summaries_exposes_safe_metadata() {
        let mut state = GlobalDbState::new();
        state.register_connection(test_config("conn1"));

        let summaries = state.list_connection_summaries();

        assert_eq!(1, summaries.len());
        assert_eq!("conn1", summaries[0].id);
        assert_eq!("test", summaries[0].name);
        assert_eq!(DatabaseType::PostgreSQL, summaries[0].database_type);
        assert_eq!(Some("postgres".to_string()), summaries[0].database);
    }

    #[tokio::test]
    async fn direct_table_metadata_uses_one_session_for_all_table_metadata() {
        let created_configs = Arc::new(StdMutex::new(Vec::new()));
        let metadata_calls = Arc::new(StdMutex::new(Vec::new()));
        let mut state = GlobalDbState::new();
        state.db_manager.postgresql = Arc::new(SlowOpenPlugin::recording_table_metadata(
            created_configs.clone(),
            metadata_calls.clone(),
        ));
        state.register_connection(test_config("direct-table-metadata"));

        let metadata = state
            .load_table_metadata_direct(DirectTableMetadataRequest {
                connection_id: "direct-table-metadata".to_string(),
                database: "app".to_string(),
                schema: Some("public".to_string()),
                table: "widgets".to_string(),
                include_table_metadata: true,
            })
            .await
            .unwrap();

        assert!(metadata.columns.is_empty());
        assert!(metadata.indexes.is_empty());
        assert!(metadata.foreign_keys.is_empty());
        assert_eq!(1, created_configs.lock().unwrap().len());
        assert_eq!(
            &["columns", "indexes", "foreign_keys"],
            metadata_calls.lock().unwrap().as_slice()
        );
    }

    #[tokio::test]
    async fn direct_view_metadata_skips_table_only_queries() {
        let created_configs = Arc::new(StdMutex::new(Vec::new()));
        let metadata_calls = Arc::new(StdMutex::new(Vec::new()));
        let mut state = GlobalDbState::new();
        state.db_manager.postgresql = Arc::new(SlowOpenPlugin::recording_table_metadata(
            created_configs.clone(),
            metadata_calls.clone(),
        ));
        state.register_connection(test_config("direct-view-metadata"));

        state
            .load_table_metadata_direct(DirectTableMetadataRequest {
                connection_id: "direct-view-metadata".to_string(),
                database: "app".to_string(),
                schema: Some("public".to_string()),
                table: "widget_view".to_string(),
                include_table_metadata: false,
            })
            .await
            .unwrap();

        assert_eq!(1, created_configs.lock().unwrap().len());
        assert_eq!(&["columns"], metadata_calls.lock().unwrap().as_slice());
    }

    #[test]
    fn streaming_invalidation_is_conservative_only_when_execution_state_is_ambiguous() {
        let script = SqlSource::Script("CREATE TABLE widgets (id INT)".to_string());
        let file = SqlSource::File(PathBuf::from("streaming-cache-test.sql"));

        assert!(!should_conservatively_invalidate_streaming(
            &file,
            false,
            StreamingExecutionOutcome::Success,
            false,
        ));
        assert!(should_conservatively_invalidate_streaming(
            &file,
            false,
            StreamingExecutionOutcome::Success,
            true,
        ));
        assert!(should_conservatively_invalidate_streaming(
            &file,
            false,
            StreamingExecutionOutcome::Cancelled,
            false,
        ));
        assert!(!should_conservatively_invalidate_streaming(
            &script,
            false,
            StreamingExecutionOutcome::Error,
            true,
        ));
        assert!(!should_conservatively_invalidate_streaming(
            &script,
            false,
            StreamingExecutionOutcome::Cancelled,
            true,
        ));
        assert!(!should_conservatively_invalidate_streaming(
            &script,
            true,
            StreamingExecutionOutcome::Success,
            true,
        ));
        assert!(should_conservatively_invalidate_streaming(
            &script,
            true,
            StreamingExecutionOutcome::Error,
            true,
        ));
        assert!(should_conservatively_invalidate_streaming(
            &script,
            true,
            StreamingExecutionOutcome::Cancelled,
            false,
        ));
    }

    #[tokio::test]
    async fn streaming_ddl_invalidates_cached_schema_metadata() {
        let test = setup_streaming_cache_test("streaming-ddl-cache-test").await;
        let tables_cached = run_streaming_source(
            &test,
            SqlSource::Script("CREATE TABLE widgets (id INT)".to_string()),
        )
        .await;

        assert!(
            !tables_cached,
            "schema metadata must already be invalidated when streaming progress closes"
        );
    }

    #[tokio::test]
    async fn streaming_query_keeps_schema_metadata_cache() {
        let test = setup_streaming_cache_test("streaming-query-cache-test").await;
        let tables_cached =
            run_streaming_source(&test, SqlSource::Script("SELECT 1".to_string())).await;

        assert!(
            tables_cached,
            "non-DDL streaming queries must not invalidate schema metadata"
        );
    }

    #[tokio::test]
    async fn streaming_file_conservatively_invalidates_connection_cache() {
        let test = setup_streaming_cache_test("streaming-file-cache-test").await;
        let tables_cached = run_streaming_source(
            &test,
            SqlSource::File(PathBuf::from("streaming-cache-test.sql")),
        )
        .await;

        assert!(
            !tables_cached,
            "SQL files may contain DDL, so connection metadata must be invalidated"
        );
    }

    #[tokio::test]
    async fn nontransactional_streaming_only_invalidates_confirmed_successful_ddl() {
        let test = setup_streaming_cache_test_with_connection(
            "streaming-partial-ddl-cache-test",
            |config| {
                MockConnection::with_streaming_results(
                    config,
                    vec![
                        SqlResult::Exec(ExecResult {
                            sql: "CREATE TABLE public.widgets (id INT)".to_string(),
                            rows_affected: 0,
                            elapsed_ms: 0,
                            message: None,
                        }),
                        SqlResult::Error(SqlErrorInfo {
                            sql: "CREATE TABLE audit.future_table (id INT)".to_string(),
                            message: "simulated failure".to_string(),
                        }),
                    ],
                )
            },
        )
        .await;
        test.cache
            .cache_tables(&test.config.id, "postgres", Some("audit"), Vec::new())
            .await;

        let public_cached = run_streaming_source(
            &test,
            SqlSource::Script(
                "CREATE TABLE public.widgets (id INT); \
                 CREATE TABLE audit.future_table (id INT)"
                    .to_string(),
            ),
        )
        .await;

        let audit_cached = test
            .cache
            .get_tables(&test.config.id, "postgres", Some("audit"))
            .await
            .is_some();

        assert!(
            !public_cached,
            "confirmed successful DDL must invalidate its schema"
        );
        assert!(
            audit_cached,
            "failed or unexecuted DDL must not invalidate unrelated metadata"
        );
    }

    #[tokio::test]
    async fn failed_transactional_streaming_conservatively_invalidates_connection_cache() {
        let test = setup_streaming_cache_test_with_connection(
            "streaming-transaction-error-cache-test",
            |config| {
                MockConnection::with_streaming_results(
                    config,
                    vec![
                        SqlResult::Exec(ExecResult {
                            sql: "CREATE TABLE public.widgets (id INT)".to_string(),
                            rows_affected: 0,
                            elapsed_ms: 0,
                            message: None,
                        }),
                        SqlResult::Error(SqlErrorInfo {
                            sql: "CREATE TABLE audit.future_table (id INT)".to_string(),
                            message: "simulated failure".to_string(),
                        }),
                    ],
                )
            },
        )
        .await;
        test.cache
            .cache_tables(&test.config.id, "postgres", Some("audit"), Vec::new())
            .await;

        let public_cached = run_streaming_source_with_options(
            &test,
            SqlSource::Script(
                "CREATE TABLE public.widgets (id INT); \
                 CREATE TABLE audit.future_table (id INT)"
                    .to_string(),
            ),
            Some(ExecOptions {
                transactional: true,
                ..Default::default()
            }),
        )
        .await;

        let audit_cached = test
            .cache
            .get_tables(&test.config.id, "postgres", Some("audit"))
            .await
            .is_some();

        assert!(!public_cached);
        assert!(
            !audit_cached,
            "a failed transaction after successful progress has an ambiguous final state"
        );
    }

    #[tokio::test]
    async fn cancelling_streaming_execution_drops_query_and_closes_session() {
        let state = GlobalDbState::new();
        let config = test_config("streaming-cancel-test");

        let disconnect_count = Arc::new(AtomicUsize::new(0));
        let streaming_started = Arc::new(AtomicBool::new(false));
        let streaming_dropped = Arc::new(AtomicBool::new(false));

        let session = ConnectionSession::new(
            Box::new(MockConnection::with_blocking_streaming(
                config.clone(),
                disconnect_count.clone(),
                streaming_started.clone(),
                streaming_dropped.clone(),
            )),
            format!("{}:session:1", config.id),
            false,
        );
        state
            .connection_manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::new(session));

        let cancellation = CancellationToken::new();
        let (tx, mut progress) = mpsc::channel(1);
        let source = SqlSource::Script("SELECT pg_sleep(60)".to_string());
        let request = StreamingExecutionRequest {
            state: state.clone(),
            config: config.clone(),
            source: Some(source.clone()),
            invalidation_source: source,
            schema: Some("public".to_string()),
            opts: ExecOptions::default(),
            tx,
            cache: None,
            cancellation: cancellation.clone(),
        };
        let execution = tokio::spawn(request.run());

        tokio::time::timeout(Duration::from_secs(2), async {
            while !streaming_started.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("streaming execution should start");

        cancellation.cancel();

        tokio::time::timeout(Duration::from_secs(2), execution)
            .await
            .expect("cancelled streaming execution should finish")
            .expect("streaming execution task should not panic");
        assert!(
            progress.recv().await.is_none(),
            "cancelled streaming progress should close"
        );

        assert!(
            streaming_dropped.load(Ordering::SeqCst),
            "cancellation must drop the in-flight driver future"
        );
        assert_eq!(
            1,
            disconnect_count.load(Ordering::SeqCst),
            "cancellation must disconnect the temporary session"
        );
        assert!(
            state
                .connection_manager
                .list_sessions(&config.id)
                .await
                .is_empty(),
            "cancelled temporary session must be removed from the pool"
        );
    }

    #[tokio::test]
    async fn execute_session_uses_existing_connection_session() {
        let state = GlobalDbState::new();
        let config = test_config("conn1");
        let session_id = "conn1:session:test".to_string();
        let executed_sql = Arc::new(StdMutex::new(Vec::new()));
        let session = ConnectionSession::new(
            Box::new(MockConnection::with_executed_sql(
                config.clone(),
                executed_sql.clone(),
            )),
            session_id.clone(),
            false,
        );
        session.mark_in_use();
        state
            .connection_manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::new(session));

        let result = state
            .execute_session(session_id.clone(), "select 1".to_string(), None)
            .await
            .unwrap();

        assert_eq!(1, result.len());
        assert_eq!(vec!["select 1".to_string()], *executed_sql.lock().unwrap());
        assert!(
            state
                .connection_manager
                .get_session_config(&session_id)
                .await
                .is_some()
        );
    }

    async fn insert_test_session(
        manager: &ConnectionManager,
        config: &DbConnectionConfig,
        session_id: &str,
        connection: MockConnection,
    ) {
        manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::new(ConnectionSession::new(
                Box::new(connection),
                session_id.to_string(),
                false,
            )));
    }

    /// 一个 session 正在执行时，另一个 session 仍必须能拿到自己的连接。
    /// 历史行为是整张会话表被写锁占住，导致 A 页签执行时 B 页签（哪怕连的是
    /// 另一台库）完全无法执行。
    #[tokio::test]
    async fn session_connection_guards_do_not_serialize_different_sessions() {
        let manager = ConnectionManager::new();
        let first = test_config("session-lock-first");
        let second = test_config("session-lock-second");
        let first_id = "session-lock-first:session:1";
        let second_id = "session-lock-second:session:1";

        insert_test_session(
            &manager,
            &first,
            first_id,
            MockConnection::new(first.clone(), true),
        )
        .await;
        insert_test_session(
            &manager,
            &second,
            second_id,
            MockConnection::new(second.clone(), true),
        )
        .await;

        let mut first_guard = manager.get_session_connection(first_id).await.unwrap();
        assert!(first_guard.connection().is_some());

        let mut second_guard = tokio::time::timeout(
            Duration::from_millis(200),
            manager.get_session_connection(second_id),
        )
        .await
        .expect("another session must not wait for the first session's statement")
        .expect("second session should be found");
        assert!(second_guard.connection().is_some());

        assert!(
            tokio::time::timeout(
                Duration::from_millis(200),
                manager.get_session_connection(first_id),
            )
            .await
            .is_err(),
            "the same session must stay exclusive while a statement runs"
        );
    }

    /// 页签 A 的长查询不得阻塞页签 B 的执行（端到端版本）。
    #[tokio::test]
    async fn executing_on_one_session_does_not_block_another_session() {
        let state = GlobalDbState::new();
        let blocked = test_config("blocked-conn");
        let free = test_config("free-conn");
        let blocked_id = "blocked-conn:session:1";
        let free_id = "free-conn:session:1";

        let execution_started = Arc::new(AtomicBool::new(false));
        let execution_dropped = Arc::new(AtomicBool::new(false));
        let executed_sql = Arc::new(StdMutex::new(Vec::new()));

        state
            .connection_manager
            .sessions
            .write()
            .await
            .entry(blocked.id.clone())
            .or_default()
            .push(Arc::new(ConnectionSession::new(
                Box::new(MockConnection::with_blocking_execution(
                    blocked.clone(),
                    Arc::new(AtomicUsize::new(0)),
                    execution_started.clone(),
                    execution_dropped.clone(),
                )),
                blocked_id.to_string(),
                false,
            )));
        state
            .connection_manager
            .sessions
            .write()
            .await
            .entry(free.id.clone())
            .or_default()
            .push(Arc::new(ConnectionSession::new(
                Box::new(MockConnection::with_executed_sql(
                    free.clone(),
                    executed_sql.clone(),
                )),
                free_id.to_string(),
                false,
            )));

        let blocked_state = state.clone();
        let blocked_execution = tokio::spawn(async move {
            blocked_state
                .execute_session(
                    blocked_id.to_string(),
                    "SELECT pg_sleep(60)".to_string(),
                    None,
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !execution_started.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("blocked session execution should start");

        let result = tokio::time::timeout(
            Duration::from_millis(500),
            state.execute_session(free_id.to_string(), "select 1".to_string(), None),
        )
        .await
        .expect("a statement on another session must run while the first one is executing")
        .expect("second session execution should succeed");
        assert_eq!(1, result.len());
        assert_eq!(vec!["select 1".to_string()], *executed_sql.lock().unwrap());

        blocked_execution.abort();
        assert!(blocked_execution.await.unwrap_err().is_cancelled());
        assert!(execution_dropped.load(Ordering::SeqCst));
    }

    /// 一个语句的释放不能清掉下一条语句对同一个 session 的占用。
    /// 占用必须按计数维护：释放与下一次获取会交错，布尔量会把正在执行的
    /// session 记为空闲，进而被清理任务回收。
    #[tokio::test]
    async fn overlapping_release_keeps_the_next_statement_claim() {
        let session = ConnectionSession::new(
            Box::new(MockConnection::new(test_config("claim"), true)),
            "claim:session:1".to_string(),
            false,
        );

        session.mark_in_use();
        session.mark_in_use();
        session.release();
        assert!(
            session.in_use(),
            "finishing the first statement must not clear the claim of the next one"
        );

        session.release();
        assert!(!session.in_use());
    }

    /// 一次语句只占用一个单位：取连接时占用，释放后退回空闲并重新可回收。
    #[tokio::test]
    async fn a_statement_claims_and_releases_a_session_exactly_once() {
        let manager = ConnectionManager::new();
        let config = test_config("balanced-claim");
        let session_id = "balanced-claim:session:1";
        let session = Arc::new(ConnectionSession::new(
            Box::new(MockConnection::new(config.clone(), true)),
            session_id.to_string(),
            false,
        ));
        manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::clone(&session));

        let guard = manager.get_session_connection(session_id).await.unwrap();
        assert!(session.in_use());
        drop(guard);
        manager.release_session(session_id).await.unwrap();

        assert!(
            !session.in_use(),
            "a released session must go back to idle so it can be reused"
        );
        assert!(
            session.is_expired(Duration::ZERO),
            "a released session must be eligible for idle cleanup"
        );

        // 复用成功后再走一遍语句的占用/释放，占用计数必须重新回到 0。
        let acquired = manager.try_acquire_session(&config).await.unwrap();
        assert_eq!(Some(session_id.to_string()), acquired);

        let guard = manager.get_session_connection(session_id).await.unwrap();
        assert!(session.in_use());
        drop(guard);
        manager.release_session(session_id).await.unwrap();
        assert!(
            !session.in_use(),
            "reusing a session must not leave a claim behind"
        );
    }

    /// 释放任务先排队等连接锁、下一条语句随后拿到连接时，session 不能被记为 idle。
    #[tokio::test]
    async fn releasing_a_session_does_not_report_the_next_statement_as_idle() {
        let manager = ConnectionManager::new();
        let config = test_config("claim-race");
        let session_id = "claim-race:session:1";

        let session = Arc::new(ConnectionSession::new(
            Box::new(MockConnection::new(config.clone(), true)),
            session_id.to_string(),
            false,
        ));
        manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::clone(&session));

        let mut first = manager.get_session_connection(session_id).await.unwrap();
        assert!(first.connection().is_some());

        // 释放任务先进连接锁队列；连接锁是 FIFO，sleep 只用来固定队列顺序。
        let release = {
            let manager = manager.clone();
            let session_id = session_id.to_string();
            tokio::spawn(async move { manager.release_session(&session_id).await })
        };
        sleep(Duration::from_millis(20)).await;

        // 下一条语句随后取同一 session：先标记占用，然后排在释放之后拿到连接。
        let next = {
            let manager = manager.clone();
            let session_id = session_id.to_string();
            tokio::spawn(async move { manager.get_session_connection(&session_id).await })
        };
        sleep(Duration::from_millis(20)).await;

        drop(first);

        let mut next = tokio::time::timeout(Duration::from_secs(2), next)
            .await
            .expect("next statement should not hang")
            .expect("join")
            .expect("next statement should get the connection");
        assert!(next.connection().is_some());
        tokio::time::timeout(Duration::from_secs(2), release)
            .await
            .expect("release should not hang")
            .expect("join")
            .expect("release should succeed");

        // 直接读 session 状态：读会话表需要写锁，旧实现里会被 guard 挡住而挂住。
        assert!(
            session.in_use(),
            "a session holding a connection for a statement must not be reported as idle"
        );
        drop(next);
    }

    /// 复用会话时不得在持有会话表锁的情况下等待某条会话的连接锁，
    /// 否则一个正在跑长查询的会话会重新堵住所有页签（含其它连接）。
    #[tokio::test]
    async fn try_acquire_session_does_not_block_the_pool_while_a_session_is_busy() {
        let manager = ConnectionManager::new();
        let busy = test_config("busy-config");
        let other = test_config("other-config");
        let busy_id = "busy-config:session:1";
        let other_id = "other-config:session:1";

        insert_test_session(
            &manager,
            &busy,
            busy_id,
            MockConnection::new(busy.clone(), true),
        )
        .await;
        insert_test_session(
            &manager,
            &other,
            other_id,
            MockConnection::new(other.clone(), true),
        )
        .await;

        // busy 会话正在执行：它的连接锁被长期持有。
        let mut busy_guard = manager.get_session_connection(busy_id).await.unwrap();
        assert!(busy_guard.connection().is_some());

        let acquire = {
            let manager = manager.clone();
            let config = busy.clone();
            tokio::spawn(async move { manager.try_acquire_session(&config).await })
        };
        sleep(Duration::from_millis(20)).await;

        // 复用扫描进行中，其它连接仍必须能取到自己的会话。
        tokio::time::timeout(
            Duration::from_millis(300),
            manager.get_session_connection(other_id),
        )
        .await
        .expect("a busy session must not block the pool for other connections")
        .expect("other session should be found");

        let acquired = tokio::time::timeout(Duration::from_secs(2), acquire)
            .await
            .expect("acquire should not block on a busy session's connection lock")
            .expect("join")
            .expect("acquire");
        assert_eq!(
            None, acquired,
            "a session that is executing a statement must not be handed out again"
        );

        drop(busy_guard);
        manager.release_session(busy_id).await.unwrap();

        let acquired =
            tokio::time::timeout(Duration::from_secs(2), manager.try_acquire_session(&busy))
                .await
                .expect("acquire should not hang")
                .expect("acquire");
        assert_eq!(Some(busy_id.to_string()), acquired);
    }

    /// 并发释放与关闭同一个 session 时，物理连接只能断开一次。
    #[tokio::test]
    async fn concurrent_release_and_close_disconnect_the_session_once() {
        let manager = ConnectionManager::new();
        let config = test_config("double-close");
        let session_id = "double-close:session:1";
        let disconnect_count = Arc::new(AtomicUsize::new(0));

        manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::new(ConnectionSession::new(
                Box::new(MockConnection::with_disconnect_count(
                    config.clone(),
                    true,
                    Arc::clone(&disconnect_count),
                )),
                session_id.to_string(),
                true,
            )));

        let mut guard = manager.get_session_connection(session_id).await.unwrap();
        assert!(guard.connection().is_some());

        // 释放任务先排队等连接锁，随后 close_session 把同一个 session 摘走。
        let release = {
            let manager = manager.clone();
            let session_id = session_id.to_string();
            tokio::spawn(async move { manager.release_session(&session_id).await })
        };
        sleep(Duration::from_millis(20)).await;

        let close = {
            let manager = manager.clone();
            let session_id = session_id.to_string();
            tokio::spawn(async move { manager.close_session(&session_id).await })
        };
        sleep(Duration::from_millis(20)).await;

        drop(guard);

        tokio::time::timeout(Duration::from_secs(2), release)
            .await
            .expect("release should not hang")
            .expect("join")
            .expect("release should succeed");
        tokio::time::timeout(Duration::from_secs(2), close)
            .await
            .expect("close should not hang")
            .expect("join")
            .expect("close should succeed");

        assert_eq!(
            1,
            disconnect_count.load(Ordering::SeqCst),
            "the physical connection must be disconnected exactly once"
        );
        assert!(manager.list_sessions(&config.id).await.is_empty());
    }

    #[tokio::test]
    async fn closing_an_executing_session_waits_for_cancellation_and_disconnects_once() {
        let state = GlobalDbState::new();
        let config = test_config("session-cancel-test");
        let session_id = "session-cancel-test:session:1".to_string();
        let disconnect_count = Arc::new(AtomicUsize::new(0));
        let execution_started = Arc::new(AtomicBool::new(false));
        let execution_dropped = Arc::new(AtomicBool::new(false));
        let session = ConnectionSession::new(
            Box::new(MockConnection::with_blocking_execution(
                config.clone(),
                disconnect_count.clone(),
                execution_started.clone(),
                execution_dropped.clone(),
            )),
            session_id.clone(),
            false,
        );
        session.mark_in_use();
        state
            .connection_manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::new(session));

        let execution_state = state.clone();
        let execution_session_id = session_id.clone();
        let execution = tokio::spawn(async move {
            execution_state
                .execute_session(
                    execution_session_id,
                    "SELECT pg_sleep(60)".to_string(),
                    None,
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !execution_started.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session execution should start");

        let close_state = state.clone();
        let close_session_id = session_id.clone();
        let mut close =
            tokio::spawn(async move { close_state.close_session_direct(&close_session_id).await });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut close)
                .await
                .is_err(),
            "close must wait for the active execution"
        );
        assert_eq!(0, disconnect_count.load(Ordering::SeqCst));

        execution.abort();
        assert!(execution.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(2), close)
            .await
            .expect("close should finish after execution cancellation")
            .expect("close task should not panic")
            .expect("session close should succeed");

        assert!(execution_dropped.load(Ordering::SeqCst));
        assert_eq!(1, disconnect_count.load(Ordering::SeqCst));
        assert!(
            state
                .connection_manager
                .list_sessions(&config.id)
                .await
                .is_empty()
        );
        assert!(state.close_session_direct(&session_id).await.is_err());
        assert_eq!(1, disconnect_count.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn export_data_opens_session_for_selected_database() {
        let mut state = GlobalDbState::new();
        let config = test_config("export-selected-database");
        state.register_connection(config);

        let created_configs = Arc::new(StdMutex::new(Vec::new()));
        state.db_manager.postgresql =
            Arc::new(SlowOpenPlugin::recording_postgres(created_configs.clone()));

        state
            .export_data_with_progress_on_tokio(ExportProgressRequest {
                connection_id: "export-selected-database".to_string(),
                config: ExportConfig {
                    database: "target_database".to_string(),
                    ..ExportConfig::default()
                },
                progress_tx: None,
            })
            .await
            .expect("export should use a session connected to the selected database");

        assert_eq!(
            vec![Some("target_database".to_string())],
            created_configs
                .lock()
                .unwrap()
                .iter()
                .map(|config| config.database.clone())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn export_data_preserves_connection_database_when_not_selected() {
        let mut state = GlobalDbState::new();
        let config = test_config("export-default-database");
        state.register_connection(config);

        let created_configs = Arc::new(StdMutex::new(Vec::new()));
        state.db_manager.postgresql =
            Arc::new(SlowOpenPlugin::recording_postgres(created_configs.clone()));

        state
            .export_data_with_progress_on_tokio(ExportProgressRequest {
                connection_id: "export-default-database".to_string(),
                config: ExportConfig::default(),
                progress_tx: None,
            })
            .await
            .expect("export should preserve the connection's default database");

        assert_eq!(
            vec![Some("postgres".to_string())],
            created_configs
                .lock()
                .unwrap()
                .iter()
                .map(|config| config.database.clone())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn switch_session_schema_uses_existing_connection_session() {
        let state = GlobalDbState::new();
        let config = test_config("conn1");
        let session_id = "conn1:session:test".to_string();
        let switched_schemas = Arc::new(StdMutex::new(Vec::new()));
        let session = ConnectionSession::new(
            Box::new(MockConnection::with_switched_schemas(
                config.clone(),
                switched_schemas.clone(),
            )),
            session_id.clone(),
            false,
        );
        session.mark_in_use();
        state
            .connection_manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::new(session));

        state
            .switch_session_schema(session_id, "analytics".to_string())
            .await
            .unwrap();

        assert_eq!(
            vec!["analytics".to_string()],
            *switched_schemas.lock().unwrap()
        );
    }

    #[test]
    fn test_cached_children_ready_blocks_unloaded_children() {
        let node = DbNode::new(
            "node-id",
            "node",
            DbNodeType::Table,
            "conn-id".to_string(),
            DatabaseType::SQLite,
        );

        assert!(!GlobalDbState::cached_children_ready(&node));
    }

    #[tokio::test]
    async fn ping_returns_error_for_sql_error_result() {
        let connection = MockConnection::new(test_config("conn1"), false);

        assert!(connection.ping().await.is_err());
    }

    #[tokio::test]
    async fn try_acquire_session_discards_idle_session_when_ping_fails() {
        let manager =
            ConnectionManager::with_config(Duration::from_secs(300), Duration::from_secs(1800));
        let config = test_config("conn1");
        let session = ConnectionSession::new(
            Box::new(MockConnection::new(config.clone(), false)),
            "conn1:session:1".to_string(),
            false,
        );

        manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::new(session));

        let acquired = manager.try_acquire_session(&config).await.unwrap();
        let remaining = manager.list_sessions(&config.id).await;

        assert!(acquired.is_none());
        assert!(remaining.is_empty());
    }

    /// 空闲期被中间设备丢弃的连接：复用时的 ping 永远不返回。
    /// 复用路径必须在有界时间内判死并丢弃会话，而不是永远等待。
    #[tokio::test]
    async fn try_acquire_session_times_out_a_hung_ping_and_discards_the_session() {
        let manager =
            ConnectionManager::with_config(Duration::from_secs(300), Duration::from_secs(1800));
        let config = test_config("dead-socket");
        let session = ConnectionSession::new(
            Box::new(MockConnection::with_dead_socket(config.clone())),
            "dead-socket:session:1".to_string(),
            false,
        );

        manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::new(session));

        let acquired = tokio::time::timeout(
            Duration::from_secs(20),
            manager.try_acquire_session(&config),
        )
        .await
        .expect("reuse validation must not hang on a dead socket")
        .unwrap();
        let remaining = manager.list_sessions(&config.id).await;

        assert!(acquired.is_none());
        assert!(remaining.is_empty(), "dead session must be discarded");
    }

    /// 关闭挂死的连接也必须有界：清理/退出路径不能被一个不回包的 disconnect 卡住。
    #[tokio::test]
    async fn close_times_out_a_hung_disconnect() {
        let config = test_config("dead-socket-close");
        let session = ConnectionSession::new(
            Box::new(MockConnection::with_dead_socket(config.clone())),
            "dead-socket-close:session:1".to_string(),
            false,
        );

        tokio::time::timeout(Duration::from_secs(20), session.close())
            .await
            .expect("closing a dead socket must be bounded");
    }

    #[tokio::test]
    async fn create_session_resolves_ssh_reference_before_opening_connection() {
        let manager = ConnectionManager::new();
        let mut config = test_config("conn-with-ssh-ref");
        config
            .extra_params
            .insert("ssh_connection_id".to_string(), "42".to_string());

        let error = manager
            .create_session(config, &DbManager::new())
            .await
            .expect_err("missing repository should fail during config resolution");

        assert!(
            error
                .to_string()
                .contains("ConnectionRepository is unavailable")
        );
    }

    #[test]
    fn single_file_lifecycle_uses_declared_file_path_fields() {
        let mut config = test_config("duckdb-path");
        config.database_type = DatabaseType::DuckDB;
        config.host = "file:/tmp/duckdb-path.duckdb".to_string();
        config.database = Some("main".to_string());

        assert_eq!(
            Some("duckdb:/tmp/duckdb-path.duckdb".to_string()),
            ConnectionLifecycle::single_file(
                "duckdb",
                &config,
                &["host".to_string(), "database".to_string()]
            )
            .physical_open_lock_key
        );
    }

    #[test]
    fn single_file_lifecycle_can_read_extra_params_path() {
        let mut config = test_config("external-duckdb-path");
        config.database_type = DatabaseType::external("singlefile");
        config.host.clear();
        config.database = None;
        config.extra_params.insert(
            "path".to_string(),
            "/tmp/external-duckdb-path.duckdb".to_string(),
        );

        assert_eq!(
            Some("singlefile:/tmp/external-duckdb-path.duckdb".to_string()),
            ConnectionLifecycle::single_file(
                "singlefile",
                &config,
                &["extra_params.path".to_string()]
            )
            .physical_open_lock_key
        );
    }

    #[tokio::test]
    async fn release_session_closes_duckdb_sessions_instead_of_idling() {
        let manager =
            ConnectionManager::with_config(Duration::from_secs(300), Duration::from_secs(1800));
        let mut config = test_config("duckdb-conn");
        config.database_type = DatabaseType::DuckDB;
        let disconnect_count = Arc::new(AtomicUsize::new(0));
        let session = ConnectionSession::new(
            Box::new(MockConnection::with_disconnect_count(
                config.clone(),
                true,
                Arc::clone(&disconnect_count),
            )),
            "duckdb-conn:session:1".to_string(),
            true,
        );
        session.mark_in_use();

        manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::new(session));

        manager
            .release_session("duckdb-conn:session:1")
            .await
            .unwrap();

        assert_eq!(1, disconnect_count.load(Ordering::SeqCst));
        assert!(manager.list_sessions(&config.id).await.is_empty());
    }

    #[tokio::test]
    async fn release_session_for_reuse_keeps_duckdb_transaction_session_idle() {
        let manager =
            ConnectionManager::with_config(Duration::from_secs(300), Duration::from_secs(1800));
        let mut config = test_config("duckdb-transaction");
        config.database_type = DatabaseType::DuckDB;
        let disconnect_count = Arc::new(AtomicUsize::new(0));
        let session = ConnectionSession::new(
            Box::new(MockConnection::with_disconnect_count(
                config.clone(),
                true,
                Arc::clone(&disconnect_count),
            )),
            "duckdb-transaction:session:1".to_string(),
            true,
        );
        session.mark_in_use();

        manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::new(session));

        manager
            .release_session_for_reuse("duckdb-transaction:session:1")
            .await
            .unwrap();

        let sessions = manager.list_sessions(&config.id).await;
        assert_eq!(0, disconnect_count.load(Ordering::SeqCst));
        assert_eq!(1, sessions.len());
        assert!(!sessions[0].in_use);
    }

    #[tokio::test]
    async fn try_acquire_session_waits_for_busy_close_on_release_session_before_returning_none() {
        let manager =
            ConnectionManager::with_config(Duration::from_secs(300), Duration::from_secs(1800));
        let mut config = test_config("duckdb-busy");
        config.database_type = DatabaseType::DuckDB;
        let session = ConnectionSession::new(
            Box::new(MockConnection::new(config.clone(), true)),
            "duckdb-busy:session:1".to_string(),
            true,
        );
        session.mark_in_use();

        manager
            .sessions
            .write()
            .await
            .entry(config.id.clone())
            .or_default()
            .push(Arc::new(session));

        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let release_manager = manager.clone();
        tokio::spawn(async move {
            release_rx.await.unwrap();
            release_manager
                .close_session("duckdb-busy:session:1")
                .await
                .unwrap();
        });

        let acquire_manager = manager.clone();
        let acquire_config = config.clone();
        let acquire_task =
            tokio::spawn(async move { acquire_manager.try_acquire_session(&acquire_config).await });

        sleep(Duration::from_millis(20)).await;
        assert!(
            !acquire_task.is_finished(),
            "busy close-on-release session should not allow opening a second physical connection"
        );

        release_tx.send(()).unwrap();
        let acquired = acquire_task.await.unwrap().unwrap();

        assert!(acquired.is_none());
    }

    #[tokio::test]
    async fn create_session_serializes_concurrent_duckdb_physical_opens() {
        let manager =
            ConnectionManager::with_config(Duration::from_secs(300), Duration::from_secs(1800));
        let mut db_manager = DbManager::new();
        let active_opens = Arc::new(AtomicUsize::new(0));
        let max_active_opens = Arc::new(AtomicUsize::new(0));
        let open_started = Arc::new(tokio::sync::Notify::new());
        db_manager.duckdb = Arc::new(SlowOpenPlugin::new(
            Arc::clone(&active_opens),
            Arc::clone(&max_active_opens),
            Arc::clone(&open_started),
        ));

        let mut config = test_config("duckdb-concurrent-open");
        config.database_type = DatabaseType::DuckDB;
        config.host = "/tmp/duckdb-concurrent-open.duckdb".to_string();
        config.database = Some("main".to_string());

        let first_manager = manager.clone();
        let first_db_manager = db_manager.clone();
        let first_config = config.clone();
        let first = tokio::spawn(async move {
            first_manager
                .create_session(first_config, &first_db_manager)
                .await
        });
        open_started.notified().await;

        let second_manager = manager.clone();
        let second_db_manager = db_manager.clone();
        let second_config = config.clone();
        let second = tokio::spawn(async move {
            second_manager
                .create_session(second_config, &second_db_manager)
                .await
        });

        let first_session = first.await.unwrap().unwrap();
        sleep(Duration::from_millis(20)).await;
        assert_eq!(
            1,
            max_active_opens.load(Ordering::SeqCst),
            "same DuckDB file should not be opened by two physical connections concurrently"
        );

        manager.close_session(&first_session).await.unwrap();
        let second_session = second.await.unwrap().unwrap();
        manager.close_session(&second_session).await.unwrap();

        assert_eq!(1, max_active_opens.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn create_session_serializes_plugin_declared_single_file_physical_opens() {
        let manager =
            ConnectionManager::with_config(Duration::from_secs(300), Duration::from_secs(1800));
        let mut db_manager = DbManager::new();
        let active_opens = Arc::new(AtomicUsize::new(0));
        let max_active_opens = Arc::new(AtomicUsize::new(0));
        let open_started = Arc::new(tokio::sync::Notify::new());
        db_manager.external_drivers.insert(
            "singlefile".to_string(),
            Arc::new(SlowOpenPlugin::with_lifecycle(
                DatabaseType::external("singlefile"),
                ConnectionLifecycle {
                    close_on_release: true,
                    physical_open_lock_key: Some("singlefile:/tmp/shared.db".to_string()),
                },
                Arc::clone(&active_opens),
                Arc::clone(&max_active_opens),
                Arc::clone(&open_started),
            )),
        );

        let mut config = test_config("singlefile-concurrent-open");
        config.database_type = DatabaseType::external("singlefile");
        config.host = "/tmp/shared.db".to_string();
        config.database = Some("main".to_string());

        let first_manager = manager.clone();
        let first_db_manager = db_manager.clone();
        let first_config = config.clone();
        let first = tokio::spawn(async move {
            first_manager
                .create_session(first_config, &first_db_manager)
                .await
        });
        open_started.notified().await;

        let second_manager = manager.clone();
        let second_db_manager = db_manager.clone();
        let second_config = config.clone();
        let second = tokio::spawn(async move {
            second_manager
                .create_session(second_config, &second_db_manager)
                .await
        });

        let first_session = first.await.unwrap().unwrap();
        sleep(Duration::from_millis(20)).await;
        assert_eq!(
            1,
            max_active_opens.load(Ordering::SeqCst),
            "single-file drivers should be serialized by plugin-declared lifecycle"
        );

        manager.close_session(&first_session).await.unwrap();
        let second_session = second.await.unwrap().unwrap();
        manager.close_session(&second_session).await.unwrap();

        assert_eq!(1, max_active_opens.load(Ordering::SeqCst));
    }
}

#[cfg(test)]
#[path = "manager_runtime_contract_tests.rs"]
mod runtime_contract_tests;
