use std::sync::{Arc, RwLock};
use std::time::Duration;

use gpui::{App, AsyncApp, Global, Subscription};
use one_core::cloud_sync::personal::{
    ConfiguredPersonalSyncStore, PersonalSyncConflict, PersonalSyncConflictRepository,
    PersonalSyncConflictResolver, PersonalSyncEvent, PersonalSyncLocalRepositorySource,
    PersonalSyncRuntimeConfig, PersonalSyncRuntimeError, PersonalSyncStore, PersonalSyncWatcher,
    PersonalSyncWorker, SqlitePersonalSyncConflictSink, SyncDeviceId, SyncStoreError,
    SyncStoreHealth, WorkerConfig, build_personal_sync_runtime_config,
};
use one_core::cloud_sync::{CloudSyncData, CloudSyncService, ConflictResolution, data_type};
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::crypto;
use one_core::gpui_tokio::Tokio;
use one_core::settings::{AppSettings, GlobalCurrentUser, PersonalSyncSettings, SyncProvider};
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, CredentialRepository, CredentialSummary, DatabaseType,
    GlobalStorageState, StoredConnection, Workspace, WorkspaceRepository,
};

use crate::personal_sync_status::PersonalSyncRuntimeStatus;

const PERSONAL_SYNC_PERIODIC_INTERVAL: Duration = Duration::from_secs(60);

pub struct GlobalPersonalSyncRuntime {
    active_config: Option<PersonalSyncRuntimeConfig>,
    runtime: Option<RunningPersonalSyncRuntime>,
    service: Arc<RwLock<CloudSyncService>>,
    status: PersonalSyncRuntimeStatus,
    generation: u64,
    pending_auto_drain: bool,
    _settings_subscription: Subscription,
    _local_event_subscription: Option<Subscription>,
}

impl Global for GlobalPersonalSyncRuntime {}

struct RunningPersonalSyncRuntime {
    store: ConfiguredPersonalSyncStore,
    worker: RunningPersonalSyncWorker,
    _watcher: Option<PersonalSyncWatcher>,
}

type RunningPersonalSyncWorker = PersonalSyncWorker<
    ConfiguredPersonalSyncStore,
    PersonalSyncLocalRepositorySource,
    SqlitePersonalSyncConflictSink,
>;

pub fn init(cx: &mut App) {
    let settings_subscription = cx.observe_global::<AppSettings>(reconcile_runtime);
    let local_event_subscription = subscribe_local_events(cx);
    cx.set_global(GlobalPersonalSyncRuntime {
        active_config: None,
        runtime: None,
        service: Arc::new(RwLock::new(CloudSyncService::new())),
        status: PersonalSyncRuntimeStatus::Disabled,
        generation: 0,
        pending_auto_drain: false,
        _settings_subscription: settings_subscription,
        _local_event_subscription: local_event_subscription,
    });
    start_periodic_auto_sync(cx);
    reconcile_runtime(cx);
}

pub fn runtime_status(cx: &App) -> PersonalSyncRuntimeStatus {
    cx.try_global::<GlobalPersonalSyncRuntime>()
        .map(|state| state.status.clone())
        .unwrap_or_default()
}

pub fn actions_enabled(cx: &App) -> bool {
    let settings = AppSettings::global(cx);
    active_personal_sync_settings(settings)
        .is_some_and(|settings| build_personal_sync_runtime_config(&settings).is_ok())
}

pub fn test_connection(cx: &mut App) {
    let Some(config) = active_or_current_config(cx) else {
        set_status(cx, PersonalSyncRuntimeStatus::Disabled);
        return;
    };
    let generation = begin_operation(cx, PersonalSyncRuntimeStatus::Syncing);
    let task = Tokio::spawn(cx, async move {
        let store = ConfiguredPersonalSyncStore::from_runtime_config(&config);
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
    let task = Tokio::spawn(cx, async move {
        resolve_personal_conflict_once(
            config,
            source,
            (*conflicts).clone(),
            selection_key,
            strategy,
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
    let task = Tokio::spawn(cx, async move {
        for (selection_key, strategy) in strategies {
            resolve_personal_conflict_once(
                config.clone(),
                source.clone(),
                (*conflicts).clone(),
                selection_key,
                strategy,
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
                .map(|connection| connection_record_display(&connection))
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
            .map(|connection| connection_record_display(&connection)),
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

fn connection_record_display(connection: &StoredConnection) -> PersonalSyncRecordDisplay {
    PersonalSyncRecordDisplay {
        name: fallback_name(
            &connection.name,
            connection.cloud_id.as_deref(),
            "connection",
        ),
        info: record_info([
            Some(connection.connection_type.label().to_string()),
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
    let task = Tokio::spawn(cx, run_sync(config, source, conflict_sink));
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
        .and_then(|settings| build_personal_sync_runtime_config(&settings));
    match config {
        Ok(config) => {
            if runtime_config_unchanged(cx, &config) {
                return;
            }
            sync_master_key_and_user(cx);
            match start_running_runtime(cx, &config) {
                Ok(runtime) => {
                    let state = cx.global_mut::<GlobalPersonalSyncRuntime>();
                    state.active_config = Some(config);
                    state.runtime = Some(runtime);
                    state.status = PersonalSyncRuntimeStatus::Ready {
                        health: SyncStoreHealth::Ready,
                        message: None,
                    };
                }
                Err(error) => {
                    let state = cx.global_mut::<GlobalPersonalSyncRuntime>();
                    state.active_config = Some(config);
                    state.runtime = None;
                    state.status = PersonalSyncRuntimeStatus::from_error(error);
                }
            }
        }
        Err(PersonalSyncRuntimeError::Disabled | PersonalSyncRuntimeError::NotConfigured) => {
            let state = cx.global_mut::<GlobalPersonalSyncRuntime>();
            state.generation += 1;
            state.active_config = None;
            state.runtime = None;
            state.status = PersonalSyncRuntimeStatus::Disabled;
            state.pending_auto_drain = false;
        }
    }
}

fn runtime_config_unchanged(cx: &App, config: &PersonalSyncRuntimeConfig) -> bool {
    cx.try_global::<GlobalPersonalSyncRuntime>()
        .is_some_and(|state| {
            state.active_config.as_ref() == Some(config) && state.runtime.is_some()
        })
}

fn start_running_runtime(
    cx: &App,
    config: &PersonalSyncRuntimeConfig,
) -> Result<RunningPersonalSyncRuntime, SyncStoreError> {
    let source = build_local_source(cx).ok_or(SyncStoreError::NotConfigured)?;
    let conflict_sink = build_conflict_sink(cx).ok_or(SyncStoreError::NotConfigured)?;
    let store = ConfiguredPersonalSyncStore::from_runtime_config(config);
    let worker = PersonalSyncWorker::with_conflict_sink(
        store.clone(),
        source,
        conflict_sink,
        WorkerConfig {
            backend_profile_id: "personal".to_string(),
            device_id: SyncDeviceId("local-device".to_string()),
        },
    );
    let watcher = if config.auto_sync {
        Some(start_watcher(
            cx,
            config.root.clone(),
            worker.clone(),
            store.clone(),
        )?)
    } else {
        None
    };
    Ok(RunningPersonalSyncRuntime {
        store,
        worker,
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
            ..
        } => Some(PersonalSyncEvent::LocalDeleted {
            data_type: data_type::CONNECTION.to_string(),
            cloud_id: cloud_id.clone(),
        }),
        ConnectionDataEvent::WorkspaceDeleted {
            cloud_id: Some(cloud_id),
            ..
        } => Some(PersonalSyncEvent::LocalDeleted {
            data_type: data_type::WORKSPACE.to_string(),
            cloud_id: cloud_id.clone(),
        }),
        ConnectionDataEvent::CredentialDeleted {
            cloud_id: Some(cloud_id),
            ..
        } => Some(PersonalSyncEvent::LocalDeleted {
            data_type: data_type::CREDENTIAL.to_string(),
            cloud_id: cloud_id.clone(),
        }),
        ConnectionDataEvent::ConnectionDeleted { cloud_id: None, .. }
        | ConnectionDataEvent::WorkspaceCreated { .. }
        | ConnectionDataEvent::WorkspaceUpdated { .. }
        | ConnectionDataEvent::WorkspaceDeleted { cloud_id: None, .. }
        | ConnectionDataEvent::CredentialDeleted { cloud_id: None, .. } => {
            Some(PersonalSyncEvent::FullScan)
        }
        ConnectionDataEvent::SchemaChanged { .. }
        | ConnectionDataEvent::CloudSyncRequested
        | ConnectionDataEvent::TeamCacheUpdated => None,
    }
}

fn enqueue_local_change(event: PersonalSyncEvent, cx: &mut App) {
    enqueue_auto_sync_event(event, cx);
}

fn enqueue_periodic_full_scan(cx: &mut App) {
    enqueue_auto_sync_event(PersonalSyncEvent::FullScan, cx);
}

fn enqueue_auto_sync_event(event: PersonalSyncEvent, cx: &mut App) {
    let settings = AppSettings::global(cx);
    if !settings.sync_enabled
        || settings.sync_provider != SyncProvider::Personal
        || !settings.personal_sync.auto_sync
    {
        return;
    }
    sync_master_key_and_user(cx);
    let Some(runtime) = cx
        .try_global::<GlobalPersonalSyncRuntime>()
        .and_then(|state| state.runtime.as_ref())
    else {
        return;
    };
    runtime.worker.enqueue(event);
    if !should_start_drain_after_enqueue(&runtime_status(cx)) {
        cx.global_mut::<GlobalPersonalSyncRuntime>()
            .pending_auto_drain = true;
        return;
    }
    start_runtime_drain(cx);
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
    let generation = begin_operation(cx, PersonalSyncRuntimeStatus::Syncing);
    let task = Tokio::spawn(cx, drain_and_flush(worker, store));
    cx.spawn(async move |cx: &mut AsyncApp| {
        let status = personal_sync_status_from_task(task.await);
        let _ = cx.update(move |cx| finish_operation_and_maybe_drain(cx, generation, status));
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

fn start_periodic_auto_sync(cx: &mut App) {
    cx.spawn(|cx: &mut AsyncApp| {
        let cx = cx.clone();
        async move {
            loop {
                cx.background_executor()
                    .timer(PERSONAL_SYNC_PERIODIC_INTERVAL)
                    .await;
                cx.update(|cx| enqueue_periodic_full_scan(cx));
            }
        }
    })
    .detach();
}

fn start_watcher(
    cx: &App,
    root: std::path::PathBuf,
    worker: RunningPersonalSyncWorker,
    store: ConfiguredPersonalSyncStore,
) -> Result<PersonalSyncWatcher, SyncStoreError> {
    let handle = Tokio::handle(cx);
    PersonalSyncWatcher::start(root, Duration::from_secs(2), move |event| {
        worker.enqueue(event);
        let worker = worker.clone();
        let store = store.clone();
        handle.spawn(async move {
            if let Err(error) = worker.drain_once().await {
                tracing::warn!(error = %error, "Personal sync watcher drain failed");
                return;
            }
            if let Err(error) = store.flush().await {
                tracing::warn!(error = %error, "Personal sync watcher flush failed");
            }
        });
    })
}

async fn run_sync(
    config: PersonalSyncRuntimeConfig,
    source: PersonalSyncLocalRepositorySource,
    conflict_sink: SqlitePersonalSyncConflictSink,
) -> Result<(), SyncStoreError> {
    let store = ConfiguredPersonalSyncStore::from_runtime_config(&config);
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

async fn resolve_personal_conflict_once(
    config: PersonalSyncRuntimeConfig,
    source: PersonalSyncLocalRepositorySource,
    conflicts: PersonalSyncConflictRepository,
    selection_key: String,
    strategy: ConflictResolution,
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
    let store = ConfiguredPersonalSyncStore::from_runtime_config(&config);
    let resolver = PersonalSyncConflictResolver::new(store.clone(), source, conflicts);
    resolver.resolve(&conflict, strategy).await?;
    store.flush().await
}

fn build_local_source(cx: &App) -> Option<PersonalSyncLocalRepositorySource> {
    let storage = cx.try_global::<GlobalStorageState>()?.storage.clone();
    let connections = storage.get::<ConnectionRepository>()?;
    let credentials = storage.get::<CredentialRepository>()?;
    let workspaces = storage.get::<WorkspaceRepository>()?;
    let service = cx
        .try_global::<GlobalPersonalSyncRuntime>()?
        .service
        .clone();
    Some(PersonalSyncLocalRepositorySource::new(
        (*connections).clone(),
        (*credentials).clone(),
        (*workspaces).clone(),
        service,
    ))
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
    active_personal_sync_settings(AppSettings::global(cx))
        .and_then(|settings| build_personal_sync_runtime_config(&settings).ok())
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

fn finish_operation(cx: &mut App, generation: u64, status: PersonalSyncRuntimeStatus) {
    let state = cx.global_mut::<GlobalPersonalSyncRuntime>();
    if state.generation == generation {
        state.status = status;
    }
}

fn finish_operation_and_maybe_drain(
    cx: &mut App,
    generation: u64,
    status: PersonalSyncRuntimeStatus,
) {
    let should_drain = {
        let state = cx.global_mut::<GlobalPersonalSyncRuntime>();
        if state.generation == generation {
            state.status = status;
        }
        let pending = state.pending_auto_drain;
        state.pending_auto_drain = false;
        pending
    };
    if should_drain {
        start_runtime_drain(cx);
    }
}

fn personal_sync_status_from_task(
    result: Result<Result<(), SyncStoreError>, tokio::task::JoinError>,
) -> PersonalSyncRuntimeStatus {
    match result {
        Ok(Ok(())) => PersonalSyncRuntimeStatus::Ready {
            health: SyncStoreHealth::Ready,
            message: None,
        },
        Ok(Err(error)) => PersonalSyncRuntimeStatus::from_error(error),
        Err(error) => PersonalSyncRuntimeStatus::failed(&error.to_string()),
    }
}

fn set_status(cx: &mut App, status: PersonalSyncRuntimeStatus) {
    cx.global_mut::<GlobalPersonalSyncRuntime>().status = status;
}
