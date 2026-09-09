//! Application-owned lifecycle for universal resource plugin runtimes.
//!
//! GPUI code may dispatch activation intents, but this service remains the
//! sole owner of provider processes and supervision.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, RwLock},
};

use extension_plugin_adapter::{
    ActivationError, ActivationHandle, ActivationManager, EventActivationManager, HostApiFactory,
    RuntimeHealth, RuntimeMonitor, RuntimeMonitorConfig, RuntimeMonitorEvent, SessionFactory,
    UniversalProviderHost,
};
use extension_runtime::{ExtensionRuntimeCatalog, GlobalExtensionRuntimeCatalog};
use one_core::gpui_tokio::Tokio;
use one_core::storage::{ConnectionRepository, GlobalStorageState};

/// A global wrapper that gives the service exactly one application owner.
#[derive(Clone)]
pub(crate) struct GlobalUniversalPluginService {
    service: UniversalPluginService,
}

impl gpui::Global for GlobalUniversalPluginService {}

impl GlobalUniversalPluginService {
    fn new(service: UniversalPluginService) -> Self {
        Self { service }
    }

    pub(crate) fn service(&self) -> UniversalPluginService {
        self.service.clone()
    }
}

/// The host-owned activation and supervision facade used by GPUI features.
#[derive(Clone)]
pub(crate) struct UniversalPluginService {
    manager: Arc<ActivationManager>,
    monitor: Arc<RuntimeMonitor>,
    catalog_source: GlobalExtensionRuntimeCatalog,
    catalog_revision: Arc<AtomicU64>,
    catalog_sync_lock: Arc<std::sync::Mutex<()>>,
    shutdown_lock: Arc<tokio::sync::Mutex<()>>,
    stopped: Arc<AtomicBool>,
    retiring_extensions: Arc<RwLock<HashSet<String>>>,
    activation_lock: Arc<tokio::sync::Mutex<()>>,
    transient_secrets: Arc<Mutex<HashMap<(String, String), Vec<u8>>>>,
}

impl UniversalPluginService {
    fn from_catalog_source(
        catalog_source: GlobalExtensionRuntimeCatalog,
        secrets: Option<Arc<ConnectionRepository>>,
    ) -> Self {
        let (catalog_revision, catalog) = catalog_source.snapshot();
        let catalog = catalog.unwrap_or_else(|| Arc::new(ExtensionRuntimeCatalog::empty()));
        let events = Arc::new(EventActivationManager::new());
        let jobs = Arc::new(extension_plugin_adapter::JobActivationManager::new());
        let blobs = extension_plugin_adapter::BlobStore::default();
        let transient_secrets = Arc::new(Mutex::new(HashMap::new()));
        let manager = Arc::new(
            ActivationManager::from_shared_catalog(
                catalog,
                production_session_factory(),
                production_host_api_factory(blobs.clone(), secrets, Arc::clone(&transient_secrets)),
            )
            .with_blob_store(blobs)
            .with_job_activation(jobs)
            .with_event_activation(Arc::clone(&events)),
        );
        let monitor = Arc::new(RuntimeMonitor::new(
            Arc::clone(&manager),
            RuntimeMonitorConfig::default(),
        ));
        Self {
            manager,
            monitor,
            catalog_source,
            catalog_revision: Arc::new(AtomicU64::new(catalog_revision)),
            catalog_sync_lock: Arc::new(std::sync::Mutex::new(())),
            shutdown_lock: Arc::new(tokio::sync::Mutex::new(())),
            stopped: Arc::new(AtomicBool::new(false)),
            retiring_extensions: Arc::new(RwLock::new(HashSet::new())),
            activation_lock: Arc::new(tokio::sync::Mutex::new(())),
            transient_secrets,
        }
    }

    #[cfg(test)]
    pub(crate) fn same_owner(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.manager, &other.manager) && Arc::ptr_eq(&self.monitor, &other.monitor)
    }

    #[allow(dead_code)]
    pub(crate) fn runtime_healths(&self) -> Vec<(String, RuntimeHealth)> {
        self.monitor
            .runtime_healths()
            .into_iter()
            .collect::<Vec<_>>()
    }

    #[allow(dead_code)]
    pub(crate) fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RuntimeMonitorEvent> {
        self.monitor.subscribe()
    }

    /// Activates a provider runtime by its namespaced runtime id.
    ///
    /// The returned handle is the caller's activation lease: release it with
    /// [`UniversalPluginService::deactivate_activation`].
    pub(crate) async fn activate_runtime(
        &self,
        runtime_id: &str,
    ) -> Result<ActivationHandle, ActivationError> {
        let _activation_guard = self.activation_lock.lock().await;
        self.sync_catalog();
        if self
            .runtime_extension(runtime_id)
            .is_some_and(|extension_id| {
                self.retiring_extensions
                    .read()
                    .is_ok_and(|retiring| retiring.contains(&extension_id))
            })
        {
            return Err(ActivationError::RuntimeNotReady {
                runtime_id: runtime_id.to_string(),
            });
        }
        let handle = self.manager.activate_runtime(runtime_id).await?;
        self.monitor.track(handle.runtime_id.clone());
        Ok(handle)
    }

    pub(crate) async fn deactivate_activation(
        &self,
        handle: &ActivationHandle,
    ) -> Result<(), ActivationError> {
        let result = self.manager.deactivate_activation(handle).await;
        if result.is_ok() && self.manager.runtime_state(&handle.runtime_id).is_err() {
            self.monitor.untrack(&handle.runtime_id);
        }
        result
    }

    /// Returns the current session generation for restart-aware clients.
    #[allow(dead_code)]
    pub(crate) fn runtime_generation(&self, runtime_id: &str) -> Result<u64, ActivationError> {
        self.manager.runtime_generation(runtime_id)
    }

    /// Acquires a client bound to the current activation-owned session.
    #[allow(dead_code)]
    pub(crate) fn universal_plugin_client(
        &self,
        runtime_id: &str,
    ) -> Result<
        extension_plugin_adapter::ManagedUniversalPluginClient,
        extension_plugin_adapter::ActivationError,
    > {
        self.sync_catalog();
        self.manager.universal_plugin_client(runtime_id)
    }

    pub(crate) fn shell_view(
        &self,
        extension_id: &str,
        view_id: &str,
    ) -> Option<extension_runtime::RegisteredShellViewContribution> {
        self.sync_catalog();
        self.catalog_source
            .get()?
            .shell_view(extension_id, view_id)
            .cloned()
    }

    pub(crate) fn resource_connection(
        &self,
        extension_id: &str,
        contribution_id: &str,
    ) -> Option<extension_runtime::RegisteredResourceConnectionContribution> {
        self.sync_catalog();
        self.catalog_source
            .get()?
            .resource_connection(extension_id, contribution_id)
            .cloned()
    }

    pub(crate) async fn deactivate_extension(&self, extension_id: &str) {
        let _activation_guard = self.activation_lock.lock().await;
        self.sync_catalog();
        let runtime_ids = self
            .catalog_source
            .get()
            .into_iter()
            .flat_map(|catalog| {
                catalog
                    .ipc_runtime_bindings()
                    .filter(|binding| binding.extension_id == extension_id)
                    .map(|binding| binding.runtime_key.clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for runtime_id in runtime_ids {
            let _ = self.manager.deactivate_runtime(&runtime_id).await;
            self.monitor.untrack(&runtime_id);
        }
    }

    pub(crate) async fn test_extension_connection(
        &self,
        contribution: extension_runtime::RegisteredResourceConnectionContribution,
        mut config: serde_json::Map<String, serde_json::Value>,
        secrets: HashMap<String, String>,
    ) -> anyhow::Result<()> {
        let prefix = format!("test-{}", uuid::Uuid::new_v4());
        let _guard = TransientSecretGuard::new(
            contribution.extension_id.clone(),
            prefix.clone(),
            secrets,
            Arc::clone(&self.transient_secrets),
        );
        config.insert(
            "credential_refs".into(),
            serde_json::Value::Object(
                _guard
                    .fields()
                    .map(|field| {
                        (
                            field.to_string(),
                            serde_json::Value::String(format!("secret://self/{prefix}:{field}")),
                        )
                    })
                    .collect(),
            ),
        );
        let activation = self.activate_runtime(&contribution.runtime_id).await?;
        let client = match self.universal_plugin_client(&contribution.runtime_id) {
            Ok(client) => client,
            Err(error) => {
                let _ = self.deactivate_activation(&activation).await;
                return Err(error.into());
            }
        };
        let opened = client
            .client()
            .open_resource(&extension_protocol::resource::ResourceOpenParams {
                resource_type: contribution.resource_type,
                config: serde_json::Value::Object(config),
                metadata: None,
            })
            .await;
        if let Ok(opened) = &opened {
            let _ = client
                .client()
                .close_resource(&extension_protocol::resource::ResourceCloseParams {
                    resource_id: opened.resource_id.clone(),
                })
                .await;
        }
        let _ = self.deactivate_activation(&activation).await;
        opened.map(|_| ()).map_err(Into::into)
    }

    pub(crate) fn begin_extension_retire(&self, extension_id: &str) {
        if let Ok(mut retiring) = self.retiring_extensions.write() {
            retiring.insert(extension_id.to_string());
        }
    }

    pub(crate) fn finish_extension_retire(&self, extension_id: &str) {
        if let Ok(mut retiring) = self.retiring_extensions.write() {
            retiring.remove(extension_id);
        }
    }

    fn runtime_extension(&self, runtime_id: &str) -> Option<String> {
        self.catalog_source
            .get()?
            .ipc_runtime_bindings()
            .find(|binding| binding.runtime_key == runtime_id)
            .map(|binding| binding.extension_id.clone())
    }

    #[allow(dead_code)]
    pub(crate) async fn invoke_resource_and_cache_blob(
        &self,
        runtime_id: &str,
        params: &extension_protocol::resource::ResourceInvokeParams,
    ) -> Result<
        extension_protocol::resource::ResourceInvokeResult,
        extension_plugin_adapter::ActivationError,
    > {
        self.sync_catalog();
        self.manager
            .invoke_resource_and_cache_blob(runtime_id, params)
            .await
    }

    #[allow(dead_code)]
    pub(crate) async fn job_result_and_cache_blob(
        &self,
        runtime_id: &str,
        params: &extension_protocol::job::JobResultParams,
    ) -> Result<extension_protocol::job::JobResultResult, extension_plugin_adapter::ActivationError>
    {
        self.sync_catalog();
        self.manager
            .job_result_and_cache_blob(runtime_id, params)
            .await
    }

    fn sync_catalog(&self) {
        let _sync_guard = self
            .catalog_sync_lock
            .lock()
            .expect("catalog sync lock poisoned");
        let (revision, catalog) = self.catalog_source.snapshot();
        if self.catalog_revision.load(Ordering::Acquire) >= revision {
            return;
        }
        let catalog = catalog.unwrap_or_else(|| Arc::new(ExtensionRuntimeCatalog::empty()));
        self.manager.replace_catalog(catalog);
        self.catalog_revision.store(revision, Ordering::Release);
    }

    /// Gracefully stops active runtimes and then stops the monitor task.
    pub(crate) async fn shutdown(&self) {
        let _shutdown_guard = self.shutdown_lock.lock().await;
        if self.stopped.load(Ordering::SeqCst) {
            return;
        }

        let runtime_ids = self
            .monitor
            .tracked_runtimes()
            .into_iter()
            .collect::<Vec<_>>();
        for runtime_id in runtime_ids {
            let _ = self.manager.deactivate_runtime(&runtime_id).await;
            self.monitor.untrack(&runtime_id);
        }
        self.monitor.stop().await;
        self.stopped.store(true, Ordering::SeqCst);
    }
}

impl UniversalPluginService {
    fn start_monitor(&self) -> Result<(), extension_plugin_adapter::RuntimeMonitorError> {
        if self.stopped.load(Ordering::SeqCst) {
            return Err(extension_plugin_adapter::RuntimeMonitorError::AlreadyRunning);
        }
        self.monitor.start()
    }
}

fn production_session_factory() -> SessionFactory {
    extension_plugin_adapter::process_session_factory()
}

fn production_host_api_factory(
    blobs: extension_plugin_adapter::BlobStore,
    secrets: Option<Arc<ConnectionRepository>>,
    transient_secrets: Arc<Mutex<HashMap<(String, String), Vec<u8>>>>,
) -> HostApiFactory {
    Arc::new(move |binding, generation| {
        let secret_resolver = ExtensionSecretResolver {
            extension_id: binding.extension_id.clone(),
            repository: secrets.clone(),
            transient_secrets: Arc::clone(&transient_secrets),
        };
        let host = UniversalProviderHost::new(
            binding.permissions.iter().cloned(),
            Arc::new(secret_resolver),
        )
        .with_blob_store(
            blobs.clone(),
            extension_plugin_adapter::BlobOwner {
                runtime_id: binding.runtime_key.clone(),
                generation,
            },
        );
        Arc::new(extension_host::HostApiHandler::new(Arc::new(host)))
    })
}

struct ExtensionSecretResolver {
    extension_id: String,
    repository: Option<Arc<ConnectionRepository>>,
    transient_secrets: Arc<Mutex<HashMap<(String, String), Vec<u8>>>>,
}

#[async_trait::async_trait]
impl extension_plugin_adapter::SecretResolver for ExtensionSecretResolver {
    async fn resolve(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Vec<u8>, extension_host::HostError> {
        if namespace != "self" {
            return Err(extension_host::HostError::protocol(
                extension_protocol::ProtocolError::new(
                    extension_protocol::error_codes::PERMISSION_DENIED,
                    "extension secrets must use the self namespace",
                ),
            ));
        }
        let extension_id = self.extension_id.clone();
        let key = key.to_string();
        if let Some(secret) = self
            .transient_secrets
            .lock()
            .ok()
            .and_then(|secrets| secrets.get(&(extension_id.clone(), key.clone())).cloned())
        {
            return Ok(secret);
        }
        let repository = self.repository.clone().ok_or_else(|| {
            extension_host::HostError::NotImplemented(
                "extension connection secret storage is unavailable".into(),
            )
        })?;
        tokio::task::spawn_blocking(move || {
            repository.resolve_extension_secret(&extension_id, &key)
        })
        .await
        .map_err(|error| extension_host::HostError::ProcessExited(error.to_string()))?
        .map_err(|error| {
            extension_host::HostError::protocol(extension_protocol::ProtocolError::new(
                extension_protocol::error_codes::SECRET_NOT_FOUND,
                error.to_string(),
            ))
        })
    }
}

struct TransientSecretGuard {
    extension_id: String,
    prefix: String,
    fields: Vec<String>,
    store: Arc<Mutex<HashMap<(String, String), Vec<u8>>>>,
}

impl TransientSecretGuard {
    fn new(
        extension_id: String,
        prefix: String,
        secrets: HashMap<String, String>,
        store: Arc<Mutex<HashMap<(String, String), Vec<u8>>>>,
    ) -> Self {
        let fields = secrets.keys().cloned().collect::<Vec<_>>();
        if let Ok(mut values) = store.lock() {
            for (field, value) in secrets {
                values.insert(
                    (extension_id.clone(), format!("{prefix}:{field}")),
                    value.into_bytes(),
                );
            }
        }
        Self {
            extension_id,
            prefix,
            fields,
            store,
        }
    }

    fn fields(&self) -> impl Iterator<Item = &str> {
        self.fields.iter().map(String::as_str)
    }
}

impl Drop for TransientSecretGuard {
    fn drop(&mut self) {
        if let Ok(mut values) = self.store.lock() {
            for field in &self.fields {
                if let Some(mut value) = values.remove(&(
                    self.extension_id.clone(),
                    format!("{}:{field}", self.prefix),
                )) {
                    value.fill(0);
                }
            }
        }
    }
}

pub(crate) fn init(cx: &mut gpui::App) {
    assert!(
        cx.try_global::<GlobalUniversalPluginService>().is_none(),
        "universal plugin service must have exactly one application owner"
    );

    let catalog_source = cx.default_global::<GlobalExtensionRuntimeCatalog>().clone();
    let secrets = cx
        .try_global::<GlobalStorageState>()
        .and_then(|storage| storage.storage.get::<ConnectionRepository>());
    let global = GlobalUniversalPluginService::new(UniversalPluginService::from_catalog_source(
        catalog_source,
        secrets,
    ));
    let service = global.service();
    let startup_service = service.clone();
    let quit_service = service.clone();
    cx.set_global(global);
    gpui_shell::init_embedded(cx);
    match crate::shell_plugin_host::ShellPluginHost::new(service, cx) {
        Ok(host) => {
            #[cfg(not(test))]
            host.start_monitor_bridge(cx);
            extension_view::register_shell_view_opener(std::rc::Rc::new(host.clone()), cx);
            cx.set_global(host);
        }
        Err(error) => tracing::warn!(%error, "failed to initialize gpui-shell host"),
    }

    // RuntimeMonitor::start must be called from the Tokio runtime. Starting it
    // through a spawned task also keeps startup off the GPUI foreground thread.
    Tokio::spawn(cx, async move {
        if let Err(error) = startup_service.start_monitor() {
            tracing::warn!(%error, "failed to start universal plugin runtime monitor");
        }
    })
    .detach();

    // Normal quit paths await `shutdown` before `cx.quit()`. This is only the
    // bounded fallback for platform-driven quit paths.
    cx.on_app_quit(move |cx| {
        let quit_service = quit_service.clone();
        let shutdown_task = Tokio::spawn(cx, async move { quit_service.shutdown().await });
        async move {
            if let Err(error) = shutdown_task.await {
                tracing::warn!(%error, "universal plugin shutdown fallback did not complete");
            }
        }
    })
    .detach();
}

pub(crate) fn spawn_shutdown(
    cx: &gpui::App,
) -> Option<gpui::Task<Result<(), one_core::gpui_tokio::JoinError>>> {
    let service = cx.try_global::<GlobalUniversalPluginService>()?.service();
    Some(Tokio::spawn(cx, async move { service.shutdown().await }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn universal_plugin_service_has_one_application_owner(cx: &mut gpui::TestAppContext) {
        let (first, second, runtime) = cx.update(|cx| {
            one_core::gpui_tokio::init(cx);
            extension_runtime::init(cx);
            init(cx);

            let first = cx.global::<GlobalUniversalPluginService>().service();
            let second = cx.global::<GlobalUniversalPluginService>().service();
            let runtime = one_core::gpui_tokio::Tokio::handle(cx);
            (first, second, runtime)
        });

        assert!(first.same_owner(&second));
        runtime.block_on(first.shutdown());
        runtime.block_on(second.shutdown());
    }

    #[tokio::test]
    async fn transient_extension_secret_does_not_require_connection_repository() {
        let store = Arc::new(Mutex::new(HashMap::from([(
            ("com.example.search".into(), "test-1:api_key".into()),
            b"secret-value".to_vec(),
        )])));
        let resolver = ExtensionSecretResolver {
            extension_id: "com.example.search".into(),
            repository: None,
            transient_secrets: store,
        };

        let secret =
            extension_plugin_adapter::SecretResolver::resolve(&resolver, "self", "test-1:api_key")
                .await
                .unwrap();

        assert_eq!(b"secret-value", secret.as_slice());
    }
}
