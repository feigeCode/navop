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
use one_core::crypto;
use one_core::gpui_tokio::Tokio;
use one_core::settings::{AppSettings, GlobalCurrentUser};
use one_core::storage::{ConnectionRepository, GlobalStorageState, WorkspaceRepository};

use crate::personal_sync_status::PersonalSyncRuntimeStatus;

pub struct GlobalPersonalSyncRuntime {
    active_config: Option<PersonalSyncRuntimeConfig>,
    runtime: Option<RunningPersonalSyncRuntime>,
    service: Arc<RwLock<CloudSyncService>>,
    status: PersonalSyncRuntimeStatus,
    generation: u64,
    _settings_subscription: Subscription,
}

impl Global for GlobalPersonalSyncRuntime {}

struct RunningPersonalSyncRuntime {
    _store: ConfiguredPersonalSyncStore,
    _worker: PersonalSyncWorker<ConfiguredPersonalSyncStore, PersonalSyncLocalRepositorySource>,
    _watcher: Option<PersonalSyncWatcher>,
}

pub fn init(cx: &mut App) {
    let settings_subscription = cx.observe_global::<AppSettings>(reconcile_runtime);
    cx.set_global(GlobalPersonalSyncRuntime {
        active_config: None,
        runtime: None,
        service: Arc::new(RwLock::new(CloudSyncService::new())),
        status: PersonalSyncRuntimeStatus::Disabled,
        generation: 0,
        _settings_subscription: settings_subscription,
    });
    reconcile_runtime(cx);
}

pub fn runtime_status(cx: &App) -> PersonalSyncRuntimeStatus {
    cx.try_global::<GlobalPersonalSyncRuntime>()
        .map(|state| state.status.clone())
        .unwrap_or_default()
}

pub fn actions_enabled(cx: &App) -> bool {
    let settings = AppSettings::global(cx);
    build_personal_sync_runtime_config(&settings.personal_sync).is_ok()
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
    match build_personal_sync_runtime_config(&settings.personal_sync) {
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
        _store: store,
        _worker: worker,
        _watcher: watcher,
    })
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
        .or_else(|| build_personal_sync_runtime_config(&AppSettings::global(cx).personal_sync).ok())
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
