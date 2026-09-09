use std::sync::Arc;

use extension_protocol::resource::{ResourceCloseParams, ResourceOpenResult};
use tokio::sync::Mutex;

use crate::{ManagedUniversalPluginClient, PluginAdapterError};

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
        let resource = self.inner.resource.lock().await.take();
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

    /// 主资源 open 结果中的能力集快照。
    pub fn capabilities(&self) -> &[String] {
        self.inner
            .capabilities
            .get()
            .map(|capabilities| capabilities.as_slice())
            .unwrap_or_default()
    }

    pub fn scope(&self, page_id: impl Into<String>, mount_id: u64) -> ResourceScope {
        ResourceScope {
            session: self.clone(),
            page_id: page_id.into(),
            mount_id,
        }
    }

    pub async fn resource_id(&self) -> Result<String, PluginAdapterError> {
        self.inner
            .resource
            .lock()
            .await
            .as_ref()
            .map(|resource| resource.resource_id.clone())
            .ok_or(PluginAdapterError::SessionClosed)
    }
}

impl ResourceScope {
    pub fn page_id(&self) -> &str {
        &self.page_id
    }

    pub fn mount_id(&self) -> u64 {
        self.mount_id
    }

    pub fn session(&self) -> ResourceSessionHandle {
        self.session.clone()
    }
}
