use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use gpui::http_client::HttpClient;
use gpui::{App, AsyncApp, Global, Subscription};
use one_core::cloud_sync::personal::{
    ConfiguredPersonalSyncStore, PersonalSyncConflict, PersonalSyncConflictRepository,
    PersonalSyncConflictResolver, PersonalSyncEvent, PersonalSyncLocalRepositorySource,
    PersonalSyncRuntimeConfig, PersonalSyncRuntimeError, PersonalSyncStore, PersonalSyncWatcher,
    PersonalSyncWorker, SqlitePersonalSyncConflictSink, SyncDeviceId, SyncStoreError,
    SyncStoreHealth, WorkerConfig, build_personal_sync_runtime_config, open_webdav_password,
};
use one_core::cloud_sync::scope::personal::SyncScopeSource;
use one_core::cloud_sync::scope::settings_payload::{SettingsGroup, extract_payload};
use one_core::cloud_sync::scope::state::{SyncItemStateRepository, SyncTypeStatusRepository};
use one_core::cloud_sync::{CloudSyncData, CloudSyncService, ConflictResolution, data_type};
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::crypto;
use one_core::gpui_tokio::Tokio;
use one_core::llm::storage::ProviderRepository;
use one_core::settings::{
    AppSettings, GlobalCurrentUser, PersonalSyncSettings, SyncProvider, SYNC_SCOPE_DESCRIPTORS,
};
use one_core::storage::quick_command::QuickCommandRepository;
use one_core::storage::sftp_favorite_path::SftpFavoritePathRepository;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, CredentialRepository, CredentialSummary, DatabaseType,
    GlobalStorageState, StorageManager, StoredConnection, Workspace, WorkspaceRepository,
};
use rust_i18n::t;

use std::collections::{HashMap, HashSet};

use crate::personal_sync_status::PersonalSyncRuntimeStatus;

/// 两次自动同步之间的最小间隔。
///
/// 变更到达时如果距上次同步不足十秒，就延迟到满十秒再跑，把这十秒内的
/// 所有变更合并成一轮——既避免「改一个设置就传一次」，也让多设备错开写入。
/// 十秒是「感知不到等待」与「合并突发变更」的折中：用户改完设置最多等十秒
/// 就会看到同步状态翻新，连续改多项也仍然只传一轮。
const PERSONAL_SYNC_MIN_INTERVAL: Duration = Duration::from_secs(10);

/// 本地长时间无变更时的兜底同步间隔（拉取别的设备的改动）。
const PERSONAL_SYNC_IDLE_INTERVAL: Duration = Duration::from_secs(600);

/// 调度器 tick：只做本地时间与待处理事件比较，不产生任何 IO。
const PERSONAL_SYNC_SCHED_TICK: Duration = Duration::from_secs(1);

/// 本地变更检测的轮询间隔（只读少量 DB 行与文件时间戳，不走网络）。
const SCOPE_CHANGE_POLL_INTERVAL: Duration = Duration::from_secs(3);

pub struct GlobalPersonalSyncRuntime {
    active_config: Option<PersonalSyncRuntimeConfig>,
    runtime: Option<RunningPersonalSyncRuntime>,
    service: Arc<RwLock<CloudSyncService>>,
    status: PersonalSyncRuntimeStatus,
    generation: u64,
    /// 最近一轮同步开始的时刻，用于「最小间隔」合并。
    last_pass_at: Option<Instant>,
    /// 当前生效的同步范围签名；变化说明用户勾选了新类型，必须重建运行时。
    scope_signature: Vec<String>,
    /// 各类自选数据的本地变更签名基线（轮询比对用）。
    scope_baseline: HashMap<String, String>,
    /// 基线尚未建立：首次轮询只建基线，不触发同步。
    scope_baseline_initialized: bool,
    /// 刚跑完一轮：下一轮轮询只刷新基线——否则同步自己写下去的内容会被
    /// 当成新的本地变更，再触发一轮空转。
    scope_baseline_stale: bool,
    _settings_subscription: Subscription,
    _settings_sync_subscription: Subscription,
    _local_event_subscription: Option<Subscription>,
}

impl Global for GlobalPersonalSyncRuntime {}

struct RunningPersonalSyncRuntime {
    store: ConfiguredPersonalSyncStore,
    worker: RunningPersonalSyncWorker,
    /// 「数据范围」扩展源，供变更轮询取轻量签名。
    scope: Option<Arc<SyncScopeSource>>,
    _watcher: Option<PersonalSyncWatcher>,
}

type RunningPersonalSyncWorker = PersonalSyncWorker<
    ConfiguredPersonalSyncStore,
    PersonalSyncLocalRepositorySource,
    SqlitePersonalSyncConflictSink,
>;

pub fn init(cx: &mut App) {
    let settings_subscription = cx.observe_global::<AppSettings>(reconcile_runtime);
    // 设置项被改动后立刻唤醒同步（只针对真正变了的、且已勾选同步的分组）
    cx.set_global(SettingsSyncBaseline::default());
    let settings_sync_subscription = cx.observe_global::<AppSettings>(observe_settings_for_sync);
    let local_event_subscription = subscribe_local_events(cx);
    cx.set_global(GlobalPersonalSyncRuntime {
        active_config: None,
        runtime: None,
        service: Arc::new(RwLock::new(CloudSyncService::new())),
        status: PersonalSyncRuntimeStatus::Disabled,
        generation: 0,
        last_pass_at: None,
        scope_signature: Vec::new(),
        scope_baseline: HashMap::new(),
        scope_baseline_initialized: false,
        scope_baseline_stale: false,
        _settings_subscription: settings_subscription,
        _settings_sync_subscription: settings_sync_subscription,
        _local_event_subscription: local_event_subscription,
    });
    start_personal_sync_scheduler(cx);
    start_scope_change_poller(cx);
    reconcile_runtime(cx);
}

pub fn runtime_status(cx: &App) -> PersonalSyncRuntimeStatus {
    cx.try_global::<GlobalPersonalSyncRuntime>()
        .map(|state| state.status.clone())
        .unwrap_or_default()
}

pub fn actions_enabled(cx: &App) -> bool {
    let settings = AppSettings::global(cx);
    active_personal_sync_settings(settings).is_some_and(|settings| {
        let password = webdav_password_for(&settings);
        build_personal_sync_runtime_config(&settings, password.as_deref()).is_ok()
    })
}

/// 取出 WebDAV 后端的明文密码。
///
/// 只在真正要建连时才解密，且结果不会离开调用栈：runtime config 虽然会带上它，
/// 但那个结构体手写了 `Debug`，日志里只会看到 `<redacted>`。
fn webdav_password_for(settings: &PersonalSyncSettings) -> Option<String> {
    let password = open_webdav_password(&settings.webdav.password);
    (!password.is_empty()).then_some(password)
}

pub fn test_connection(cx: &mut App) {
    let Some(config) = active_or_current_config(cx) else {
        set_status(cx, PersonalSyncRuntimeStatus::Disabled);
        return;
    };
    let generation = begin_operation(cx, PersonalSyncRuntimeStatus::Syncing);
    let http = cx.http_client();
    let task = Tokio::spawn(cx, async move {
        let store = ConfiguredPersonalSyncStore::from_runtime_config(&config, http)?;
        store.probe().await
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let status = match task.await {
            Ok(Ok(status)) => PersonalSyncRuntimeStatus::Ready {
                health: status.health,
                message: status.message,
            },
            Ok(Err(error)) => PersonalSyncRuntimeStatus::from_error(error),
            Err(error) => PersonalSyncRuntimeStatus::failed(&error.to_string()),
        };
        let _ = cx.update(move |cx| finish_operation(cx, generation, status));
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

pub fn sync_now(cx: &mut App) {
    tracing::info!("个人同步：用户手动触发「立即同步」");
    let Some(config) = active_or_current_config(cx) else {
        set_status(cx, PersonalSyncRuntimeStatus::Disabled);
        return;
    };
    sync_master_key_and_user(cx);
    if let Some(runtime) = cx
        .try_global::<GlobalPersonalSyncRuntime>()
        .filter(|state| state.active_config.as_ref() == Some(&config))
        .and_then(|state| state.runtime.as_ref())
    {
        runtime.worker.enqueue(PersonalSyncEvent::FullScan);
        start_runtime_drain(cx);
        return;
    }

    let Some(source) = build_local_source(cx) else {
        set_status(
            cx,
            PersonalSyncRuntimeStatus::failed("personal sync storage is unavailable"),
        );
        return;
    };
    let Some(conflict_sink) = build_conflict_sink(cx) else {
        set_status(
            cx,
            PersonalSyncRuntimeStatus::failed("personal sync conflict storage is unavailable"),
        );
        return;
    };
    run_temporary_full_scan(cx, config, source, conflict_sink);
}

pub(crate) fn list_personal_conflicts(
    cx: &App,
) -> Result<Vec<PersonalSyncConflict>, SyncStoreError> {
    let conflicts = build_conflict_repository(cx).ok_or(SyncStoreError::NotConfigured)?;
    conflicts
        .list("personal")
        .map_err(|error| SyncStoreError::Io(error.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersonalSyncConflictDisplayInfo {
    pub local: Option<PersonalSyncRecordDisplay>,
    pub remote: Option<PersonalSyncRecordDisplay>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersonalSyncRecordDisplay {
    pub name: String,
    pub info: Option<String>,
}

pub(crate) fn refresh_personal_sync_identity(cx: &mut App) {
    sync_master_key_and_user(cx);
}

pub(crate) fn personal_conflict_display_info(
    conflict: &PersonalSyncConflict,
    cx: &App,
) -> PersonalSyncConflictDisplayInfo {
    PersonalSyncConflictDisplayInfo {
        local: local_conflict_display(conflict, cx),
        remote: remote_conflict_display(conflict, cx),
    }
}

pub(crate) fn personal_conflict_selection_key(data_type: &str, record_id: &str) -> String {
    serde_json::to_string(&(data_type, record_id))
        .expect("personal sync conflict selection key must serialize")
}

fn parse_personal_conflict_selection_key(
    selection_key: &str,
) -> Result<(String, String), SyncStoreError> {
    serde_json::from_str(selection_key).map_err(|error| {
        SyncStoreError::Parse(format!(
            "invalid personal sync conflict selection key: {error}"
        ))
    })
}

pub fn resolve_personal_conflict(
    selection_key: String,
    strategy: ConflictResolution,
    cx: &mut App,
) {
    sync_master_key_and_user(cx);
    let Some(config) = active_or_current_config(cx) else {
        set_status(cx, PersonalSyncRuntimeStatus::Disabled);
        return;
    };
    let Some(source) = build_local_source(cx) else {
        set_status(
            cx,
            PersonalSyncRuntimeStatus::failed("personal sync storage is unavailable"),
        );
        return;
    };
    let Some(conflicts) = build_conflict_repository(cx) else {
        set_status(
            cx,
            PersonalSyncRuntimeStatus::failed("personal sync conflict storage is unavailable"),
        );
        return;
    };

    let generation = begin_operation(cx, PersonalSyncRuntimeStatus::Syncing);
    let http = cx.http_client();
    let task = Tokio::spawn(cx, async move {
        resolve_personal_conflict_once(
            config,
            source,
            (*conflicts).clone(),
            selection_key,
            strategy,
            http,
        )
        .await
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let status = personal_sync_status_from_task(task.await);
        let _ = cx.update(move |cx| finish_operation(cx, generation, status));
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

pub fn resolve_personal_conflicts(strategies: Vec<(String, ConflictResolution)>, cx: &mut App) {
    if strategies.is_empty() {
        return;
    }
    if let [(selection_key, strategy)] = strategies.as_slice() {
        resolve_personal_conflict(selection_key.clone(), *strategy, cx);
        return;
    }
    sync_master_key_and_user(cx);
    let Some(config) = active_or_current_config(cx) else {
        set_status(cx, PersonalSyncRuntimeStatus::Disabled);
        return;
    };
    let Some(source) = build_local_source(cx) else {
        set_status(
            cx,
            PersonalSyncRuntimeStatus::failed("personal sync storage is unavailable"),
        );
        return;
    };
    let Some(conflicts) = build_conflict_repository(cx) else {
        set_status(
            cx,
            PersonalSyncRuntimeStatus::failed("personal sync conflict storage is unavailable"),
        );
        return;
    };

    let generation = begin_operation(cx, PersonalSyncRuntimeStatus::Syncing);
    let http = cx.http_client();
    let task = Tokio::spawn(cx, async move {
        for (selection_key, strategy) in strategies {
            resolve_personal_conflict_once(
                config.clone(),
                source.clone(),
                (*conflicts).clone(),
                selection_key,
                strategy,
                http.clone(),
            )
            .await?;
        }
        Ok(())
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let status = personal_sync_status_from_task(task.await);
        let _ = cx.update(move |cx| finish_operation(cx, generation, status));
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

fn local_conflict_display(
    conflict: &PersonalSyncConflict,
    cx: &App,
) -> Option<PersonalSyncRecordDisplay> {
    let snapshot = parse_local_conflict_snapshot(conflict)?;
    let storage = cx.try_global::<GlobalStorageState>()?.storage.clone();
    match snapshot.data_type.as_str() {
        data_type::CONNECTION => {
            let id = parse_prefixed_id(&snapshot.local_id, "connection:")?;
            storage
                .get::<ConnectionRepository>()?
                .get(id)
                .ok()
                .flatten()
                .map(|connection| connection_record_display(&connection, cx))
        }
        data_type::WORKSPACE => {
            let id = parse_prefixed_id(&snapshot.local_id, "workspace:")?;
            storage
                .get::<WorkspaceRepository>()?
                .get(id)
                .ok()
                .flatten()
                .map(|workspace| workspace_record_display(&workspace))
        }
        data_type::CREDENTIAL => {
            let id = parse_prefixed_id(&snapshot.local_id, "credential:")?;
            storage
                .get::<CredentialRepository>()?
                .get_summary(id)
                .ok()
                .flatten()
                .map(|credential| credential_summary_record_display(&credential))
        }
        _ => None,
    }
}

fn remote_conflict_display(
    conflict: &PersonalSyncConflict,
    cx: &App,
) -> Option<PersonalSyncRecordDisplay> {
    let remote = parse_remote_conflict_snapshot(conflict)?;
    let service = cx
        .try_global::<GlobalPersonalSyncRuntime>()?
        .service
        .clone();
    let service = service.read().ok()?;
    match remote.data_type.as_str() {
        data_type::CONNECTION => service
            .decrypt_sync_data_connection(&remote)
            .ok()
            .map(|connection| connection_record_display(&connection, cx)),
        data_type::WORKSPACE => service
            .decrypt_sync_data_workspace(&remote)
            .ok()
            .map(|workspace| workspace_record_display(&workspace)),
        data_type::CREDENTIAL => {
            service
                .decrypt_sync_data_credential(&remote)
                .ok()
                .map(|credential| {
                    credential_record_display(
                        &credential.name,
                        credential.username.as_deref(),
                        credential.cloud_id.as_deref(),
                    )
                })
        }
        _ => None,
    }
}

fn connection_record_display(connection: &StoredConnection, cx: &App) -> PersonalSyncRecordDisplay {
    PersonalSyncRecordDisplay {
        name: fallback_name(
            &connection.name,
            connection.cloud_id.as_deref(),
            "connection",
        ),
        info: record_info([
            Some(crate::connection_visuals::connection_type_display_label(
                connection, cx,
            )),
            connection_endpoint(connection),
        ]),
    }
}

fn workspace_record_display(workspace: &Workspace) -> PersonalSyncRecordDisplay {
    PersonalSyncRecordDisplay {
        name: fallback_name(&workspace.name, workspace.cloud_id.as_deref(), "workspace"),
        info: None,
    }
}

fn credential_summary_record_display(credential: &CredentialSummary) -> PersonalSyncRecordDisplay {
    credential_record_display(
        &credential.name,
        credential.username.as_deref(),
        credential.cloud_id.as_deref(),
    )
}

fn credential_record_display(
    name: &str,
    username: Option<&str>,
    cloud_id: Option<&str>,
) -> PersonalSyncRecordDisplay {
    PersonalSyncRecordDisplay {
        name: fallback_name(name, cloud_id, "credential"),
        info: record_info([username
            .filter(|username| !username.is_empty())
            .map(ToString::to_string)]),
    }
}

fn connection_endpoint(connection: &StoredConnection) -> Option<String> {
    match connection.connection_type {
        ConnectionType::Database => database_endpoint(connection),
        ConnectionType::SshSftp => connection
            .to_ssh_params()
            .ok()
            .map(|params| user_host_port(&params.username, &params.host, params.port)),
        ConnectionType::Redis => connection
            .to_redis_params()
            .ok()
            .map(|params| format!("{}:{}/{}", params.host, params.port, params.db_index)),
        ConnectionType::MongoDB => mongodb_endpoint(connection),
        ConnectionType::Mqtt => connection
            .to_mqtt_params()
            .ok()
            .map(|params| format!("{}:{}", params.host, params.port)),
        ConnectionType::Serial => connection
            .to_serial_params()
            .ok()
            .map(|params| params.port_name),
        ConnectionType::Telnet => connection
            .to_telnet_params()
            .ok()
            .map(|params| format!("{}:{}", params.host, params.port)),
        ConnectionType::Rdp | ConnectionType::Vnc => remote_desktop_endpoint(connection),
        ConnectionType::PortForwarding => port_forwarding_endpoint(connection),
        _ => None,
    }
}

fn database_endpoint(connection: &StoredConnection) -> Option<String> {
    let params = connection.to_db_connection().ok()?;
    if matches!(
        params.database_type,
        DatabaseType::SQLite | DatabaseType::DuckDB
    ) {
        return Some(params.host);
    }
    let database = params
        .database
        .map(|db| format!("/{db}"))
        .unwrap_or_default();
    Some(format!(
        "{}{}",
        user_host_port(&params.username, &params.host, params.port),
        database
    ))
}

fn mongodb_endpoint(connection: &StoredConnection) -> Option<String> {
    let params = connection.to_mongodb_params().ok()?;
    if !params.host.is_empty() {
        return Some(match params.port {
            Some(port) => format!("{}:{}", params.host, port),
            None => params.host,
        });
    }
    (!params.connection_string.is_empty()).then(|| mongo_uri_target(&params.connection_string))
}

fn remote_desktop_endpoint(connection: &StoredConnection) -> Option<String> {
    let params = connection.to_remote_desktop_params().ok()?;
    Some(match params.username.as_deref() {
        Some(username) if !username.is_empty() => {
            user_host_port(username, &params.host, params.port)
        }
        _ => format!("{}:{}", params.host, params.port),
    })
}

fn port_forwarding_endpoint(connection: &StoredConnection) -> Option<String> {
    let params = connection.to_port_forwarding_params().ok()?;
    Some(match params.kind {
        one_core::storage::PortForwardingKind::Local => format!(
            "{}:{} -> {}:{}",
            params.bind_host, params.bind_port, params.target_host, params.target_port
        ),
        one_core::storage::PortForwardingKind::Remote => format!(
            "{}:{} <- {}:{}",
            params.bind_host, params.bind_port, params.target_host, params.target_port
        ),
        one_core::storage::PortForwardingKind::Dynamic => {
            format!("SOCKS {}:{}", params.bind_host, params.bind_port)
        }
    })
}

fn parse_local_conflict_snapshot(
    conflict: &PersonalSyncConflict,
) -> Option<one_core::cloud_sync::personal::PersonalSyncItemSnapshot> {
    serde_json::from_str(conflict.local_snapshot.as_deref()?).ok()
}

fn parse_remote_conflict_snapshot(conflict: &PersonalSyncConflict) -> Option<CloudSyncData> {
    serde_json::from_str(conflict.remote_snapshot.as_deref()?).ok()
}

fn parse_prefixed_id(local_id: &str, prefix: &str) -> Option<i64> {
    local_id.strip_prefix(prefix)?.parse().ok()
}

fn fallback_name(name: &str, id: Option<&str>, fallback: &str) -> String {
    if !name.trim().is_empty() {
        return name.to_string();
    }
    id.unwrap_or(fallback).to_string()
}

fn record_info<const N: usize>(parts: [Option<String>; N]) -> Option<String> {
    let parts = parts.into_iter().flatten().filter(|part| !part.is_empty());
    let text = parts.collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

fn user_host_port(username: &str, host: &str, port: u16) -> String {
    if username.is_empty() {
        return format!("{host}:{port}");
    }
    format!("{username}@{host}:{port}")
}

fn mongo_uri_target(uri: &str) -> String {
    let without_scheme = uri.split_once("://").map(|(_, rest)| rest).unwrap_or(uri);
    let without_auth = without_scheme
        .rsplit_once('@')
        .map(|(_, target)| target)
        .unwrap_or(without_scheme);
    without_auth
        .split(['/', '?'])
        .next()
        .unwrap_or(without_auth)
        .to_string()
}

fn run_temporary_full_scan(
    cx: &mut App,
    config: PersonalSyncRuntimeConfig,
    source: PersonalSyncLocalRepositorySource,
    conflict_sink: SqlitePersonalSyncConflictSink,
) {
    let generation = begin_operation(cx, PersonalSyncRuntimeStatus::Syncing);
    // 手动同步也记下开跑时刻：否则「从未跑过轮次」的启动轮兜底会在它结束后
    // 又补一轮，且「十秒内不重复跑」的冷却账目也不完整。
    cx.global_mut::<GlobalPersonalSyncRuntime>().last_pass_at = Some(Instant::now());
    let http = cx.http_client();
    let task = Tokio::spawn(cx, run_sync(config, source, conflict_sink, http));
    cx.spawn(async move |cx: &mut AsyncApp| {
        let status = personal_sync_status_from_task(task.await);
        let _ = cx.update(move |cx| finish_operation(cx, generation, status));
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

fn reconcile_runtime(cx: &mut App) {
    let settings = AppSettings::global(cx);
    let config = active_personal_sync_settings(settings)
        .ok_or(PersonalSyncRuntimeError::Disabled)
        .and_then(|settings| {
            let password = webdav_password_for(&settings);
            build_personal_sync_runtime_config(&settings, password.as_deref())
        });
    match config {
        Ok(config) => {
            if runtime_config_unchanged(cx, &config) {
                return;
            }
            sync_master_key_and_user(cx);
            match start_running_runtime(cx, &config) {
                Ok(runtime) => {
                    // 先算签名再拿可变借用：`current_scope_signature` 要 `&cx`，
                    // 与下面 `global_mut` 的可变借用不能重叠（E0502）。
                    let scope_signature = current_scope_signature(cx);
                    let state = cx.global_mut::<GlobalPersonalSyncRuntime>();
                    state.active_config = Some(config);
                    state.runtime = Some(runtime);
                    state.last_pass_at = None;
                    state.scope_signature = scope_signature;
                    // 运行时（重）建：基线作废，等下一轮轮询重建，避免把旧签名
                    // 当成「刚发生的变更」再触发一轮。
                    state.scope_baseline.clear();
                    state.scope_baseline_initialized = false;
                    state.scope_baseline_stale = false;
                    state.status = PersonalSyncRuntimeStatus::Ready {
                        health: SyncStoreHealth::Ready,
                        message: None,
                    };
                    // 起来先跑一轮：既及时暴露后端问题（例如 Git 后端指向的
                    // 不是仓库），也让新勾选的同步范围立刻上传。
                    // 这一轮是整个启动链路的关键节点：日志单独记一行，
                    // 丢没丢一目了然（调度器另有「从未跑过轮次」兜底）。
                    tracing::info!(
                        data_types = state.scope_signature.len(),
                        "个人同步：同步运行时已建立，入队首轮全量扫描"
                    );
                    enqueue_auto_sync_event(PersonalSyncEvent::FullScan, cx);
                }
                Err(error) => {
                    tracing::warn!(error = %error, "个人同步：同步运行时建立失败");
                    let state = cx.global_mut::<GlobalPersonalSyncRuntime>();
                    state.active_config = Some(config);
                    state.runtime = None;
                    state.status = PersonalSyncRuntimeStatus::from_error(error);
                }
            }
        }
        Err(PersonalSyncRuntimeError::Disabled | PersonalSyncRuntimeError::NotConfigured) => {
            tracing::info!("个人同步：同步未开启或当前 provider 不是个人同步，后台同步不参与");
            let state = cx.global_mut::<GlobalPersonalSyncRuntime>();
            state.generation += 1;
            state.active_config = None;
            state.runtime = None;
            state.status = PersonalSyncRuntimeStatus::Disabled;
            state.last_pass_at = None;
            state.scope_baseline.clear();
            state.scope_baseline_initialized = false;
            state.scope_baseline_stale = false;
        }
    }
}

fn runtime_config_unchanged(cx: &App, config: &PersonalSyncRuntimeConfig) -> bool {
    cx.try_global::<GlobalPersonalSyncRuntime>()
        .is_some_and(|state| {
            state.active_config.as_ref() == Some(config)
                && state.runtime.is_some()
                && state.scope_signature == current_scope_signature(cx)
        })
}

/// 当前生效的同步范围签名（排序后的 data_type 列表）。
///
/// 后端配置没变但用户勾选 / 取消勾选了数据类型时，也必须重建运行时：
/// `SyncScopeSource` 的 `enabled` 是装配那一刻的快照，不重建的话新勾选的类型
/// 永远进不了 `list_items`——表现为「明明勾了，立即同步还是没有这类数据」。
fn current_scope_signature(cx: &App) -> Vec<String> {
    let settings = AppSettings::global(cx);
    let mut signature: Vec<String> = settings
        .sync_scope
        .effective(settings.sync_provider)
        .into_iter()
        .filter_map(|key| {
            one_core::settings::sync_scope_descriptor(key).map(|d| d.data_type.to_string())
        })
        .collect();
    signature.sort();
    signature
}

fn start_running_runtime(
    cx: &App,
    config: &PersonalSyncRuntimeConfig,
) -> Result<RunningPersonalSyncRuntime, SyncStoreError> {
    // scope 同时交给 worker（列举本地项）和变更轮询（取轻量签名）：enabled
    // 是装配那一刻的快照，两边必须是同一份，否则轮询到的类型和同步到的不一致。
    let scope = build_scope_source(cx);
    let source =
        build_local_source_with_scope(cx, scope.clone()).ok_or(SyncStoreError::NotConfigured)?;
    let conflict_sink = build_conflict_sink(cx).ok_or(SyncStoreError::NotConfigured)?;
    let store = ConfiguredPersonalSyncStore::from_runtime_config(config, cx.http_client())?;
    let worker = PersonalSyncWorker::with_conflict_sink(
        store.clone(),
        source,
        conflict_sink,
        WorkerConfig {
            backend_profile_id: "personal".to_string(),
            device_id: SyncDeviceId("local-device".to_string()),
        },
    );
    // WebDAV 后端没有本地目录，文件监听无从谈起：交给调度器与变更轮询驱动。
    let watcher = if config.auto_sync && !config.root.as_os_str().is_empty() {
        Some(start_watcher(config.root.clone(), worker.clone())?)
    } else {
        None
    };
    Ok(RunningPersonalSyncRuntime {
        store,
        worker,
        scope,
        _watcher: watcher,
    })
}

fn subscribe_local_events(cx: &mut App) -> Option<Subscription> {
    let notifier = get_notifier(cx)?;
    Some(
        cx.subscribe(&notifier, |_, event: &ConnectionDataEvent, cx| {
            if let Some(sync_event) = personal_sync_event_from_connection_event(event) {
                enqueue_local_change(sync_event, cx);
            }
        }),
    )
}

pub(crate) fn personal_sync_event_from_connection_event(
    event: &ConnectionDataEvent,
) -> Option<PersonalSyncEvent> {
    match event {
        ConnectionDataEvent::ConnectionCreated { connection }
        | ConnectionDataEvent::ConnectionUpdated { connection } => {
            let local_id = format!("connection:{}", connection.id?);
            Some(PersonalSyncEvent::LocalChanged {
                data_type: data_type::CONNECTION.to_string(),
                local_id,
            })
        }
        ConnectionDataEvent::CredentialCreated { credential_id }
        | ConnectionDataEvent::CredentialUpdated { credential_id } => {
            Some(PersonalSyncEvent::LocalChanged {
                data_type: data_type::CREDENTIAL.to_string(),
                local_id: format!("credential:{credential_id}"),
            })
        }
        ConnectionDataEvent::ConnectionDeleted {
            cloud_id: Some(cloud_id),
            last_synced_at,
            ..
        } => Some(PersonalSyncEvent::LocalDeleted {
            data_type: data_type::CONNECTION.to_string(),
            cloud_id: cloud_id.clone(),
            last_synced_at: *last_synced_at,
        }),
        ConnectionDataEvent::WorkspaceDeleted {
            cloud_id: Some(cloud_id),
            last_synced_at,
            ..
        } => Some(PersonalSyncEvent::LocalDeleted {
            data_type: data_type::WORKSPACE.to_string(),
            cloud_id: cloud_id.clone(),
            last_synced_at: *last_synced_at,
        }),
        ConnectionDataEvent::CredentialDeleted {
            cloud_id: Some(cloud_id),
            last_synced_at,
            ..
        } => Some(PersonalSyncEvent::LocalDeleted {
            data_type: data_type::CREDENTIAL.to_string(),
            cloud_id: cloud_id.clone(),
            last_synced_at: *last_synced_at,
        }),
        ConnectionDataEvent::ConnectionDeleted { cloud_id: None, .. }
        | ConnectionDataEvent::WorkspaceCreated { .. }
        | ConnectionDataEvent::WorkspaceUpdated { .. }
        | ConnectionDataEvent::WorkspaceDeleted { cloud_id: None, .. }
        | ConnectionDataEvent::CredentialDeleted { cloud_id: None, .. } => {
            Some(PersonalSyncEvent::FullScan)
        }
        ConnectionDataEvent::SchemaChanged { .. }
        | ConnectionDataEvent::TableDataCleared { .. }
        | ConnectionDataEvent::CloudSyncRequested
        | ConnectionDataEvent::TeamCacheUpdated => None,
    }
}

fn enqueue_local_change(event: PersonalSyncEvent, cx: &mut App) {
    enqueue_auto_sync_event(event, cx);
}

/// 某类自选同步数据发生本地变更：尽快跑一轮同步。
///
/// 注意 `run_locked_pass` 每轮都会重新列举全部本地项与远端记录再整体规划，
/// 所以这里做不到「只同步这一种类型」——事件只是「尽快跑一轮」的信号。
fn notify_scope_data_changed(cx: &mut App, data_type: &str) {
    enqueue_local_change(
        PersonalSyncEvent::LocalChanged {
            data_type: data_type.to_string(),
            local_id: String::new(),
        },
        cx,
    );
}

/// 各设置分组上次观察到的白名单载荷。
///
/// 监听 `AppSettings` 全局拿不到新旧值，只能自己留一份快照比对。快照只含
/// 白名单字段，所以同步开关 / 同步范围这类字段改动不会触发同步 —— 也就不会
/// 出现「改同步设置 → 触发同步 → 又写设置」的自激循环。
#[derive(Default)]
struct SettingsSyncBaseline {
    /// 首次观察只建基线、不触发同步，避免一启动就跑一轮
    initialized: bool,
    payloads: HashMap<String, String>,
}

impl Global for SettingsSyncBaseline {}

/// `AppSettings` 变更时，为真正变了且已开启同步的分组唤醒同步。
fn observe_settings_for_sync(cx: &mut App) {
    let mut current: Vec<(String, String)> = Vec::new();
    {
        let settings = AppSettings::global(cx);
        let provider = settings.sync_provider;
        for group in SettingsGroup::ALL {
            let data_type = group.data_type();
            let Some(descriptor) = SYNC_SCOPE_DESCRIPTORS
                .iter()
                .find(|item| item.data_type == data_type)
            else {
                continue;
            };
            if !settings.sync_scope.is_active(descriptor.key, provider) {
                continue;
            }
            let Ok(payload) = extract_payload(*group, settings) else {
                continue;
            };
            let Ok(canonical) = serde_json::to_string(&payload) else {
                continue;
            };
            current.push((data_type.to_string(), canonical));
        }
    }

    let mut changed: Vec<String> = Vec::new();
    {
        let baseline = cx.global_mut::<SettingsSyncBaseline>();
        let initialized = baseline.initialized;
        baseline.initialized = true;
        for (data_type, canonical) in current {
            if initialized && baseline.payloads.get(&data_type) != Some(&canonical) {
                changed.push(data_type.clone());
            }
            baseline.payloads.insert(data_type, canonical);
        }
    }

    if !changed.is_empty() {
        tracing::info!(
            groups = %changed.join(", "),
            "个人同步：设置分组发生变更，触发同步"
        );
    }
    for data_type in changed {
        notify_scope_data_changed(cx, &data_type);
    }
}

fn enqueue_periodic_full_scan(cx: &mut App) {
    enqueue_auto_sync_event(PersonalSyncEvent::FullScan, cx);
}

/// 自动同步总闸：总开关 + 个人同步 provider + 自动同步子开关 + 运行时已在跑。
///
/// 运行时存在但 `auto_sync` 关闭时（用户只想手动点「立即同步」），调度器与
/// 变更轮询都静默，不产生任何后台流量。
fn personal_auto_sync_enabled(cx: &App) -> bool {
    let settings = AppSettings::global(cx);
    settings.sync_enabled
        && settings.sync_provider == SyncProvider::Personal
        && settings.personal_sync.auto_sync
        && cx.try_global::<GlobalPersonalSyncRuntime>()
            .is_some_and(|state| state.runtime.is_some())
}

fn enqueue_auto_sync_event(event: PersonalSyncEvent, cx: &mut App) {
    if !personal_auto_sync_enabled(cx) {
        // 记下被哪道闸门拦下：排查「改了却没同步」时，这一行就是答案。
        let settings = AppSettings::global(cx);
        let reason = if !settings.sync_enabled {
            "同步总开关未打开"
        } else if settings.sync_provider != SyncProvider::Personal {
            "当前同步 provider 不是个人同步"
        } else if !settings.personal_sync.auto_sync {
            "自动同步子开关未打开（仅手动同步）"
        } else {
            "同步运行时尚未就绪"
        };
        tracing::info!(event = ?event, reason, "个人同步：变更未入队");
        return;
    }
    sync_master_key_and_user(cx);
    let Some(runtime) = cx
        .try_global::<GlobalPersonalSyncRuntime>()
        .and_then(|state| state.runtime.as_ref())
    else {
        return;
    };
    tracing::info!(event = ?event, "个人同步：变更入队");
    runtime.worker.enqueue(event);
    maybe_start_scheduled_drain(cx);
}

/// 冷却已满就立刻跑；没满就留给调度器，等某个 tick 补跑。
///
/// 十秒内无论来多少变更（设置观察者、连接通知、变更轮询、文件监听）都
/// 合并成一轮：既削服务器压力，也让多设备错开写入、少撞远端锁。
fn maybe_start_scheduled_drain(cx: &mut App) {
    if !should_start_drain_after_enqueue(&runtime_status(cx)) {
        return;
    }
    if !personal_sync_cooldown_elapsed(cx) {
        tracing::info!(
            cooldown_secs = PERSONAL_SYNC_MIN_INTERVAL.as_secs(),
            "个人同步：距上轮同步不足冷却时间，本轮延迟到冷却结束后执行"
        );
        return;
    }
    start_runtime_drain(cx);
}

/// 距上次同步是否已满十秒。
pub(crate) fn personal_sync_cooldown_elapsed(cx: &App) -> bool {
    cx.try_global::<GlobalPersonalSyncRuntime>()
        .and_then(|state| state.last_pass_at)
        .map_or(true, |last| last.elapsed() >= PERSONAL_SYNC_MIN_INTERVAL)
}

pub(crate) fn should_start_drain_after_enqueue(status: &PersonalSyncRuntimeStatus) -> bool {
    !matches!(status, PersonalSyncRuntimeStatus::Syncing)
}

fn start_runtime_drain(cx: &mut App) {
    if active_personal_sync_settings(AppSettings::global(cx)).is_none() {
        return;
    }
    let Some(state) = cx.try_global::<GlobalPersonalSyncRuntime>() else {
        return;
    };
    if matches!(state.status, PersonalSyncRuntimeStatus::Syncing) {
        return;
    }
    let Some(runtime) = state.runtime.as_ref() else {
        return;
    };
    let worker = runtime.worker.clone();
    let store = runtime.store.clone();
    // 记下开跑时刻：调度器据此执行「十秒内不重复跑」与「空闲 10 分钟兜底」。
    cx.global_mut::<GlobalPersonalSyncRuntime>().last_pass_at = Some(Instant::now());
    let generation = begin_operation(cx, PersonalSyncRuntimeStatus::Syncing);
    tracing::info!("个人同步：开始一轮同步");
    let task = Tokio::spawn(cx, drain_and_flush(worker, store));
    cx.spawn(async move |cx: &mut AsyncApp| {
        let outcome = task.await;
        match &outcome {
            Ok(Ok(())) => tracing::info!("个人同步：本轮同步完成"),
            Ok(Err(error)) => tracing::warn!(error = %error, "个人同步：本轮同步失败"),
            Err(error) => tracing::warn!(error = %error, "个人同步：本轮同步任务异常退出"),
        }
        let status = personal_sync_status_from_task(outcome);
        let _ = cx.update(move |cx| finish_operation(cx, generation, status));
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

async fn drain_and_flush(
    worker: RunningPersonalSyncWorker,
    store: ConfiguredPersonalSyncStore,
) -> Result<(), SyncStoreError> {
    worker.drain_once().await?;
    store.flush().await
}

/// 同步调度器：每秒醒一次，只按本地状态决策，不产生任何 IO。
///
/// - 有待处理事件且冷却已满 → 立刻跑；
/// - 冷却没满 → 什么都不做，事件继续攒着，满十秒后的某个 tick 一次性跑掉；
/// - 还从未跑过轮次 → 补跑启动轮（`reconcile_runtime` 入队的那轮可能丢了）；
/// - 没有事件且距上次同步已 10 分钟 → 兜底跑一轮（拉取其他设备的改动）。
fn start_personal_sync_scheduler(cx: &mut App) {
    cx.spawn(|cx: &mut AsyncApp| {
        let cx = cx.clone();
        async move {
            loop {
                cx.background_executor()
                    .timer(PERSONAL_SYNC_SCHED_TICK)
                    .await;
                cx.update(|cx| personal_sync_scheduler_tick(cx));
            }
        }
    })
    .detach();
}

fn personal_sync_scheduler_tick(cx: &mut App) {
    if !personal_auto_sync_enabled(cx) {
        return;
    }
    let has_pending = cx
        .try_global::<GlobalPersonalSyncRuntime>()
        .and_then(|state| state.runtime.as_ref())
        .is_some_and(|runtime| runtime.worker.has_pending());
    if has_pending {
        maybe_start_scheduled_drain(cx);
        return;
    }
    // 启动轮兜底：运行时在跑、自动同步开着，却还从未跑过任何一轮 ——
    // reconcile_runtime 建好运行时时入队的那轮 FullScan 可能因主线程冻结
    // （init 期间 UI 线程被长任务卡住）或竞态静默丢失。这里补一轮，保证
    // 启动轮最迟在一秒内补跑，不让首轮前的本地变更干等。
    let missed_startup = cx
        .try_global::<GlobalPersonalSyncRuntime>()
        .is_some_and(|state| {
            missed_startup_pass(state.runtime.is_some(), state.last_pass_at, &state.status)
        });
    if missed_startup {
        tracing::info!("个人同步：检测到启动轮未执行，补跑一轮全量扫描");
        enqueue_periodic_full_scan(cx);
        return;
    }
    if personal_sync_idle_elapsed(cx) {
        enqueue_periodic_full_scan(cx);
    }
}

/// 启动轮兜底判定：运行时已在跑、还从未跑过轮次、且当前没在同步。
///
/// 抽成纯函数便于单测：三种状态组合的语义不依赖 GPUI 全局。正常流程里
/// `reconcile_runtime` 建好运行时后约一秒内就会跑首轮并把 `last_pass_at`
/// 置上，所以这个判定只在「启动轮真的丢了」时才成立。
pub(crate) fn missed_startup_pass(
    runtime_active: bool,
    last_pass_at: Option<Instant>,
    status: &PersonalSyncRuntimeStatus,
) -> bool {
    runtime_active && last_pass_at.is_none() && !matches!(status, PersonalSyncRuntimeStatus::Syncing)
}

/// 距上次同步是否已超过空闲兜底间隔。
///
/// 还没跑过首轮时为 `false`：「从未跑过轮次」由调度器的启动轮兜底负责
/// （见 `personal_sync_scheduler_tick`），不走空闲兜底。
pub(crate) fn personal_sync_idle_elapsed(cx: &App) -> bool {
    cx.try_global::<GlobalPersonalSyncRuntime>()
        .and_then(|state| state.last_pass_at)
        .is_some_and(|last| last.elapsed() >= PERSONAL_SYNC_IDLE_INTERVAL)
}

/// 本地变更轮询：每 3 秒比对一次各类自选数据的轻量签名。
///
/// 写入点散落在多个 crate（notes 在 `crates/notes`、saved_sql 是裸
/// `fs::write`、快捷命令在 terminal_view），逐个挂钩不现实，统一用轮询覆盖。
/// 连接 / 工作空间 / 钥匙串走既有的 `ConnectionDataEvent` 通知，设置类走
/// `AppSettings` 观察者按白名单载荷比对，都不在这里重复。
fn start_scope_change_poller(cx: &mut App) {
    cx.spawn(|cx: &mut AsyncApp| {
        let cx = cx.clone();
        async move {
            loop {
                cx.background_executor()
                    .timer(SCOPE_CHANGE_POLL_INTERVAL)
                    .await;
                let scope = cx.update(|cx| {
                    if !personal_auto_sync_enabled(cx) {
                        return None;
                    }
                    cx.try_global::<GlobalPersonalSyncRuntime>()
                        .and_then(|state| state.runtime.as_ref())
                        .and_then(|runtime| runtime.scope.clone())
                });
                let Some(scope) = scope else {
                    continue;
                };
                // 签名计算只读少量 DB 行与文件时间戳，放后台线程，不卡 UI。
                let signatures = cx
                    .background_executor()
                    .spawn(async move { scope.change_signatures() })
                    .await;
                cx.update(|cx| apply_scope_signatures(cx, signatures));
            }
        }
    })
    .detach();
}

/// 把本轮签名与基线比对：首轮与 stale 只建/刷基线，否则有差异就入队一轮。
fn apply_scope_signatures(cx: &mut App, signatures: Vec<(String, String)>) {
    // 同步进行中（或刚结束的宽限期内）：本地正被同步自己写，此刻的签名既不是
    // 「用户改前」也不是「用户改后」，跳过；等 finish 时置的 stale 让下一轮
    // 轮询刷新基线。
    let syncing = cx
        .try_global::<GlobalPersonalSyncRuntime>()
        .and_then(|state| state.runtime.as_ref())
        .is_some_and(|runtime| runtime.worker.is_syncing_or_just_finished());
    if syncing {
        return;
    }

    let (changed, changed_types) = {
        let state = cx.global_mut::<GlobalPersonalSyncRuntime>();
        if !state.scope_baseline_initialized {
            tracing::info!(
                types = signatures.len(),
                "个人同步：本地变更轮询基线已建立"
            );
            state.scope_baseline = signatures.into_iter().collect();
            state.scope_baseline_initialized = true;
            (false, Vec::new())
        } else if state.scope_baseline_stale {
            tracing::info!("个人同步：上一轮同步刚结束，本轮轮询只刷新基线（防自激）");
            state.scope_baseline = signatures.into_iter().collect();
            state.scope_baseline_stale = false;
            (false, Vec::new())
        } else {
            let changed_types: Vec<String> = signatures
                .iter()
                .filter(|(data_type, signature)| {
                    state.scope_baseline.get(data_type) != Some(signature)
                })
                .map(|(data_type, _)| data_type.clone())
                .collect();
            if changed_types.is_empty() {
                (false, Vec::new())
            } else {
                state.scope_baseline = signatures.into_iter().collect();
                (true, changed_types)
            }
        }
    };
    if changed {
        tracing::info!(
            data_types = %changed_types.join(", "),
            "个人同步：轮询检测到本地变更，入队全量扫描"
        );
        enqueue_periodic_full_scan(cx);
    }
}

/// 监听同步目录（仅文件夹后端有本地目录）。
///
/// 回调只做「入队」：何时真正跑由 UI 线程的调度器决定（状态、十秒冷却、
/// 变更合并都归它管），调度器 tick 每秒一次，最多晚一秒发现。自己写下去的
/// 记录也会触发监听，用「同步中 + 宽限期」挡掉，否则每轮真实同步都跟着一轮空转。
fn start_watcher(
    root: std::path::PathBuf,
    worker: RunningPersonalSyncWorker,
) -> Result<PersonalSyncWatcher, SyncStoreError> {
    PersonalSyncWatcher::start(root, Duration::from_secs(2), move |event| {
        if worker.is_syncing_or_just_finished() {
            return;
        }
        tracing::info!(event = ?event, "个人同步：同步目录文件变更，入队同步");
        worker.enqueue(event);
    })
}

async fn run_sync(
    config: PersonalSyncRuntimeConfig,
    source: PersonalSyncLocalRepositorySource,
    conflict_sink: SqlitePersonalSyncConflictSink,
    http: Arc<dyn HttpClient>,
) -> Result<(), SyncStoreError> {
    let store = ConfiguredPersonalSyncStore::from_runtime_config(&config, http)?;
    let worker = PersonalSyncWorker::with_conflict_sink(
        store.clone(),
        source,
        conflict_sink,
        WorkerConfig {
            backend_profile_id: "personal".to_string(),
            device_id: SyncDeviceId("local-device".to_string()),
        },
    );
    worker.enqueue(PersonalSyncEvent::FullScan);
    worker.drain_once().await?;
    store.flush().await
}

pub(crate) fn build_conflict_sink(cx: &App) -> Option<SqlitePersonalSyncConflictSink> {
    let conflicts = build_conflict_repository(cx)?;
    Some(SqlitePersonalSyncConflictSink::new(
        "personal".to_string(),
        conflicts,
    ))
}

fn build_conflict_repository(cx: &App) -> Option<Arc<PersonalSyncConflictRepository>> {
    let storage = cx.try_global::<GlobalStorageState>()?.storage.clone();
    storage.get::<PersonalSyncConflictRepository>()
}

/// 本地条目被删除后，把它遗留的同步冲突一并清掉。
///
/// 冲突表的主键是 `(profile, data_type, 云端 record_id)`，但解析冲突要按云端 id
/// 反查本地条目（`local.list_items()`）。本地条目消失后冲突就永远解不开 ——
/// 两个按钮分别报 `connection not found` / `Connection N not found`；更糟的是点
/// 「使用远程版本」还会把刚删掉的条目重新拉回本地。所以删除实体时必须一起清掉。
pub(crate) fn forget_personal_conflict(storage: &StorageManager, data_type: &str, cloud_id: &str) {
    let Some(conflicts) = storage.get::<PersonalSyncConflictRepository>() else {
        return;
    };
    if let Err(error) = conflicts.delete("personal", data_type, cloud_id) {
        tracing::warn!(
            error = %error,
            data_type,
            cloud_id,
            "Failed to forget personal sync conflict after local delete"
        );
    }
}

async fn resolve_personal_conflict_once(
    config: PersonalSyncRuntimeConfig,
    source: PersonalSyncLocalRepositorySource,
    conflicts: PersonalSyncConflictRepository,
    selection_key: String,
    strategy: ConflictResolution,
    http: Arc<dyn HttpClient>,
) -> Result<(), SyncStoreError> {
    let (data_type, record_id) = parse_personal_conflict_selection_key(&selection_key)?;
    let conflict = conflicts
        .list("personal")
        .map_err(|error| SyncStoreError::Io(error.to_string()))?
        .into_iter()
        .find(|conflict| conflict.data_type == data_type && conflict.record_id == record_id)
        .ok_or_else(|| {
            SyncStoreError::Parse(format!(
                "personal sync conflict not found: {data_type}/{record_id}"
            ))
        })?;
    let store = ConfiguredPersonalSyncStore::from_runtime_config(&config, http)?;
    let resolver = PersonalSyncConflictResolver::new(store.clone(), source, conflicts);
    resolver.resolve(&conflict, strategy).await?;
    store.flush().await
}

fn build_local_source(cx: &App) -> Option<PersonalSyncLocalRepositorySource> {
    build_local_source_with_scope(cx, build_scope_source(cx))
}

/// 与 [`build_local_source`] 相同，但复用调用方已装配的 scope。
///
/// 运行时把同一个 `Arc<SyncScopeSource>` 同时交给 worker 和变更轮询，
/// 不能各建一份：`enabled` 是装配那一刻的快照，两份快照可能不一致。
fn build_local_source_with_scope(
    cx: &App,
    scope: Option<Arc<SyncScopeSource>>,
) -> Option<PersonalSyncLocalRepositorySource> {
    let storage = cx.try_global::<GlobalStorageState>()?.storage.clone();
    let connections = storage.get::<ConnectionRepository>()?;
    let credentials = storage.get::<CredentialRepository>()?;
    let workspaces = storage.get::<WorkspaceRepository>()?;
    let service = cx
        .try_global::<GlobalPersonalSyncRuntime>()?
        .service
        .clone();

    // scope_env / user_id 必须与 `build_scope_source` 和设置页读取时用的一致
    // （都是 "personal"），否则连接 / 工作空间 / 钥匙串的同步状态查不到。
    let source = PersonalSyncLocalRepositorySource::new(
        (*connections).clone(),
        (*credentials).clone(),
        (*workspaces).clone(),
        service.clone(),
        SyncTypeStatusRepository::new(storage.connection()),
        "personal".to_string(),
        "personal".to_string(),
    );

    Some(source.with_scope(scope))
}

/// 装配「用户自选同步数据范围」扩展源
///
/// 拿不到任一依赖时返回 `None`（功能降级为只同步连接 / 工作空间 / 钥匙串），
/// 而不是让整条同步链路起不来。
fn build_scope_source(cx: &App) -> Option<Arc<SyncScopeSource>> {
    let storage = cx.try_global::<GlobalStorageState>()?.storage.clone();
    let service = cx
        .try_global::<GlobalPersonalSyncRuntime>()?
        .service
        .clone();
    let quick_commands = storage.get::<QuickCommandRepository>()?;
    let sftp = storage.get::<SftpFavoritePathRepository>()?;
    let providers = storage.get::<ProviderRepository>()?;

    let settings = AppSettings::global(cx);
    let enabled: HashSet<String> = settings
        .sync_scope
        .effective(settings.sync_provider)
        .into_iter()
        .filter_map(|key| {
            one_core::settings::sync_scope_descriptor(key).map(|d| d.data_type.to_string())
        })
        .collect();

    Some(Arc::new(SyncScopeSource::new(
        "personal".to_string(),
        "personal".to_string(),
        enabled,
        SyncItemStateRepository::new(storage.connection()),
        SyncTypeStatusRepository::new(storage.connection()),
        (*quick_commands).clone(),
        (*sftp).clone(),
        (*providers).clone(),
        Some(Arc::new(crate::sync_note_adapter::FileSystemNoteAdapter)),
        service,
    )))
}

fn sync_master_key_and_user(cx: &mut App) {
    let user = GlobalCurrentUser::get_user(cx);
    let raw_key = crypto::get_raw_master_key();
    let service = cx.global::<GlobalPersonalSyncRuntime>().service.clone();
    if let Ok(mut service) = service.write() {
        if let Some(user) = user {
            service.set_logged_in(user.id);
        }
        if let Some(raw_key) = raw_key {
            service.set_master_key_directly(raw_key);
        }
    }
}

fn active_or_current_config(cx: &App) -> Option<PersonalSyncRuntimeConfig> {
    active_personal_sync_settings(AppSettings::global(cx)).and_then(|settings| {
        let password = webdav_password_for(&settings);
        build_personal_sync_runtime_config(&settings, password.as_deref()).ok()
    })
}

fn active_personal_sync_settings(settings: &AppSettings) -> Option<PersonalSyncSettings> {
    if !settings.sync_enabled || settings.sync_provider != SyncProvider::Personal {
        return None;
    }
    Some(settings.personal_sync.clone())
}

fn begin_operation(cx: &mut App, status: PersonalSyncRuntimeStatus) -> u64 {
    let state = cx.global_mut::<GlobalPersonalSyncRuntime>();
    state.generation += 1;
    state.status = status;
    state.generation
}

/// 一轮（或一次性的）同步操作结束：收状态，并把变更轮询的基线标记为作废。
///
/// 同步自己写下去的内容（远端改动落到本地 DB / 文件、冲突解析写回本地）会被
/// 轮询当成「用户刚改的」；置了 stale 后，下一轮轮询只刷新基线、不再触发同步。
fn finish_operation(cx: &mut App, generation: u64, status: PersonalSyncRuntimeStatus) {
    let state = cx.global_mut::<GlobalPersonalSyncRuntime>();
    if state.generation == generation {
        state.status = status;
    }
    state.scope_baseline_stale = true;
}

fn personal_sync_status_from_task(
    result: Result<Result<(), SyncStoreError>, tokio::task::JoinError>,
) -> PersonalSyncRuntimeStatus {
    match result {
        Ok(Ok(())) => {
            crate::personal_sync_status::note_sync_completed();
            PersonalSyncRuntimeStatus::Ready {
                health: SyncStoreHealth::Ready,
                message: None,
            }
        }
        Ok(Err(error)) => PersonalSyncRuntimeStatus::from_error(error),
        Err(error) => PersonalSyncRuntimeStatus::failed(&error.to_string()),
    }
}

fn set_status(cx: &mut App, status: PersonalSyncRuntimeStatus) {
    cx.global_mut::<GlobalPersonalSyncRuntime>().status = status;
}

/// 主密钥变更后，把个人同步远端记录从旧密钥重加密为新密钥。
///
/// 业务时间戳没变，个人同步 planner 会判定「已同步」而不重传，所以云端存量
/// 密文必须主动遍历重写（方案甲）。未完成的部分记 pending 标记，下次同步前补做。
pub fn reencrypt_cloud_data(
    old_key: &str,
    new_key: &str,
    key_version: u32,
    notifier: Option<crate::master_key_flow::CloudReencryptNotifier>,
    cx: &mut App,
) -> bool {
    let Some(config) = configured_sync_config(cx) else {
        tracing::info!("个人同步未配置，跳过云端重加密");
        return false;
    };
    let http = cx.http_client();
    let old_key = old_key.to_string();
    let new_key = new_key.to_string();

    let task = Tokio::spawn(cx, async move {
        let store = ConfiguredPersonalSyncStore::from_runtime_config(&config, http)?;
        // 重加密只用 re_encrypt_sync_data（显式传旧/新密钥），不依赖服务内的当前密钥
        let service = CloudSyncService::new();
        let stats =
            one_core::cloud_sync::reencrypt_personal(&store, &service, &old_key, &new_key, key_version)
                .await?;
        store.flush().await?;
        Ok::<_, SyncStoreError>(stats)
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        let message = match task.await {
            Ok(Ok(stats)) => {
                one_core::master_key_meta::set_pending_cloud_reencrypt(stats.incomplete());
                tracing::info!(
                    "个人同步云端重加密完成：共 {} 条，重加密 {} 条，失败 {} 条",
                    stats.total,
                    stats.re_encrypted,
                    stats.failed
                );
                if stats.incomplete() {
                    t!("Encryption.cloud_reencrypt_failed").to_string()
                } else {
                    t!("Encryption.cloud_reencrypt_done").to_string()
                }
            }
            Ok(Err(error)) => {
                one_core::master_key_meta::set_pending_cloud_reencrypt(true);
                tracing::error!("个人同步云端重加密失败: {error}");
                t!("Encryption.cloud_reencrypt_failed").to_string()
            }
            Err(error) => {
                one_core::master_key_meta::set_pending_cloud_reencrypt(true);
                tracing::error!("个人同步云端重加密任务失败: {error}");
                t!("Encryption.cloud_reencrypt_failed").to_string()
            }
        };
        let _ = cx.update(move |cx: &mut App| {
            if let Some(notify) = notifier.as_ref() {
                notify(cx, message.clone());
            }
        });
        Ok::<(), anyhow::Error>(())
    })
    .detach();
    true
}

/// 重置主密钥时清空个人同步远端数据（记录软删除并提交后端）。
pub fn wipe_cloud_data(cx: &mut App) {
    let Some(config) = configured_sync_config(cx) else {
        return;
    };
    let http = cx.http_client();

    let task = Tokio::spawn(cx, async move {
        let store = ConfiguredPersonalSyncStore::from_runtime_config(&config, http)?;
        let records = store.list_records(None, None).await?;
        let total = records.len();
        for record in records {
            store
                .tombstone_record(&record.data_type, &record.id, Some(record.version))
                .await?;
        }
        store.flush().await?;
        Ok::<_, SyncStoreError>(total)
    });

    cx.spawn(async move |_cx: &mut AsyncApp| {
        match task.await {
            Ok(Ok(total)) => {
                one_core::master_key_meta::set_pending_cloud_wipe(false);
                tracing::info!("个人同步云端数据已清空：{total} 条");
            }
            Ok(Err(error)) => {
                one_core::master_key_meta::set_pending_cloud_wipe(true);
                tracing::error!("个人同步云端数据清空失败: {error}");
            }
            Err(error) => {
                one_core::master_key_meta::set_pending_cloud_wipe(true);
                tracing::error!("个人同步云端数据清空任务失败: {error}");
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

/// 取当前个人同步后端配置。
///
/// 与 [`active_or_current_config`] 的区别是不要求「同步总开关已打开且当前
/// provider 正是个人同步」：重置主密钥时无论当前走哪条同步链路，都要清掉
/// 已经落在个人同步后端里的数据。
fn configured_sync_config(cx: &App) -> Option<PersonalSyncRuntimeConfig> {
    let settings = AppSettings::global(cx);
    let password = webdav_password_for(&settings.personal_sync);
    build_personal_sync_runtime_config(&settings.personal_sync, password.as_deref()).ok()
}
