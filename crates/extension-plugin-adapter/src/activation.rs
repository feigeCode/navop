//! Host-authoritative lazy activation for native IPC provider runtimes.
//!
//! This layer owns provider process sessions and activation references. It does
//! not mount a GPUI view; a later gpui-shell integration layer consumes these
//! runtime services after activation succeeds.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
    time::Duration,
};

use crate::blob_store::BlobStore;
use crate::event_activation::EventActivationManager;
use crate::job_activation::{JobActivationHandle, JobActivationManager, RetiredJob};
use crate::provider_permissions::ResourceOpenAuthorizer;
use extension_host::{HostError, ProcessRpcSession, RequestOptions};
use extension_protocol::blob::{
    BlobCloseParams, BlobOpenParams, BlobReadParams, BlobReadResult, INLINE_BLOB_THRESHOLD_BYTES,
};
use extension_protocol::event_stream::{
    EventCloseParams, EventOpenParams, EventOpenResult, EventReadParams, EventReadResult,
};
use extension_protocol::job::{
    JobCancelParams, JobCloseParams, JobResultParams, JobResultResult, JobStartParams,
    JobStatusParams, JobStatusResult,
};
use extension_protocol::resource::{ResourceInvokeParams, ResourceInvokeResult};
use extension_protocol::result_ref::ResultRef;
use extension_runtime::{
    ExtensionRuntimeCatalog, RegisteredIpcRuntimeBinding, extension::manifest::current_host_version,
};
use futures::FutureExt;
use futures::future::BoxFuture;
use parking_lot::{Mutex as SyncMutex, RwLock};
use tokio::{
    sync::{Mutex, Notify, broadcast, mpsc},
    time::{Instant, sleep},
};

use thiserror::Error;

/// Asynchronous factory used to start a runtime session.
///
/// The indirection keeps process supervision independently testable. The
/// production implementation is [`process_session_factory`].
pub type SessionFactory = Arc<
    dyn Fn(SessionContext) -> BoxFuture<'static, Result<Arc<dyn ManagedRpcSession>, HostError>>
        + Send
        + Sync,
>;

/// Inputs needed to start one activation-owned runtime session.
#[derive(Debug, Clone)]
pub struct SessionContext {
    pub binding: RegisteredIpcRuntimeBinding,
    pub host_api: Arc<extension_host::HostApiHandler>,
}

/// Creates permission-enforcing reverse Host API dispatchers for activations.
pub type HostApiFactory = Arc<
    dyn Fn(RegisteredIpcRuntimeBinding, u64) -> Arc<extension_host::HostApiHandler> + Send + Sync,
>;

/// The process-session capability required by the activation manager.
///
/// `ProcessRpcSession` is the only production implementation. The trait exists
/// so ownership semantics can be tested without spawning a child process.
pub trait ManagedRpcSession: Send + Sync {
    fn shutdown<'a>(&'a self) -> BoxFuture<'a, ()>;

    /// Reports whether the transport or child process has already exited.
    fn is_closed(&self) -> bool {
        false
    }

    /// Performs a process-level health request.
    ///
    /// The default is useful for in-process test doubles. A failed ping does
    /// not by itself prove that a process crashed; supervision only restarts
    /// after `is_closed` confirms closure.
    fn ping<'a>(&'a self) -> BoxFuture<'a, Result<(), HostError>> {
        async {
            if self.is_closed() {
                Err(HostError::Closed)
            } else {
                Ok(())
            }
        }
        .boxed()
    }

    fn universal_plugin_client(
        &self,
        _open_authorizer: Option<extension_host::OpenAuthorizer>,
    ) -> Option<extension_host::UniversalPluginClient> {
        None
    }
}

impl ManagedRpcSession for ProcessRpcSession {
    fn shutdown<'a>(&'a self) -> BoxFuture<'a, ()> {
        ProcessRpcSession::shutdown(self).boxed()
    }

    fn universal_plugin_client(
        &self,
        open_authorizer: Option<extension_host::OpenAuthorizer>,
    ) -> Option<extension_host::UniversalPluginClient> {
        let session = Arc::new(self.clone_session());
        let client = extension_host::UniversalPluginClient::new(session);
        Some(match open_authorizer {
            Some(authorizer) => client.with_open_authorizer(authorizer),
            None => client,
        })
    }

    fn is_closed(&self) -> bool {
        ProcessRpcSession::is_closed(self)
    }

    fn ping<'a>(&'a self) -> BoxFuture<'a, Result<(), HostError>> {
        if !self.declares_method(extension_protocol::method::PING) {
            return async { Ok(()) }.boxed();
        }
        ProcessRpcSession::request_value(
            self,
            extension_protocol::method::PING,
            serde_json::Value::Null,
        )
        .map(|result| result.map(|_| ()))
        .boxed()
    }
}

/// Build the production IPC session factory for native resource providers.
///
/// Each call receives a fresh instance ID, negotiates the extension API, and
/// starts one `ProcessRpcSession`. Restart/backoff supervision is owned by
/// [`ActivationManager`] and remains host-authoritative.
pub fn process_session_factory() -> SessionFactory {
    Arc::new(|context| {
        Box::pin(async move {
            let host_version = current_host_version().to_string();
            let instance_id = uuid::Uuid::new_v4().to_string();
            let negotiation = extension_host::NegotiationConfig::new(host_version, instance_id)
                .offer_api("extension", "1.0");
            let config = crate::process_session_config(&context.binding, negotiation)
                .map_err(|error| HostError::Config(error.to_string()))?;
            let config = config.with_host_api(context.host_api);
            let session = ProcessRpcSession::start(config).await?;
            Ok(Arc::new(session) as Arc<dyn ManagedRpcSession>)
        })
    })
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ActivationError {
    #[error("IPC runtime `{runtime_id}` is not registered")]
    RuntimeNotFound { runtime_id: String },
    #[error("failed to activate runtime: {0}")]
    SessionStart(String),
    #[error("host blob cache operation failed: {0}")]
    HostBlob(String),
    #[error("failed to activate runtime `{runtime_id}`")]
    InvalidRuntime { runtime_id: String },
    #[error("runtime `{runtime_id}` has no active session")]
    RuntimeNotReady { runtime_id: String },
}

/// Host-owned restart timing policy.
///
/// Extension manifests choose whether restart is enabled and how many restarts
/// are budgeted; timing remains host-authoritative and cannot be configured by
/// an extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupervisionPolicy {
    pub initial_restart_backoff: Duration,
    pub max_restart_backoff: Duration,
    pub backoff_multiplier: u32,
}

impl Default for SupervisionPolicy {
    fn default() -> Self {
        Self {
            initial_restart_backoff: Duration::from_millis(250),
            max_restart_backoff: Duration::from_secs(8),
            backoff_multiplier: 2,
        }
    }
}

impl SupervisionPolicy {
    fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        let mut backoff = self.initial_restart_backoff;
        for _ in 1..attempt {
            backoff = backoff
                .checked_mul(self.backoff_multiplier)
                .unwrap_or(self.max_restart_backoff)
                .min(self.max_restart_backoff);
        }
        backoff
    }
}

/// A point-in-time supervision snapshot for an activated runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHealth {
    pub state: RuntimeActivationState,
    pub session_closed: bool,
    pub ping_error: Option<String>,
    pub restart_attempts: u32,
    pub restart_budget: u32,
    pub restart_backoff_remaining: Option<Duration>,
}

impl From<HostError> for ActivationError {
    fn from(error: HostError) -> Self {
        Self::SessionStart(error.to_string())
    }
}

impl ActivationError {
    fn session_start(error: HostError) -> Self {
        Self::SessionStart(error.to_string())
    }
}

/// The result of a successful runtime activation.
///
/// This value describes ownership and is safe to copy into a UI entity. It does
/// not own or control the provider process; shutdown remains manager-owned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationHandle {
    pub extension_id: String,
    pub runtime_id: String,
    /// Stable identity for this activation mount.
    ///
    /// Unlike `runtime_generation`, this value survives provider process
    /// restarts and changes only after the activation is fully released and
    /// activated again.
    pub activation_id: u64,
    /// Process generation observed when activation completed.
    pub runtime_generation: u64,
    pub state: RuntimeActivationState,
}

/// A typed client bound to one activation-owned session generation.
///
/// The generation lets callers detect that a supervisor has replaced a closed
/// process and reacquire a client instead of silently using the old transport.
#[derive(Clone)]
pub struct ManagedUniversalPluginClient {
    pub runtime_id: String,
    pub generation: u64,
    extension_id: String,
    client: extension_host::UniversalPluginClient,
    blobs: Option<Arc<BlobStore>>,
    events: Option<Arc<EventActivationManager>>,
    jobs: Option<Arc<JobActivationManager>>,
}

impl ManagedUniversalPluginClient {
    pub fn client(&self) -> &extension_host::UniversalPluginClient {
        &self.client
    }

    pub fn runtime_generation(&self) -> u64 {
        self.generation
    }

    pub fn blob_owner(&self) -> crate::BlobOwner {
        crate::BlobOwner {
            runtime_id: self.runtime_id.clone(),
            generation: self.generation,
        }
    }

    pub fn blob_store(&self) -> Option<&BlobStore> {
        self.blobs.as_deref()
    }

    pub fn event_activation(&self) -> Option<&EventActivationManager> {
        self.events.as_deref()
    }

    pub fn job_activation(&self) -> Option<&JobActivationManager> {
        self.jobs.as_deref()
    }

    fn with_blob_store(mut self, blobs: Arc<BlobStore>) -> Self {
        self.blobs = Some(blobs);
        self
    }

    fn with_event_activation(mut self, events: Arc<EventActivationManager>) -> Self {
        self.events = Some(events);
        self
    }

    fn with_job_activation(mut self, jobs: Arc<JobActivationManager>) -> Self {
        self.jobs = Some(jobs);
        self
    }

    pub async fn start_job(
        &self,
        params: &JobStartParams,
    ) -> Result<JobActivationHandle, HostError> {
        let result = self.client.start_job(params).await?;
        let Some(jobs) = &self.jobs else {
            return Ok(JobActivationHandle {
                extension_id: self.extension_id.clone(),
                runtime_id: self.runtime_id.clone(),
                generation: self.generation,
                job_id: result.job_id,
            });
        };
        match jobs.register_start(
            &self.extension_id,
            &self.runtime_id,
            self.generation,
            &result,
        ) {
            Ok(handle) => Ok(handle),
            Err(error) => {
                let _ = self
                    .client
                    .close_job(&JobCloseParams {
                        job_id: result.job_id,
                    })
                    .await;
                Err(error)
            }
        }
    }

    pub async fn job_status(
        &self,
        handle: &JobActivationHandle,
    ) -> Result<JobStatusResult, HostError> {
        self.validate_job(handle)?;
        let result = self
            .client
            .job_status(&JobStatusParams {
                job_id: handle.job_id.clone(),
            })
            .await?;
        if let Some(jobs) = &self.jobs {
            jobs.update_status(handle, &result)?;
        }
        Ok(result)
    }

    pub async fn cancel_job(&self, handle: &JobActivationHandle) -> Result<(), HostError> {
        self.validate_job(handle)?;
        self.client
            .cancel_job(&JobCancelParams {
                job_id: handle.job_id.clone(),
            })
            .await
    }

    pub async fn job_result(
        &self,
        handle: &JobActivationHandle,
    ) -> Result<JobResultResult, HostError> {
        self.validate_job(handle)?;
        let result = self
            .client
            .job_result(&JobResultParams {
                job_id: handle.job_id.clone(),
            })
            .await?;
        self.validate_job(handle)?;
        if let Some(jobs) = &self.jobs {
            jobs.mark_result_observed(handle)?;
        }
        Ok(result)
    }

    pub async fn close_job(&self, handle: &JobActivationHandle) -> Result<(), HostError> {
        self.validate_job(handle)?;
        let result = self
            .client
            .close_job(&JobCloseParams {
                job_id: handle.job_id.clone(),
            })
            .await;
        self.cleanup_job(handle);
        result
    }

    fn validate_job(&self, handle: &JobActivationHandle) -> Result<(), HostError> {
        if handle.extension_id != self.extension_id
            || handle.runtime_id != self.runtime_id
            || handle.generation != self.generation
        {
            return Err(HostError::protocol(extension_protocol::ProtocolError::new(
                extension_protocol::error::error_codes::PERMISSION_DENIED,
                "job handle is not owned by this runtime generation",
            )));
        }
        if let Some(jobs) = &self.jobs {
            jobs.validate(handle)?;
        }
        Ok(())
    }

    fn cleanup_job(&self, handle: &JobActivationHandle) {
        let Some(jobs) = &self.jobs else {
            return;
        };
        let blob_ids = jobs.close(handle);
        let Some(blobs) = &self.blobs else {
            return;
        };
        for blob_id in blob_ids {
            let _ = blobs.remove_owned_blob(&self.blob_owner(), &blob_id);
        }
    }

    /// Opens a provider event stream and registers its host-owned identity.
    ///
    /// If the runtime is replaced while the provider response is in flight,
    /// the stale result is rejected and the provider stream is closed without
    /// being registered for the replacement generation.
    pub async fn open_event_stream(
        &self,
        params: &EventOpenParams,
    ) -> Result<EventOpenResult, HostError> {
        let result = self.client.open_event_stream(params).await?;
        if let Err(error) = self.register_event_stream(&result) {
            // Registration can fail after the provider has already allocated a
            // stream. Always attempt provider-side cleanup before surfacing the
            // host lifecycle error.
            let _ = self
                .client
                .close_event_stream(&EventCloseParams {
                    stream_id: result.stream_id.clone(),
                })
                .await;
            return Err(error);
        }
        Ok(result)
    }

    /// Reads an exact extension/runtime/generation-owned event stream.
    ///
    /// A terminal `closed` result removes host registration. Transient RPC
    /// errors keep the registration; lifecycle cleanup remains authoritative.
    pub async fn read_event_stream(
        &self,
        params: &EventReadParams,
    ) -> Result<EventReadResult, HostError> {
        self.read_event_stream_with_options(params, RequestOptions::default())
            .await
    }

    pub async fn read_event_stream_with_options(
        &self,
        params: &EventReadParams,
        options: RequestOptions,
    ) -> Result<EventReadResult, HostError> {
        self.ensure_event_read(params)?;
        let result = self
            .client
            .read_event_stream_with_options(params, options)
            .await?;
        if result.closed {
            self.complete_event_stream(&params.stream_id);
        }
        Ok(result)
    }

    /// Closes the provider stream and removes only this owner's registration.
    ///
    /// Host cleanup happens even when provider close fails; the process may be
    /// gone, and retaining a permanently unusable stream would leak capacity.
    pub async fn close_event_stream(&self, params: &EventCloseParams) -> Result<(), HostError> {
        self.ensure_event_close(params)?;
        let result = self.client.close_event_stream(params).await;
        self.complete_event_stream(&params.stream_id);
        result
    }

    fn register_event_stream(&self, result: &EventOpenResult) -> Result<(), HostError> {
        let Some(events) = &self.events else {
            return Ok(());
        };
        events.open(
            &self.extension_id,
            &self.runtime_id,
            self.generation,
            result,
        )?;
        Ok(())
    }

    fn ensure_event_read(&self, params: &EventReadParams) -> Result<(), HostError> {
        let Some(events) = &self.events else {
            return Ok(());
        };
        events.validate_read(
            &self.extension_id,
            &self.runtime_id,
            self.generation,
            params,
        )?;
        Ok(())
    }

    fn ensure_event_close(&self, params: &EventCloseParams) -> Result<(), HostError> {
        let Some(events) = &self.events else {
            return Ok(());
        };
        events.validate_close(
            &self.extension_id,
            &self.runtime_id,
            self.generation,
            params,
        )?;
        Ok(())
    }

    fn complete_event_stream(&self, stream_id: &str) {
        if let Some(events) = &self.events {
            events.complete(
                &self.extension_id,
                &self.runtime_id,
                self.generation,
                stream_id,
            );
        }
    }

    /// Reads either a provider-owned blob or a host-owned result blob.
    ///
    /// Host blob ids are a distinct wire namespace. They never fall through to
    /// a provider, which prevents a replacement process from minting an id that
    /// reads data from another generation's host cache.
    pub async fn read_blob(&self, params: &BlobReadParams) -> Result<BlobReadResult, HostError> {
        if let Some(blobs) = self
            .blobs
            .as_ref()
            .filter(|_| is_host_blob_id(&params.blob_id))
        {
            return blobs.read(&self.owner(), params).map_err(host_blob_error);
        }
        self.client.read_blob(params).await
    }

    /// Closes either a provider-owned blob or the matching host-owned blob.
    pub async fn close_blob(&self, params: &BlobCloseParams) -> Result<(), HostError> {
        if let Some(blobs) = self
            .blobs
            .as_ref()
            .filter(|_| is_host_blob_id(&params.blob_id))
        {
            return blobs.close(&self.owner(), params).map_err(host_blob_error);
        }
        self.client.close_blob(params).await
    }

    fn owner(&self) -> crate::BlobOwner {
        self.blob_owner()
    }
}

/// Lets the activation manager cache the inline variants of resource/job results.
trait CachedResult {
    fn inline_value(&self) -> Option<&serde_json::Value>;
    fn replace_with_blob(&mut self, blob_id: extension_protocol::blob::BlobId);
}

impl CachedResult for ResourceInvokeResult {
    fn inline_value(&self) -> Option<&serde_json::Value> {
        match &self.result {
            ResultRef::Inline { value } => Some(value),
            ResultRef::Blob { .. } | ResultRef::EventStream { .. } => None,
        }
    }

    fn replace_with_blob(&mut self, blob_id: extension_protocol::blob::BlobId) {
        self.result = ResultRef::Blob { id: blob_id };
    }
}

impl CachedResult for JobResultResult {
    fn inline_value(&self) -> Option<&serde_json::Value> {
        match &self.result {
            ResultRef::Inline { value } => Some(value),
            ResultRef::Blob { .. } | ResultRef::EventStream { .. } => None,
        }
    }

    fn replace_with_blob(&mut self, blob_id: extension_protocol::blob::BlobId) {
        self.result = ResultRef::Blob { id: blob_id };
    }
}

fn is_host_blob_id(blob_id: &str) -> bool {
    blob_id.starts_with("host-blob-")
}

fn host_blob_error(error: crate::BlobStoreError) -> HostError {
    HostError::protocol(error.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeActivationState {
    Starting,
    Restarting,
    Active,
    /// The process transport is still open, but its health ping failed.
    Degraded,
    /// Restart is disabled or a restart could not be initiated.
    Failed,
    /// The configured restart budget has been exhausted.
    CrashLoop,
}

struct ActivatedRuntime {
    extension_id: String,
    binding: RegisteredIpcRuntimeBinding,
    activations: BTreeSet<u64>,
    state: RuntimeActivationState,
    session: Option<Arc<dyn ManagedRpcSession>>,
    start_generation: u64,
    factory_claimed: bool,
    restart_attempts: u32,
    next_restart_at: Option<Instant>,
}

#[derive(Default)]
struct ActivationState {
    runtimes: BTreeMap<String, ActivatedRuntime>,
    start_locks: BTreeMap<String, Arc<Mutex<()>>>,
    deactivations: BTreeMap<String, u64>,
    next_activation_id: u64,
}

impl ActivationState {
    fn allocate_activation_id(&mut self) -> u64 {
        let activation_id = self.next_activation_id;
        self.next_activation_id = self
            .next_activation_id
            .checked_add(1)
            .expect("activation id space exhausted");
        activation_id
    }
}

struct StartingRuntime {
    binding: RegisteredIpcRuntimeBinding,
    generation: u64,
}

struct StartClaimGuard {
    state: Arc<SyncMutex<ActivationState>>,
    runtime_id: String,
    generation: u64,
    armed: bool,
}

impl StartClaimGuard {
    fn new(state: Arc<SyncMutex<ActivationState>>, runtime_id: String, generation: u64) -> Self {
        Self {
            state,
            runtime_id,
            generation,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StartClaimGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut state = self.state.lock();
        let abandoned = state.runtimes.get(&self.runtime_id).is_some_and(|runtime| {
            runtime.start_generation == self.generation
                && runtime.factory_claimed
                && runtime.session.is_none()
        });
        if abandoned {
            state.runtimes.remove(&self.runtime_id);
        }
    }
}

enum CheckDecision {
    Return(RuntimeHealth, Option<Arc<dyn ManagedRpcSession>>),
    Restart {
        binding: Box<RegisteredIpcRuntimeBinding>,
        generation: u64,
        attempt: u32,
    },
}

/// Manages lazy runtime starts, runtime sharing across panels, and reference-counted shutdown.
pub struct ActivationManager {
    catalog: Arc<RwLock<Arc<ExtensionRuntimeCatalog>>>,
    session_factory: SessionFactory,
    host_api_factory: HostApiFactory,
    supervision_policy: SupervisionPolicy,
    blobs: Option<Arc<BlobStore>>,
    events: Option<Arc<EventActivationManager>>,
    jobs: Option<Arc<JobActivationManager>>,
    state: Arc<SyncMutex<ActivationState>>,
}

impl ActivationManager {
    pub fn new(
        catalog: ExtensionRuntimeCatalog,
        session_factory: SessionFactory,
        host_api_factory: HostApiFactory,
    ) -> Self {
        Self::from_shared_catalog(Arc::new(catalog), session_factory, host_api_factory)
    }

    /// Creates a manager sharing the application's immutable catalog snapshot.
    ///
    /// The GPUI runtime catalog is stored as an `Arc`. Sharing that value here
    /// avoids loading installed extension manifests twice while preserving a
    /// single ownership point for activation authorization.
    pub fn from_shared_catalog(
        catalog: Arc<ExtensionRuntimeCatalog>,
        session_factory: SessionFactory,
        host_api_factory: HostApiFactory,
    ) -> Self {
        Self {
            catalog: Arc::new(RwLock::new(catalog)),
            session_factory,
            host_api_factory,
            supervision_policy: SupervisionPolicy::default(),
            blobs: None,
            events: None,
            jobs: None,
            state: Arc::new(SyncMutex::new(ActivationState::default())),
        }
    }

    pub fn with_supervision_policy(mut self, policy: SupervisionPolicy) -> Self {
        self.supervision_policy = policy;
        self
    }

    /// Attaches a process-wide store for provider result blobs.
    ///
    /// Once attached, retired runtime generations cannot leave cached result
    /// data behind. Restarting a provider increments its generation before
    /// cleanup, so an old client cannot read a replacement process's data.
    pub fn with_blob_store(mut self, blobs: BlobStore) -> Self {
        self.blobs = Some(Arc::new(blobs));
        self
    }

    pub fn blob_store(&self) -> Option<&BlobStore> {
        self.blobs.as_deref()
    }

    /// Attaches the host-owned lifecycle manager for provider event streams.
    ///
    /// The provider still produces and buffers events, while Navop owns the
    /// stream registry, authorization, and cleanup across restart generations.
    pub fn with_event_activation(mut self, events: Arc<EventActivationManager>) -> Self {
        self.events = Some(events);
        self
    }

    pub fn event_activation(&self) -> Option<&EventActivationManager> {
        self.events.as_deref()
    }

    pub fn with_job_activation(mut self, jobs: Arc<JobActivationManager>) -> Self {
        self.jobs = Some(jobs);
        self
    }

    pub fn job_activation(&self) -> Option<&JobActivationManager> {
        self.jobs.as_deref()
    }

    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }

    pub fn replace_catalog(&self, catalog: Arc<ExtensionRuntimeCatalog>) {
        *self.catalog.write() = catalog;
    }

    pub async fn activate_runtime(
        &self,
        runtime_id: &str,
    ) -> Result<ActivationHandle, ActivationError> {
        let (binding, runtime_id, generation, activation_id) = {
            let mut state = self.state.lock();
            let binding = self
                .catalog
                .read()
                .ipc_runtime_bindings()
                .find(|binding| binding.runtime_key == runtime_id)
                .cloned()
                .ok_or_else(|| ActivationError::RuntimeNotFound {
                    runtime_id: runtime_id.to_owned(),
                })?;

            if state.runtimes.contains_key(runtime_id) {
                let activation_id = state.allocate_activation_id();
                let runtime = state
                    .runtimes
                    .get(runtime_id)
                    .expect("runtime still exists");
                if runtime.binding != binding {
                    return Err(ActivationError::RuntimeNotReady {
                        runtime_id: runtime_id.to_owned(),
                    });
                }
                if runtime
                    .session
                    .as_ref()
                    .is_some_and(|session| !session.is_closed())
                {
                    let runtime = state
                        .runtimes
                        .get_mut(runtime_id)
                        .expect("runtime still exists");
                    runtime.activations.insert(activation_id);
                    return Ok(ActivationHandle {
                        extension_id: runtime.extension_id.clone(),
                        runtime_id: runtime_id.to_owned(),
                        activation_id,
                        runtime_generation: runtime.start_generation,
                        state: runtime.state,
                    });
                }
                (
                    binding,
                    runtime_id.to_owned(),
                    runtime.start_generation,
                    activation_id,
                )
            } else {
                let activation_id = state.allocate_activation_id();
                let generation = state
                    .deactivations
                    .get(runtime_id)
                    .map(|generation| generation + 1)
                    .unwrap_or_default();
                state.runtimes.insert(
                    runtime_id.to_owned(),
                    ActivatedRuntime {
                        extension_id: binding.extension_id.clone(),
                        binding: binding.clone(),
                        activations: BTreeSet::new(),
                        state: RuntimeActivationState::Starting,
                        session: None,
                        start_generation: generation,
                        factory_claimed: false,
                        restart_attempts: 0,
                        next_restart_at: None,
                    },
                );

                (binding, runtime_id.to_owned(), generation, activation_id)
            }
        };

        let start_lock = {
            let mut state = self.state.lock();
            Arc::clone(state.start_locks.entry(runtime_id.clone()).or_default())
        };
        let _start_guard = start_lock.lock().await;

        let starting = {
            let mut state = self.state.lock();
            let Some(runtime) = state.runtimes.get_mut(&runtime_id) else {
                return Err(ActivationError::RuntimeNotFound {
                    runtime_id: runtime_id.clone(),
                });
            };
            if runtime.start_generation != generation {
                return Err(ActivationError::RuntimeNotFound {
                    runtime_id: runtime_id.clone(),
                });
            }
            if runtime
                .session
                .as_ref()
                .is_some_and(|session| !session.is_closed())
            {
                runtime.activations.insert(activation_id);
                return Ok(ActivationHandle {
                    extension_id: binding.extension_id.clone(),
                    runtime_id: runtime_id.clone(),
                    activation_id,
                    runtime_generation: runtime.start_generation,
                    state: runtime.state,
                });
            }
            runtime.factory_claimed = true;
            StartingRuntime {
                binding,
                generation,
            }
        };
        let mut claim_guard = StartClaimGuard::new(
            Arc::clone(&self.state),
            runtime_id.clone(),
            starting.generation,
        );

        let context = SessionContext {
            binding: starting.binding.clone(),
            host_api: (self.host_api_factory)(starting.binding.clone(), starting.generation),
        };
        let session = (self.session_factory)(context).await.map_err(|error| {
            let mut state = self.state.lock();
            if let Some(runtime) = state.runtimes.get(&runtime_id) {
                if runtime.start_generation == starting.generation
                    && runtime.state == RuntimeActivationState::Starting
                    && runtime.session.is_none()
                {
                    state.runtimes.remove(&runtime_id);
                }
            }
            ActivationError::session_start(error)
        })?;

        let stale_session: Option<Arc<dyn ManagedRpcSession>> = {
            let mut state = self.state.lock();
            if let Some(runtime) = state.runtimes.get_mut(&runtime_id)
                && runtime.start_generation == starting.generation
            {
                runtime.activations.insert(activation_id);
                runtime.state = RuntimeActivationState::Active;
                if let Some(events) = &self.events {
                    events.mark_runtime_active(&runtime_id, starting.generation);
                }
                if let Some(jobs) = &self.jobs {
                    jobs.mark_runtime_active(&runtime_id, starting.generation);
                }
                runtime.session.replace(session)
            } else {
                Some(session)
            }
        };
        if let Some(session) = stale_session {
            session.shutdown().await;
            return Err(ActivationError::RuntimeNotFound {
                runtime_id: runtime_id.clone(),
            });
        }
        claim_guard.disarm();

        Ok(ActivationHandle {
            extension_id: starting.binding.extension_id.clone(),
            runtime_id,
            activation_id,
            runtime_generation: starting.generation,
            state: RuntimeActivationState::Active,
        })
    }

    /// Releases exactly the activation represented by `handle`.
    ///
    /// Stale handles are idempotent no-ops. In particular, a delayed GPUI view
    /// close cannot release a newer mount that reused the same runtime.
    pub async fn deactivate_activation(
        &self,
        handle: &ActivationHandle,
    ) -> Result<(), ActivationError> {
        let session = self.take_activation_session(
            &handle.extension_id,
            &handle.runtime_id,
            handle.activation_id,
        )?;
        if let Some(session) = session {
            session.shutdown().await;
        }
        Ok(())
    }

    fn take_activation_session(
        &self,
        extension_id: &str,
        runtime_id: &str,
        activation_id: u64,
    ) -> Result<Option<Arc<dyn ManagedRpcSession>>, ActivationError> {
        let mut state = self.state.lock();
        let Some(runtime) = state.runtimes.get_mut(runtime_id) else {
            return Ok(None);
        };
        if runtime.extension_id != extension_id || !runtime.activations.remove(&activation_id) {
            return Ok(None);
        }
        if !runtime.activations.is_empty() {
            return Ok(None);
        }

        let generation = runtime.start_generation;
        let session = runtime.session.take();
        state
            .deactivations
            .insert(runtime_id.to_owned(), generation);
        state.runtimes.remove(runtime_id);
        if let Some(blobs) = &self.blobs {
            blobs.remove_generation(runtime_id, generation);
        }
        if let Some(events) = &self.events {
            events.remove_runtime(runtime_id);
        }
        if let Some(jobs) = &self.jobs {
            self.cleanup_retired_jobs(jobs.remove_runtime(runtime_id));
        }
        Ok(session)
    }

    /// Returns a typed client for the current activated session generation.
    ///
    /// A client acquired here remains bound to that process generation. After
    /// a supervisor restart, compare `generation` and acquire a new client.
    pub fn universal_plugin_client(
        &self,
        runtime_id: &str,
    ) -> Result<ManagedUniversalPluginClient, ActivationError> {
        let (binding, generation, session) =
            {
                let state = self.state.lock();
                let runtime = state.runtimes.get(runtime_id).ok_or_else(|| {
                    ActivationError::RuntimeNotFound {
                        runtime_id: runtime_id.to_owned(),
                    }
                })?;
                (
                    runtime.binding.clone(),
                    runtime.start_generation,
                    runtime
                        .session
                        .clone()
                        .ok_or_else(|| ActivationError::RuntimeNotReady {
                            runtime_id: runtime_id.to_owned(),
                        })?,
                )
            };
        let authorizer = Arc::new(
            ResourceOpenAuthorizer::new(binding.permissions.iter().cloned()).into_host_authorizer(),
        );
        if session.is_closed() {
            return Err(ActivationError::RuntimeNotReady {
                runtime_id: runtime_id.to_owned(),
            });
        }
        let client = session
            .universal_plugin_client(Some(authorizer))
            .ok_or_else(|| ActivationError::RuntimeNotReady {
                runtime_id: runtime_id.to_owned(),
            })?;

        let mut managed_client = ManagedUniversalPluginClient {
            runtime_id: runtime_id.to_owned(),
            generation,
            extension_id: binding.extension_id.clone(),
            client,
            blobs: None,
            events: None,
            jobs: None,
        };
        if let Some(blobs) = &self.blobs {
            managed_client = managed_client.with_blob_store(blobs.clone());
        }
        if let Some(events) = &self.events {
            managed_client = managed_client.with_event_activation(events.clone());
        }
        if let Some(jobs) = &self.jobs {
            managed_client = managed_client.with_job_activation(jobs.clone());
        }
        Ok(managed_client)
    }

    /// Invokes a provider resource and caches large inline JSON in the host store.
    ///
    /// Small results stay inline. Provider-owned `ResultRef::Blob` and
    /// `ResultRef::EventStream` values are passed through unchanged: this host
    /// store only owns bytes that have already crossed the IPC boundary.
    pub async fn invoke_resource_and_cache_blob(
        &self,
        runtime_id: &str,
        params: &ResourceInvokeParams,
    ) -> Result<ResourceInvokeResult, ActivationError> {
        let managed_client = self.universal_plugin_client(runtime_id)?;
        let result = managed_client
            .client
            .invoke_resource(params)
            .await
            .map_err(|error| ActivationError::SessionStart(error.to_string()))?;
        let result = self
            .cache_inline_result(
                runtime_id,
                managed_client.generation,
                result,
                "resource_invoke",
                &params.method,
            )
            .await?;
        Ok(result)
    }

    /// Reads a completed job result and caches large inline JSON in the host store.
    pub async fn job_result_and_cache_blob(
        &self,
        runtime_id: &str,
        params: &JobResultParams,
    ) -> Result<JobResultResult, ActivationError> {
        let managed_client = self.universal_plugin_client(runtime_id)?;
        let handle = managed_client
            .jobs
            .as_ref()
            .ok_or_else(|| ActivationError::SessionStart("no job activation manager".into()))?
            .handle(
                &managed_client.extension_id,
                runtime_id,
                managed_client.generation,
                &params.job_id,
            )
            .map_err(ActivationError::session_start)?;
        let result = managed_client
            .job_result(&handle)
            .await
            .map_err(|error| ActivationError::SessionStart(error.to_string()))?;
        let cached = self
            .cache_inline_result(
                runtime_id,
                managed_client.generation,
                result,
                "job_result",
                &params.job_id,
            )
            .await?;
        if let ResultRef::Blob { id } = &cached.result
            && id.starts_with("host-blob-")
        {
            managed_client
                .jobs
                .as_ref()
                .expect("job manager checked above")
                .attach_blob(&handle, id)
                .map_err(ActivationError::session_start)?;
        }
        Ok(cached)
    }

    fn cleanup_retired_jobs(&self, retired: Vec<RetiredJob>) {
        let Some(blobs) = &self.blobs else {
            return;
        };
        for job in retired {
            let owner = crate::BlobOwner {
                runtime_id: job.handle.runtime_id,
                generation: job.handle.generation,
            };
            for blob_id in job.blob_ids {
                let _ = blobs.remove_owned_blob(&owner, &blob_id);
            }
        }
    }

    async fn cache_inline_result<T>(
        &self,
        runtime_id: &str,
        generation: u64,
        mut result: T,
        source: &str,
        source_id: &str,
    ) -> Result<T, ActivationError>
    where
        T: CachedResult,
    {
        let Some(value) = result.inline_value() else {
            return Ok(result);
        };
        let data = serde_json::to_vec(value)
            .map_err(|error| ActivationError::HostBlob(error.to_string()))?;
        if data.len() as u64 <= INLINE_BLOB_THRESHOLD_BYTES {
            return Ok(result);
        }

        // Do not create a host cache entry if the provider was replaced while
        // its response was in flight. The next generation gets a new owner and
        // must not make an old response visible as its own result.
        self.ensure_runtime_generation(runtime_id, generation)?;
        let blobs = self.blobs.as_ref().ok_or_else(|| {
            ActivationError::HostBlob("the activation manager has no host blob store".into())
        })?;
        let opened = blobs
            .open(
                &crate::BlobOwner {
                    runtime_id: runtime_id.to_owned(),
                    generation,
                },
                &BlobOpenParams {
                    conn_id: None,
                    content_type: Some("application/json".into()),
                    metadata: Some(serde_json::json!({
                        "source": source,
                        "source_id": source_id,
                    })),
                },
                data,
            )
            .map_err(|error| ActivationError::HostBlob(error.to_string()))?;
        result.replace_with_blob(opened.blob_id);
        Ok(result)
    }

    fn ensure_runtime_generation(
        &self,
        runtime_id: &str,
        generation: u64,
    ) -> Result<(), ActivationError> {
        let state = self.state.lock();
        let runtime =
            state
                .runtimes
                .get(runtime_id)
                .ok_or_else(|| ActivationError::RuntimeNotFound {
                    runtime_id: runtime_id.to_owned(),
                })?;
        if runtime.start_generation == generation && runtime.session.is_some() {
            Ok(())
        } else {
            Err(ActivationError::RuntimeNotReady {
                runtime_id: runtime_id.to_owned(),
            })
        }
    }

    pub async fn deactivate_runtime(&self, runtime_id: &str) -> Result<(), ActivationError> {
        let start_lock = {
            let mut state = self.state.lock();
            Arc::clone(state.start_locks.entry(runtime_id.to_owned()).or_default())
        };
        let _start_guard = start_lock.lock().await;
        let runtime = {
            let mut state = self.state.lock();
            let runtime = state.runtimes.remove(runtime_id);
            if let Some(runtime) = &runtime {
                state
                    .deactivations
                    .insert(runtime_id.to_owned(), runtime.start_generation);
                if let Some(blobs) = &self.blobs {
                    blobs.remove_generation(runtime_id, runtime.start_generation);
                }
                if let Some(events) = &self.events {
                    events.remove_runtime(runtime_id);
                }
                self.cleanup_retired_jobs(
                    self.jobs
                        .as_ref()
                        .map(|jobs| jobs.remove_runtime(runtime_id))
                        .unwrap_or_default(),
                );
            }
            runtime
        };
        if let Some(runtime) = runtime {
            if let Some(session) = runtime.session {
                session.shutdown().await;
            }
        }
        Ok(())
    }

    pub async fn deactivate_extension(&self, extension_id: &str) -> Result<(), ActivationError> {
        let runtime_ids = self
            .state
            .lock()
            .runtimes
            .iter()
            .filter(|(_, runtime)| runtime.extension_id == extension_id)
            .map(|(runtime_id, _)| runtime_id.clone())
            .collect::<Vec<_>>();
        for runtime_id in runtime_ids {
            self.deactivate_runtime(&runtime_id).await?;
        }
        Ok(())
    }

    pub fn runtime_state(
        &self,
        runtime_id: &str,
    ) -> Result<RuntimeActivationState, ActivationError> {
        self.state
            .lock()
            .runtimes
            .get(runtime_id)
            .map(|runtime| runtime.state)
            .ok_or(ActivationError::RuntimeNotFound {
                runtime_id: runtime_id.to_owned(),
            })
    }

    /// Returns the current process generation for restart-aware client use.
    pub fn runtime_generation(&self, runtime_id: &str) -> Result<u64, ActivationError> {
        self.state
            .lock()
            .runtimes
            .get(runtime_id)
            .map(|runtime| runtime.start_generation)
            .ok_or_else(|| ActivationError::RuntimeNotFound {
                runtime_id: runtime_id.to_owned(),
            })
    }

    /// Inspect process health without changing process state.
    pub async fn runtime_health(&self, runtime_id: &str) -> Result<RuntimeHealth, ActivationError> {
        let session = self.active_session(runtime_id);
        let ping_error = match &session {
            Some(session) if !session.is_closed() => {
                session.ping().await.err().map(|error| error.to_string())
            }
            _ => None,
        };
        self.health_snapshot(runtime_id, ping_error).await
    }

    /// Check a runtime and restart it when process closure is confirmed.
    ///
    /// This method is deterministic and pull-oriented: the host UI or process
    /// monitor decides when to poll. Elapsed backoff is observed, but this
    /// method never sleeps before attempting a restart.
    pub async fn check_runtime(&self, runtime_id: &str) -> Result<RuntimeHealth, ActivationError> {
        let health = self.runtime_health(runtime_id).await?;
        if !health.session_closed {
            return Ok(health);
        }

        let start_lock = {
            let mut state = self.state.lock();
            Arc::clone(state.start_locks.entry(runtime_id.to_owned()).or_default())
        };
        let _start_guard = start_lock.lock().await;

        let decision: Result<CheckDecision, ActivationError> = {
            let mut state = self.state.lock();
            let runtime = state.runtimes.get_mut(runtime_id).ok_or_else(|| {
                ActivationError::RuntimeNotFound {
                    runtime_id: runtime_id.to_owned(),
                }
            })?;
            let binding = runtime.binding.clone();
            let budget = binding.max_restart_attempts;

            if !binding.auto_restart {
                runtime.state = RuntimeActivationState::Failed;
                let stale_session = runtime.session.take();
                let health = self.health_locked(&mut state, runtime_id);
                Ok(CheckDecision::Return(health, stale_session))
            } else if runtime.restart_attempts >= budget {
                runtime.state = RuntimeActivationState::CrashLoop;
                let stale_session = runtime.session.take();
                let health = self.health_locked(&mut state, runtime_id);
                Ok(CheckDecision::Return(health, stale_session))
            } else if let Some(restart_at) = runtime.next_restart_at
                && restart_at > Instant::now()
            {
                runtime.state = RuntimeActivationState::Restarting;
                Ok(CheckDecision::Return(
                    self.health_locked(&mut state, runtime_id),
                    None,
                ))
            } else {
                runtime.restart_attempts += 1;
                runtime.state = RuntimeActivationState::Restarting;
                runtime.factory_claimed = true;
                Ok(CheckDecision::Restart {
                    binding: Box::new(binding),
                    generation: runtime.start_generation,
                    attempt: runtime.restart_attempts,
                })
            }
        };

        match decision? {
            CheckDecision::Return(health, stale_session) => {
                if let Some(session) = stale_session {
                    session.shutdown().await;
                }
                return Ok(health);
            }
            CheckDecision::Restart {
                binding,
                generation,
                attempt,
            } => {
                let binding = *binding;
                let stale_session = self
                    .restart_runtime(runtime_id, binding, generation, attempt)
                    .await;
                if let Some(session) = stale_session {
                    session.shutdown().await;
                }
                self.health_snapshot(runtime_id, None).await
            }
        }
    }

    async fn restart_runtime(
        &self,
        runtime_id: &str,
        binding: RegisteredIpcRuntimeBinding,
        generation: u64,
        attempt: u32,
    ) -> Option<Arc<dyn ManagedRpcSession>> {
        let context = SessionContext {
            binding: binding.clone(),
            host_api: (self.host_api_factory)(binding.clone(), generation + 1),
        };
        let session = (self.session_factory)(context).await.ok();

        let mut stale_session = None;
        {
            let mut state = self.state.lock();
            if let Some(runtime) = state.runtimes.get_mut(runtime_id)
                && runtime.start_generation == generation
                && runtime.restart_attempts == attempt
            {
                let backoff = self.supervision_policy.backoff_for_attempt(attempt);
                runtime.next_restart_at = Some(Instant::now() + backoff);
                match session {
                    Some(session) => {
                        runtime.state = RuntimeActivationState::Active;
                        runtime.start_generation += 1;
                        runtime.factory_claimed = false;
                        stale_session = runtime.session.replace(session);
                        if let Some(blobs) = &self.blobs {
                            blobs.remove_generation(runtime_id, generation);
                        }
                        if let Some(events) = &self.events {
                            events.retire_generation(runtime_id, generation);
                            events.mark_runtime_active(runtime_id, generation + 1);
                        }
                        if let Some(jobs) = &self.jobs {
                            self.cleanup_retired_jobs(
                                jobs.retire_generation(runtime_id, generation),
                            );
                            jobs.mark_runtime_active(runtime_id, generation + 1);
                        }
                    }
                    None => {
                        runtime.state = if attempt >= binding.max_restart_attempts {
                            RuntimeActivationState::CrashLoop
                        } else {
                            RuntimeActivationState::Restarting
                        };
                        runtime.factory_claimed = false;
                    }
                }
            } else if let Some(session) = session {
                stale_session = Some(session);
            }
        }
        stale_session
    }

    async fn health_snapshot(
        &self,
        runtime_id: &str,
        ping_error: Option<String>,
    ) -> Result<RuntimeHealth, ActivationError> {
        let session = self.active_session(runtime_id);
        let mut state = self.state.lock();
        let runtime =
            state
                .runtimes
                .get_mut(runtime_id)
                .ok_or_else(|| ActivationError::RuntimeNotFound {
                    runtime_id: runtime_id.to_owned(),
                })?;
        let session_closed = runtime
            .session
            .as_ref()
            .is_some_and(|session| session.is_closed())
            || (session.is_none() && runtime.state != RuntimeActivationState::Starting);
        let ping_error = if session_closed { None } else { ping_error };
        match (session_closed, ping_error.as_ref()) {
            (false, None) => {
                if runtime.state == RuntimeActivationState::Degraded {
                    runtime.state = RuntimeActivationState::Active;
                }
            }
            (false, Some(_)) => runtime.state = RuntimeActivationState::Degraded,
            (true, _) => {
                if runtime.state != RuntimeActivationState::Restarting
                    && runtime.state != RuntimeActivationState::Failed
                    && runtime.state != RuntimeActivationState::CrashLoop
                {
                    runtime.state = RuntimeActivationState::Failed;
                }
            }
        }
        Ok(self.health_locked_with_ping(&mut state, runtime_id, ping_error))
    }

    fn health_locked(&self, state: &mut ActivationState, runtime_id: &str) -> RuntimeHealth {
        self.health_locked_with_ping(state, runtime_id, None)
    }

    fn health_locked_with_ping(
        &self,
        state: &mut ActivationState,
        runtime_id: &str,
        ping_error: Option<String>,
    ) -> RuntimeHealth {
        let runtime = state
            .runtimes
            .get(runtime_id)
            .expect("caller checked runtime");
        let session_closed = runtime
            .session
            .as_ref()
            .is_some_and(|session| session.is_closed());
        let restart_backoff_remaining = runtime
            .next_restart_at
            .filter(|restart_at| *restart_at > Instant::now())
            .map(|restart_at| restart_at - Instant::now());
        RuntimeHealth {
            state: runtime.state,
            session_closed: session_closed || runtime.session.is_none(),
            ping_error,
            restart_attempts: runtime.restart_attempts,
            restart_budget: runtime.binding.max_restart_attempts,
            restart_backoff_remaining,
        }
    }

    fn active_session(&self, runtime_id: &str) -> Option<Arc<dyn ManagedRpcSession>> {
        self.state
            .lock()
            .runtimes
            .get(runtime_id)
            .and_then(|runtime| runtime.session.clone())
    }

    #[cfg(test)]
    pub(super) fn clear_restart_backoff_for_test(&self, runtime_id: &str) {
        if let Some(runtime) = self.state.lock().runtimes.get_mut(runtime_id) {
            runtime.next_restart_at = None;
        }
    }
}

/// Configuration for the independent host-side runtime monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeMonitorConfig {
    pub check_interval: Duration,
}

impl Default for RuntimeMonitorConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(5),
        }
    }
}

/// A state snapshot or removal notification emitted by [`RuntimeMonitor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeMonitorEvent {
    HealthChanged {
        runtime_id: String,
        health: RuntimeHealth,
    },
    RuntimeRemoved {
        runtime_id: String,
    },
    CheckFailed {
        runtime_id: String,
        error: ActivationError,
    },
}

/// Error returned when a monitor operation cannot be started.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuntimeMonitorError {
    #[error("runtime monitor is already running")]
    AlreadyRunning,
}

/// Periodically invokes the pull-oriented activation supervisor.
///
/// The monitor deliberately owns no process state. It only schedules checks,
/// serializes checks per runtime, collapses repeated snapshots, and publishes
/// state transitions to consumers such as a GPUI activation status entity.
impl fmt::Debug for RuntimeMonitor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeMonitor")
            .field("manager_catalog", &*self.manager.catalog.read())
            .field("config", &self.config)
            .field("tracked_runtimes", &self.tracked_runtimes())
            .finish_non_exhaustive()
    }
}

pub struct RuntimeMonitor {
    manager: Arc<ActivationManager>,
    config: RuntimeMonitorConfig,
    monitor_state: Arc<SyncMutex<BTreeMap<String, Option<RuntimeHealth>>>>,
    running: SyncMutex<Option<mpsc::Sender<()>>>,
    stopped: Arc<Notify>,
    events: broadcast::Sender<RuntimeMonitorEvent>,
}

impl RuntimeMonitor {
    pub fn new(manager: Arc<ActivationManager>, config: RuntimeMonitorConfig) -> Self {
        let (events, _) = broadcast::channel(128);
        Self {
            manager,
            config,
            monitor_state: Arc::new(SyncMutex::new(BTreeMap::new())),
            running: SyncMutex::new(None),
            stopped: Arc::new(Notify::new()),
            events,
        }
    }

    /// Subscribes to health state transitions and runtime removal events.
    ///
    /// A receiver is returned even while the monitor is stopped; new
    /// subscriptions remain connected until their receiver is dropped.
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeMonitorEvent> {
        self.events.subscribe()
    }

    /// Tracks a runtime without performing an immediate health check.
    ///
    /// Tracking is intentionally separate from activation. A tracked runtime
    /// that has never been observed does not produce a removal event; once a
    /// health snapshot has been observed, disappearance from the manager emits
    /// `RuntimeRemoved` on the next cycle.
    pub fn track(&self, runtime_id: impl Into<String>) {
        self.monitor_state
            .lock()
            .entry(runtime_id.into())
            .or_insert(None);
    }

    /// Stops tracking a runtime. This does not deactivate its provider.
    pub fn untrack(&self, runtime_id: &str) {
        self.monitor_state.lock().remove(runtime_id);
    }

    /// Returns the currently tracked runtime IDs in deterministic order.
    pub fn tracked_runtimes(&self) -> BTreeSet<String> {
        self.monitor_state.lock().keys().cloned().collect()
    }

    /// Returns the last observed health snapshot for a tracked runtime.
    pub fn runtime_health(&self, runtime_id: &str) -> Option<RuntimeHealth> {
        self.monitor_state.lock().get(runtime_id).cloned().flatten()
    }

    /// Returns all last-observed health snapshots in deterministic order.
    pub fn runtime_healths(&self) -> BTreeMap<String, RuntimeHealth> {
        self.monitor_state
            .lock()
            .iter()
            .filter_map(|(runtime_id, health)| {
                health.clone().map(|health| (runtime_id.clone(), health))
            })
            .collect()
    }

    /// Runs the periodic monitor inline.
    ///
    /// This method is useful for deterministic tests and host runtimes that
    /// already own a scheduler. It performs at most one supervision cycle.
    pub async fn run_once(&self) {
        let runtime_ids: Vec<String> = self.monitor_state.lock().keys().cloned().collect();
        for runtime_id in runtime_ids {
            if let Some(event) = self.check_once(&runtime_id).await {
                let _ = self.events.send(event);
            }
        }
    }

    /// Starts one monitor task if no task is currently running.
    pub fn start(&self) -> Result<(), RuntimeMonitorError> {
        let mut running = self.running.lock();
        if running.is_some() {
            return Err(RuntimeMonitorError::AlreadyRunning);
        }
        let (stop_tx, mut stop_rx) = mpsc::channel(1);
        running.replace(stop_tx);
        drop(running);

        let manager = Arc::clone(&self.manager);
        let monitor_state = Arc::clone(&self.monitor_state);
        let events = self.events.clone();
        let stopped = Arc::clone(&self.stopped);
        let check_interval = self.config.check_interval;

        tokio::spawn(async move {
            loop {
                if monitor_should_stop(&mut stop_rx) {
                    break;
                }

                let runtime_ids: Vec<String> = monitor_state.lock().keys().cloned().collect();
                for runtime_id in runtime_ids {
                    // Do not cancel an in-flight supervision check. The manager
                    // and production ping requests are bounded, while monitor
                    // checks are serialized here to prevent overlap per runtime.
                    if let Some(event) =
                        check_runtime_event(&manager, &monitor_state, &runtime_id).await
                    {
                        let _ = events.send(event);
                    }
                }

                tokio::select! {
                    _ = stop_rx.recv() => break,
                    _ = sleep(check_interval) => {}
                }
            }

            stopped.notify_one();
        });

        Ok(())
    }

    /// Stops the monitor task and waits for it to acknowledge shutdown.
    pub async fn stop(&self) {
        let stop = self.running.lock().take();
        if let Some(stop) = stop {
            let _ = stop.send(()).await;
            self.stopped.notified().await;
        }
    }

    async fn check_once(&self, runtime_id: &str) -> Option<RuntimeMonitorEvent> {
        check_runtime_event(&self.manager, &self.monitor_state, runtime_id).await
    }
}

async fn check_runtime_event(
    manager: &ActivationManager,
    monitor_state: &SyncMutex<BTreeMap<String, Option<RuntimeHealth>>>,
    runtime_id: &str,
) -> Option<RuntimeMonitorEvent> {
    match manager.check_runtime(runtime_id).await {
        Ok(health) => {
            let mut state = monitor_state.lock();
            let last_health = state.entry(runtime_id.to_owned()).or_default();
            if last_health.as_ref() == Some(&health) {
                return None;
            }
            last_health.replace(health.clone());
            drop(state);
            Some(RuntimeMonitorEvent::HealthChanged {
                runtime_id: runtime_id.to_owned(),
                health,
            })
        }
        Err(ActivationError::RuntimeNotFound { .. }) => {
            let mut state = monitor_state.lock();
            let last_health = state.get_mut(runtime_id)?;
            last_health.take()?;
            drop(state);
            Some(RuntimeMonitorEvent::RuntimeRemoved {
                runtime_id: runtime_id.to_owned(),
            })
        }
        Err(error) => Some(RuntimeMonitorEvent::CheckFailed {
            runtime_id: runtime_id.to_owned(),
            error,
        }),
    }
}

fn monitor_should_stop(stop_rx: &mut mpsc::Receiver<()>) -> bool {
    match stop_rx.try_recv() {
        Ok(()) | Err(mpsc::error::TryRecvError::Disconnected) => true,
        Err(mpsc::error::TryRecvError::Empty) => false,
    }
}
