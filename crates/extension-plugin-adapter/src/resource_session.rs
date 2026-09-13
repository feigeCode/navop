use std::sync::Arc;

use extension_host::CancellationToken;
use extension_protocol::{
    event_stream::{EventOpenParams, EventOpenResult},
    resource::{ResourceCloseParams, ResourceOpenResult},
};
use std::sync::Mutex;

use crate::{
    EventStreamSubscription, EventStreamSubscriptionConfig, JobActivationHandle, JobSnapshot,
    ManagedUniversalPluginClient, PluginAdapterError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSessionIdentity {
    pub extension_id: String,
    pub runtime_id: String,
    pub runtime_generation: u64,
    pub session_epoch: u64,
}

#[derive(Clone)]
pub struct ResourceSessionHandle {
    inner: Arc<ResourceSessionInner>,
}

pub struct ResourceSessionOwner {
    inner: Arc<ResourceSessionInner>,
}

pub struct ResourceScope {
    session: ResourceSessionHandle,
    page_id: String,
    mount_id: u64,
    cancellation: CancellationToken,
}

struct ResourceSessionInner {
    identity: ResourceSessionIdentity,
    client: ManagedUniversalPluginClient,
    capabilities: std::sync::OnceLock<Vec<String>>,
    resource: Mutex<Option<ResourceOpenResult>>,
}

impl ResourceSessionOwner {
    pub fn new(
        identity: ResourceSessionIdentity,
        client: ManagedUniversalPluginClient,
        resource: ResourceOpenResult,
    ) -> Self {
        let capabilities = std::sync::OnceLock::new();
        let _ = capabilities.set(resource.capabilities.clone());
        Self {
            inner: Arc::new(ResourceSessionInner {
                identity,
                client,
                capabilities,
                resource: Mutex::new(Some(resource)),
            }),
        }
    }

    pub fn handle(&self) -> ResourceSessionHandle {
        ResourceSessionHandle {
            inner: self.inner.clone(),
        }
    }

    pub async fn close(&self) -> Result<(), PluginAdapterError> {
        let resource = self
            .inner
            .resource
            .lock()
            .expect("resource session lock poisoned")
            .take();
        let Some(resource) = resource else {
            return Ok(());
        };
        self.inner
            .client
            .client()
            .close_resource(&ResourceCloseParams {
                resource_id: resource.resource_id,
            })
            .await
            .map_err(|error| PluginAdapterError::Session(error.to_string()))
    }
}

impl ResourceSessionHandle {
    pub fn identity(&self) -> &ResourceSessionIdentity {
        &self.inner.identity
    }

    pub fn generation(&self) -> u64 {
        self.inner.identity.runtime_generation
    }

    /// 受限访问:页面只能通过 dispatcher 走命名操作,不直接持有裸 client。
    pub(crate) fn client(&self) -> &ManagedUniversalPluginClient {
        &self.inner.client
    }

    pub fn resource_snapshot(&self) -> Result<ResourceOpenResult, PluginAdapterError> {
        self.inner
            .resource
            .lock()
            .expect("resource session lock poisoned")
            .clone()
            .ok_or(PluginAdapterError::SessionClosed)
    }

    pub fn managed_client(&self) -> &ManagedUniversalPluginClient {
        &self.inner.client
    }

    pub fn task_snapshots(&self) -> Vec<JobSnapshot> {
        let Ok(resource) = self.resource_snapshot() else {
            return Vec::new();
        };
        self.inner
            .client
            .job_activation()
            .map(|jobs| {
                jobs.snapshots_for_resource(
                    &self.inner.identity.extension_id,
                    &self.inner.identity.runtime_id,
                    self.inner.identity.runtime_generation,
                    &resource.resource_id,
                )
            })
            .unwrap_or_default()
    }

    pub async fn cancel_task(&self, job_id: &str) -> Result<(), PluginAdapterError> {
        let handle = JobActivationHandle {
            extension_id: self.inner.identity.extension_id.clone(),
            runtime_id: self.inner.identity.runtime_id.clone(),
            generation: self.inner.identity.runtime_generation,
            job_id: job_id.to_string(),
        };
        let resource_id = self.resource_id().await?;
        if !self
            .inner
            .client
            .job_activation()
            .is_some_and(|jobs| jobs.is_owned_by_resource(&handle, &resource_id))
        {
            return Err(PluginAdapterError::Session(
                "job is not owned by this resource session".into(),
            ));
        }
        self.inner
            .client
            .cancel_job(&handle)
            .await
            .map_err(|error| PluginAdapterError::Session(error.to_string()))?;
        self.inner
            .client
            .close_job(&handle)
            .await
            .map_err(|error| PluginAdapterError::Session(error.to_string()))
    }

    /// 主资源 open 结果中的能力集快照。
    pub fn capabilities(&self) -> &[String] {
        self.inner
            .capabilities
            .get()
            .map(|capabilities| capabilities.as_slice())
            .unwrap_or_default()
    }

    /// 主资源 open 结果的 metadata（provider 声明的连接目标摘要等）。
    /// 终端等宿主组件据此把页面操作绑定回同一连接目标。
    pub fn metadata(&self) -> Option<serde_json::Value> {
        self.inner
            .resource
            .lock()
            .expect("resource session lock poisoned")
            .as_ref()
            .and_then(|resource| resource.metadata.clone())
    }

    pub fn scope(&self, page_id: impl Into<String>, mount_id: u64) -> ResourceScope {
        ResourceScope {
            session: self.clone(),
            page_id: page_id.into(),
            mount_id,
            cancellation: CancellationToken::new(),
        }
    }

    pub async fn resource_id(&self) -> Result<String, PluginAdapterError> {
        self.inner
            .resource
            .lock()
            .expect("resource session lock poisoned")
            .as_ref()
            .map(|resource| resource.resource_id.clone())
            .ok_or(PluginAdapterError::SessionClosed)
    }
}

impl ResourceScope {
    pub async fn open_event_stream(
        &self,
        kind: impl Into<String>,
        capacity: Option<u32>,
    ) -> Result<EventOpenResult, PluginAdapterError> {
        self.session
            .managed_client()
            .open_event_stream(&EventOpenParams {
                conn_id: None,
                kind: kind.into(),
                capacity,
            })
            .await
            .map_err(|error| PluginAdapterError::Session(error.to_string()))
    }

    pub fn page_id(&self) -> &str {
        &self.page_id
    }

    pub fn mount_id(&self) -> u64 {
        self.mount_id
    }

    pub fn session(&self) -> ResourceSessionHandle {
        self.session.clone()
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Starts a pull subscription owned by this page scope. Dropping or
    /// cancelling the scope stops the supervisor and closes the provider stream.
    pub fn subscribe_events(
        &self,
        stream: &EventOpenResult,
        config: EventStreamSubscriptionConfig,
    ) -> EventStreamSubscription {
        EventStreamSubscription::spawn_with_cancel(
            self.session.managed_client().clone(),
            stream.stream_id.clone(),
            config,
            self.cancellation(),
        )
    }
}

impl Drop for ResourceScope {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}
