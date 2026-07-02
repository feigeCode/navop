use std::sync::{Arc, RwLock};
use std::time::Duration;

use gpui::{App, AsyncApp, Global, Subscription};
use one_core::cloud_sync::CloudSyncService;
use one_core::cloud_sync::personal::{
    ConfiguredPersonalSyncStore, PersonalSyncEvent, PersonalSyncLocalRepositorySource,
    PersonalSyncRuntimeConfig, PersonalSyncRuntimeError, PersonalSyncStore, PersonalSyncWatcher,
    PersonalSyncWorker, SyncDeviceId, SyncStoreError, SyncStoreHealth,
    build_personal_sync_runtime_config,
};
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::crypto;
use one_core::gpui_tokio::Tokio;
use one_core::settings::{AppSettings, GlobalCurrentUser, PersonalSyncSettings, SyncProvider};
use one_core::storage::{ConnectionRepository, GlobalStorageState, WorkspaceRepository};

use crate::personal_sync_status::PersonalSyncRuntimeStatus;

const PERSONAL_SYNC_PERIODIC_INTERVAL: Duration = Duration::from_secs(60);

pub struct GlobalPersonalSyncRuntime {
    active_config: Option<PersonalSyncRuntimeConfig>,
    runtime: Option<RunningPersonalSyncRuntime>,
    service: Arc<RwLock<CloudSyncService>>,
    status: PersonalSyncRuntimeStatus,
    generation: u64,
    _settings_subscription: Subscription,
    _local_event_subscription: Option<Subscription>,
}

impl Global for GlobalPersonalSyncRuntime {}

struct RunningPersonalSyncRuntime {
    store: ConfiguredPersonalSyncStore,
    worker: PersonalSyncWorker<ConfiguredPersonalSyncStore, PersonalSyncLocalRepositorySource>,
    _watcher: Option<PersonalSyncWatcher>,
}

pub fn init(cx: &mut App) {
    let settings_subscription = cx.observe_global::<AppSettings>(reconcile_runtime);
    let local_event_subscription = subscribe_local_events(cx);
    cx.set_global(GlobalPersonalSyncRuntime {
        active_config: None,
        runtime: None,
        service: Arc::new(RwLock::new(CloudSyncService::new())),
        status: PersonalSyncRuntimeStatus::Disabled,
        generation: 0,
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
    let Some(source) = build_local_source(cx) else {
        set_status(
            cx,
            PersonalSyncRuntimeStatus::failed("personal sync storage is unavailable"),
        );
        return;
    };
    sync_master_key_and_user(cx);
    let service = cx.global::<GlobalPersonalSyncRuntime>().service.clone();
    let generation = begin_operation(cx, PersonalSyncRuntimeStatus::Syncing);
    let task = Tokio::spawn(cx, run_sync(config, source, service));
    cx.spawn(async move |cx: &mut AsyncApp| {
        let status = match task.await {
            Ok(Ok(())) => PersonalSyncRuntimeStatus::Ready {
                health: SyncStoreHealth::Ready,
                message: None,
            },
            Ok(Err(error)) => PersonalSyncRuntimeStatus::from_error(error),
            Err(error) => PersonalSyncRuntimeStatus::failed(&error.to_string()),
        };
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
            state.active_config = None;
            state.runtime = None;
            state.status = PersonalSyncRuntimeStatus::Disabled;
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
    let store = ConfiguredPersonalSyncStore::from_runtime_config(config);
    let worker = PersonalSyncWorker::new(
        store.clone(),
        source,
        one_core::cloud_sync::personal::WorkerConfig {
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
            let local_id = connection.id?.to_string();
            Some(PersonalSyncEvent::LocalChanged {
                data_type: one_core::cloud_sync::data_type::CONNECTION.to_string(),
                local_id,
            })
        }
        ConnectionDataEvent::ConnectionDeleted {
            cloud_id: Some(cloud_id),
            ..
        } => Some(PersonalSyncEvent::LocalDeleted {
            data_type: one_core::cloud_sync::data_type::CONNECTION.to_string(),
            cloud_id: cloud_id.clone(),
        }),
        ConnectionDataEvent::WorkspaceDeleted {
            cloud_id: Some(cloud_id),
            ..
        } => Some(PersonalSyncEvent::LocalDeleted {
            data_type: one_core::cloud_sync::data_type::WORKSPACE.to_string(),
            cloud_id: cloud_id.clone(),
        }),
        ConnectionDataEvent::ConnectionDeleted { cloud_id: None, .. }
        | ConnectionDataEvent::WorkspaceCreated { .. }
        | ConnectionDataEvent::WorkspaceUpdated { .. }
        | ConnectionDataEvent::WorkspaceDeleted { cloud_id: None, .. } => {
            Some(PersonalSyncEvent::FullScan)
        }
        ConnectionDataEvent::SchemaChanged { .. } | ConnectionDataEvent::CloudSyncRequested => None,
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
    if settings.sync_provider != SyncProvider::Personal || !settings.personal_sync.auto_sync {
        return;
    }
    sync_master_key_and_user(cx);
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
    worker.enqueue(event);
    let generation = begin_operation(cx, PersonalSyncRuntimeStatus::Syncing);
    let task = Tokio::spawn(cx, drain_and_flush(worker, store));
    cx.spawn(async move |cx: &mut AsyncApp| {
        let status = match task.await {
            Ok(Ok(())) => PersonalSyncRuntimeStatus::Ready {
                health: SyncStoreHealth::Ready,
                message: None,
            },
            Ok(Err(error)) => PersonalSyncRuntimeStatus::from_error(error),
            Err(error) => PersonalSyncRuntimeStatus::failed(&error.to_string()),
        };
        let _ = cx.update(move |cx| finish_operation(cx, generation, status));
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

async fn drain_and_flush(
    worker: PersonalSyncWorker<ConfiguredPersonalSyncStore, PersonalSyncLocalRepositorySource>,
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
    worker: PersonalSyncWorker<ConfiguredPersonalSyncStore, PersonalSyncLocalRepositorySource>,
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
    _service: Arc<RwLock<CloudSyncService>>,
) -> Result<(), SyncStoreError> {
    let store = ConfiguredPersonalSyncStore::from_runtime_config(&config);
    let worker = PersonalSyncWorker::new(
        store.clone(),
        source,
        one_core::cloud_sync::personal::WorkerConfig {
            backend_profile_id: "personal".to_string(),
            device_id: SyncDeviceId("local-device".to_string()),
        },
    );
    worker.enqueue(PersonalSyncEvent::FullScan);
    worker.drain_once().await?;
    store.flush().await
}

fn build_local_source(cx: &App) -> Option<PersonalSyncLocalRepositorySource> {
    let storage = cx.try_global::<GlobalStorageState>()?.storage.clone();
    let connections = storage.get::<ConnectionRepository>()?;
    let workspaces = storage.get::<WorkspaceRepository>()?;
    let service = cx
        .try_global::<GlobalPersonalSyncRuntime>()?
        .service
        .clone();
    Some(PersonalSyncLocalRepositorySource::new(
        (*connections).clone(),
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
    cx.try_global::<GlobalPersonalSyncRuntime>()
        .and_then(|state| state.active_config.clone())
        .or_else(|| {
            active_personal_sync_settings(AppSettings::global(cx))
                .and_then(|settings| build_personal_sync_runtime_config(&settings).ok())
        })
}

fn active_personal_sync_settings(settings: &AppSettings) -> Option<PersonalSyncSettings> {
    if settings.sync_provider != SyncProvider::Personal {
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

fn set_status(cx: &mut App, status: PersonalSyncRuntimeStatus) {
    cx.global_mut::<GlobalPersonalSyncRuntime>().status = status;
}
