use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
};

use extension_host::CancellationToken;
use extension_plugin_adapter::{JobActivationHandle, ManagedUniversalPluginClient};
use extension_protocol::{
    blob::BlobCloseParams,
    event_stream::EventCloseParams,
    resource::{ResourceCloseParams, ResourceOpenResult},
    result_ref::ResultRef,
};
use gpui_shell::{HostError, HostObject, HostValue};

use super::value::json_to_host;
use crate::universal_plugins::UniversalPluginService;

use super::error::{ErrorCode, navop_error, service_error};

#[derive(Clone)]
struct ProviderHandle {
    alias: String,
    runtime_id: String,
    generation: u64,
    provider_id: String,
    owned: bool,
    /// 派生自哪个 resource 句柄;resource 关闭时只回收属于它的子句柄。
    resource: Option<String>,
}

#[derive(Clone)]
struct JobHandle {
    alias: String,
    resource: String,
    provider: JobActivationHandle,
}

pub(crate) struct ShellMountSession {
    service: UniversalPluginService,
    backends: BTreeMap<String, String>,
    resources: Mutex<HashMap<String, ProviderHandle>>,
    blobs: Mutex<HashMap<String, ProviderHandle>>,
    events: Mutex<HashMap<String, ProviderHandle>>,
    jobs: Mutex<HashMap<String, JobHandle>>,
    root: CancellationToken,
    pub(super) tokio: tokio::runtime::Handle,
}

impl ShellMountSession {
    pub(crate) fn new(
        service: UniversalPluginService,
        backends: BTreeMap<String, String>,
        tokio: tokio::runtime::Handle,
    ) -> Self {
        Self {
            service,
            backends,
            resources: Mutex::new(HashMap::new()),
            blobs: Mutex::new(HashMap::new()),
            events: Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
            root: CancellationToken::new(),
            tokio,
        }
    }

    /// 每次 HostModule 调用派生的子令牌;mount 关闭时随根令牌一起取消。
    pub(super) fn call_token(&self) -> CancellationToken {
        self.root.child()
    }

    /// 取消所有仍在飞行中的 HostModule 调用。
    pub(crate) fn cancel(&self) {
        self.root.cancel();
    }

    pub(super) fn client(&self, alias: &str) -> Result<ManagedUniversalPluginClient, HostError> {
        let runtime_id = self.backends.get(alias).ok_or_else(|| {
            navop_error(
                ErrorCode::BackendNotFound,
                format!("unknown backend alias `{alias}`"),
            )
        })?;
        self.service
            .universal_plugin_client(runtime_id)
            .map_err(service_error)
    }

    pub(super) fn register_resource(
        &self,
        alias: String,
        client: &ManagedUniversalPluginClient,
        result: ResourceOpenResult,
    ) -> Result<HostValue, HostError> {
        self.register_resource_with_ownership(alias, client, result, true)
    }

    /// Registers a primary resource borrowed from the connection tab.
    /// Page cleanup must never close this resource.
    pub(crate) fn register_borrowed_resource(
        &self,
        alias: String,
        client: &ManagedUniversalPluginClient,
        result: ResourceOpenResult,
    ) -> Result<HostValue, HostError> {
        self.register_resource_with_ownership(alias, client, result, false)
    }

    fn register_resource_with_ownership(
        &self,
        alias: String,
        client: &ManagedUniversalPluginClient,
        result: ResourceOpenResult,
        owned: bool,
    ) -> Result<HostValue, HostError> {
        let handle = new_handle("resource");
        self.resources
            .lock()
            .expect("shell resource registry poisoned")
            .insert(
                handle.clone(),
                ProviderHandle {
                    alias,
                    runtime_id: client.runtime_id.clone(),
                    generation: client.generation,
                    provider_id: result.resource_id,
                    owned,
                    resource: None,
                },
            );
        Ok(HostObject::new()
            .field("handle", handle)
            .field("capabilities", result.capabilities)
            .field(
                "metadata",
                result.metadata.as_ref().map(json_to_host).transpose()?,
            )
            .into())
    }

    pub(super) fn resource(
        &self,
        handle: &str,
    ) -> Result<(String, ManagedUniversalPluginClient, String), HostError> {
        let record = self
            .resources
            .lock()
            .map_err(|_| {
                navop_error(ErrorCode::RuntimeUnavailable, "shell resource registry poisoned")
            })?
            .get(handle)
            .cloned()
            .ok_or_else(|| invalid_handle("resource", handle))?;
        let client = self.checked_client(&record)?;
        Ok((record.alias, client, record.provider_id))
    }

    pub(super) fn close_resource_record(&self, handle: &str, provider_id: &str) {
        remove_matching(&self.resources, handle, provider_id);
    }

    /// 取出派生自该 resource 的 blob/event/job 句柄;其他 resource 的句柄不受影响。
    pub(super) fn take_resource_children(&self, handle: &str) -> ScopedHandles {
        ScopedHandles {
            blobs: drain_scoped(&self.blobs, handle),
            events: drain_scoped(&self.events, handle),
            jobs: self
                .jobs
                .lock()
                .map(|mut jobs| {
                    let (scoped, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *jobs)
                        .into_iter()
                        .partition(|(_, job)| job.resource == handle);
                    jobs.extend(rest);
                    scoped.into_iter().map(|(_, job)| job).collect()
                })
                .unwrap_or_default(),
        }
    }

    pub(super) fn service(&self) -> UniversalPluginService {
        self.service.clone()
    }

    pub(super) fn result_ref(
        &self,
        alias: String,
        resource: &str,
        generation: u64,
        result: &ResultRef,
    ) -> Result<HostValue, HostError> {
        match result {
            ResultRef::Inline { value } => Ok(HostObject::new()
                .field("kind", "inline")
                .field("value", json_to_host(value)?)
                .into()),
            ResultRef::Blob { id } => {
                let handle = self.insert_provider_handle(
                    &self.blobs,
                    "blob",
                    alias,
                    resource,
                    generation,
                    id,
                )?;
                Ok(HostObject::new()
                    .field("kind", "blob")
                    .field("handle", handle)
                    .into())
            }
            ResultRef::EventStream { id } => {
                let handle = self.insert_provider_handle(
                    &self.events,
                    "event",
                    alias,
                    resource,
                    generation,
                    id,
                )?;
                Ok(HostObject::new()
                    .field("kind", "event_stream")
                    .field("handle", handle)
                    .into())
            }
        }
    }

    pub(super) fn blob(
        &self,
        handle: &str,
    ) -> Result<(ManagedUniversalPluginClient, String), HostError> {
        self.provider_handle(&self.blobs, "blob", handle)
    }

    pub(super) fn close_blob_record(&self, handle: &str, provider_id: &str) {
        remove_matching(&self.blobs, handle, provider_id);
    }

    pub(super) fn register_job(
        &self,
        alias: String,
        resource: &str,
        provider: JobActivationHandle,
    ) -> String {
        let handle = new_handle("job");
        self.jobs
            .lock()
            .expect("shell job registry poisoned")
            .insert(
                handle.clone(),
                JobHandle {
                    alias,
                    resource: resource.to_string(),
                    provider,
                },
            );
        handle
    }

    /// 返回 `(alias, resource 句柄, client, provider job)`。
    pub(super) fn job(
        &self,
        handle: &str,
    ) -> Result<
        (
            String,
            String,
            ManagedUniversalPluginClient,
            JobActivationHandle,
        ),
        HostError,
    > {
        let record = self
            .jobs
            .lock()
            .map_err(|_| navop_error(ErrorCode::RuntimeUnavailable, "shell job registry poisoned"))?
            .get(handle)
            .cloned()
            .ok_or_else(|| invalid_handle("job", handle))?;
        let client = self.client(&record.alias)?;
        if client.generation != record.provider.generation {
            return Err(stale_handle("job", handle));
        }
        Ok((record.alias, record.resource, client, record.provider))
    }

    pub(super) fn close_job_record(&self, handle: &str) {
        self.jobs
            .lock()
            .expect("shell job registry poisoned")
            .remove(handle);
    }

    pub(super) fn register_event(
        &self,
        alias: String,
        resource: &str,
        generation: u64,
        provider_id: &str,
    ) -> Result<String, HostError> {
        self.insert_provider_handle(
            &self.events,
            "event",
            alias,
            resource,
            generation,
            provider_id,
        )
    }

    pub(super) fn event(
        &self,
        handle: &str,
    ) -> Result<(ManagedUniversalPluginClient, String), HostError> {
        self.provider_handle(&self.events, "event", handle)
    }

    pub(super) fn close_event_record(&self, handle: &str, provider_id: &str) {
        remove_matching(&self.events, handle, provider_id);
    }

    pub(super) fn runtime_info(&self, alias: &str) -> Result<HostValue, HostError> {
        let client = self.client(alias)?;
        Ok(HostObject::new()
            .field("backend", alias)
            .field("runtimeId", client.runtime_id)
            .field(
                "generation",
                json_to_host(&serde_json::json!(client.generation))?,
            )
            .into())
    }

    fn checked_client(
        &self,
        record: &ProviderHandle,
    ) -> Result<ManagedUniversalPluginClient, HostError> {
        let client = self.client(&record.alias)?;
        if client.generation != record.generation {
            return Err(stale_handle("provider", &record.provider_id));
        }
        Ok(client)
    }

    fn insert_provider_handle(
        &self,
        registry: &Mutex<HashMap<String, ProviderHandle>>,
        kind: &str,
        alias: String,
        resource: &str,
        generation: u64,
        provider_id: &str,
    ) -> Result<String, HostError> {
        let runtime_id = self.backends.get(&alias).cloned().ok_or_else(|| {
            navop_error(
                ErrorCode::BackendNotFound,
                format!("unknown backend alias `{alias}`"),
            )
        })?;
        let handle = new_handle(kind);
        registry
            .lock()
            .expect("shell handle registry poisoned")
            .insert(
                handle.clone(),
                ProviderHandle {
                    alias,
                    runtime_id,
                    generation,
                    provider_id: provider_id.to_string(),
                    owned: true,
                    resource: Some(resource.to_string()),
                },
            );
        Ok(handle)
    }

    fn provider_handle(
        &self,
        registry: &Mutex<HashMap<String, ProviderHandle>>,
        kind: &str,
        handle: &str,
    ) -> Result<(ManagedUniversalPluginClient, String), HostError> {
        let record = registry
            .lock()
            .map_err(|_| {
                navop_error(ErrorCode::RuntimeUnavailable, "shell handle registry poisoned")
            })?
            .get(handle)
            .cloned()
            .ok_or_else(|| invalid_handle(kind, handle))?;
        let client = self.checked_client(&record)?;
        Ok((client, record.provider_id))
    }
}

pub(crate) use cleanup::ScopedHandles;
use cleanup::{drain_scoped, invalid_handle, new_handle, remove_matching, stale_handle};
mod cleanup;
