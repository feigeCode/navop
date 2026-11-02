//! 页面挂载期间的能力平面会话（nsh 形态）。
//!
//! 与 gpui-shell 版 `ShellMountSession` 同样的记账规则：resource / blob /
//! event / job 四张句柄表、provider 重启后的 generation 过期检测、以及
//! 「随 resource 关闭回收派生句柄」。差别只在出入口是 JSON —— nsh 的
//! `host.call` 传一个 JSON 载荷，页面拿到的也是 JSON，不再需要 HostValue
//! 那一层转换。
//!
//! 线程模型：页面 handler 跑在引擎线程上，绝不能阻塞，因此这里所有方法
//! 都是 async（由 handler 搬到 tokio 上跑），结构体本身必须 `Send + Sync`
//! ——所以不能像信任库那样放 `Rc<RefCell<_>>`。

use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
    time::Duration,
};

use extension_host::{CancellationToken, RequestOptions};
use extension_plugin_adapter::{
    BindingContext, JobActivationHandle, ManagedUniversalPluginClient, ResourceSessionHandle,
    dispatch_invoke_scoped, dispatch_job_scoped,
};
use extension_protocol::{
    blob::{BlobCloseParams, BlobReadParams, MAX_BLOB_CHUNK_BYTES},
    event_stream::{EventCloseParams, EventOpenParams, EventReadParams},
    job::JobStartParams,
    resource::{ResourceCloseParams, ResourceInvokeParams, ResourceOpenParams, ResourcePingParams},
    result_ref::ResultRef,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    host_error::{ErrorCode, NavopError, service_error},
    universal_plugins::UniversalPluginService,
};
use extension_protocol::resource::ResourceOpenResult;
use extension_runtime::RegisteredResourceWorkbenchContribution;

/// provider 侧关闭的总预算；超时后放弃剩余句柄，交由 runtime 最终 shutdown 兜底。
const CLOSE_ALL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct ProviderHandle {
    /// provider 运行时 id：清理时按它取客户端（此时已不关心别名）。
    runtime_id: String,
    generation: u64,
    provider_id: String,
    /// 派生自哪个 resource 句柄；resource 关闭时只回收属于它的子句柄。
    resource: Option<String>,
    /// 句柄是否归本会话所有。资源工作台**借**过来的主资源是 `false`：
    /// 关闭页面只放弃使用权，句柄本身归外层连接会话，关掉就把别人的
    /// 连接掐了。
    owned: bool,
}

/// 嵌入资源工作台的页面需要的上下文：工作台描述、借来的会话、
/// 以及宿主给的页面上下文（route/selection/connection）。
struct WorkbenchContext {
    descriptor: RegisteredResourceWorkbenchContribution,
    handle: ResourceSessionHandle,
    page_context: Value,
}

#[derive(Clone)]
struct JobHandle {
    runtime_id: String,
    provider: JobActivationHandle,
    resource: String,
}

/// 一次页面挂载对应的能力会话。
pub(crate) struct NshSession {
    service: UniversalPluginService,
    backends: BTreeMap<String, String>,
    resources: Mutex<HashMap<String, ProviderHandle>>,
    blobs: Mutex<HashMap<String, ProviderHandle>>,
    events: Mutex<HashMap<String, ProviderHandle>>,
    jobs: Mutex<HashMap<String, JobHandle>>,
    workbench: Mutex<Option<std::sync::Arc<WorkbenchContext>>>,
    root: CancellationToken,
    tokio: tokio::runtime::Handle,
}

impl NshSession {
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
            workbench: Mutex::new(None),
            root: CancellationToken::new(),
            tokio,
        }
    }

    /// 每次 host 调用派生的子令牌；页面关闭时随根令牌一起取消。
    pub(crate) fn call_token(&self) -> CancellationToken {
        self.root.child()
    }

    /// 取消所有仍在飞行中的调用。
    pub(crate) fn cancel(&self) {
        self.root.cancel();
    }

    pub(crate) fn tokio(&self) -> tokio::runtime::Handle {
        self.tokio.clone()
    }

    // ---- resource -----------------------------------------------------

    pub(crate) async fn open_resource(
        &self,
        alias: &str,
        resource_type: String,
        config: Value,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let client = self.client(alias)?;
        let result = client
            .client()
            .open_resource_with_options(
                &ResourceOpenParams {
                    resource_type,
                    config,
                    metadata: None,
                },
                RequestOptions::default().with_cancel(cancel.clone()),
            )
            .await
            .map_err(NavopError::from)?;
        if cancel.is_cancelled() {
            // 页面已经关了：provider 那边刚建出来的 resource 不能泄漏。
            let _ = client
                .client()
                .close_resource(&ResourceCloseParams {
                    resource_id: result.resource_id.clone(),
                })
                .await;
            return Err(cancelled("resource open cancelled"));
        }
        let handle = new_handle("resource");
        self.resources
            .lock()
            .map_err(|_| poisoned("resource"))?
            .insert(
                handle.clone(),
                ProviderHandle {
                    runtime_id: client.runtime_id.clone(),
                    generation: client.generation,
                    provider_id: result.resource_id.clone(),
                    resource: None,
                    owned: true,
                },
            );
        Ok(json!({
            "handle": handle,
            "capabilities": result.capabilities,
            "metadata": result.metadata,
        }))
    }

    pub(crate) async fn invoke_resource(
        &self,
        handle: &str,
        method: String,
        params: Value,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let (client, resource_id) = self.resource(handle)?;
        let result = client
            .client()
            .invoke_resource_with_options(
                &ResourceInvokeParams {
                    resource_id,
                    method,
                    params,
                },
                RequestOptions::default().with_cancel(cancel),
            )
            .await
            .map_err(NavopError::from)?;
        self.result_ref(&client, handle, &result.result)
    }

    pub(crate) async fn ping_resource(
        &self,
        handle: &str,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let (client, resource_id) = self.resource(handle)?;
        client
            .client()
            .ping_resource_with_options(
                &ResourcePingParams { resource_id },
                RequestOptions::default().with_cancel(cancel),
            )
            .await
            .map_err(NavopError::from)?;
        Ok(Value::Null)
    }

    pub(crate) async fn close_resource(
        &self,
        handle: &str,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let (client, resource_id) = self.resource(handle)?;
        self.close_children(handle).await;
        client
            .client()
            .close_resource_with_options(
                &ResourceCloseParams {
                    resource_id: resource_id.clone(),
                },
                RequestOptions::default().with_cancel(cancel),
            )
            .await
            .map_err(NavopError::from)?;
        self.forget_matching(&self.resources, handle, &resource_id);
        Ok(Value::Null)
    }

    // ---- job ----------------------------------------------------------

    pub(crate) async fn start_job(
        &self,
        resource: &str,
        method: String,
        params: Value,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let (client, resource_id) = self.resource(resource)?;
        let job = client
            .start_job_with_options(
                &JobStartParams {
                    resource_id: Some(resource_id),
                    method,
                    params,
                },
                RequestOptions::default().with_cancel(cancel.clone()),
            )
            .await
            .map_err(NavopError::from)?;
        if cancel.is_cancelled() {
            let _ = client.cancel_job(&job).await;
            let _ = client.close_job(&job).await;
            return Err(cancelled("job start cancelled"));
        }
        let handle = new_handle("job");
        self.jobs.lock().map_err(|_| poisoned("job"))?.insert(
            handle.clone(),
            JobHandle {
                runtime_id: client.runtime_id.clone(),
                provider: job,
                resource: resource.to_owned(),
            },
        );
        // provider 协议目前只有 "running" 一种初始状态（job 的终态靠 result 得知）。
        Ok(json!({ "handle": handle, "state": "running" }))
    }

    pub(crate) async fn job_status(
        &self,
        handle: &str,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let (client, job) = self.job(handle)?;
        let status = client
            .job_status_with_options(&job, RequestOptions::default().with_cancel(cancel))
            .await
            .map_err(NavopError::from)?;
        serde_json::to_value(status).map_err(protocol)
    }

    pub(crate) async fn cancel_job(
        &self,
        handle: &str,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let (client, job) = self.job(handle)?;
        client
            .cancel_job_with_options(&job, RequestOptions::default().with_cancel(cancel))
            .await
            .map_err(NavopError::from)?;
        Ok(Value::Null)
    }

    pub(crate) async fn job_result(
        &self,
        handle: &str,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let (client, job) = self.job(handle)?;
        let resource = self
            .jobs
            .lock()
            .map_err(|_| poisoned("job"))?
            .get(handle)
            .map(|record| record.resource.clone())
            .ok_or_else(|| invalid_handle("job", handle))?;
        let result = client
            .job_result_with_options(&job, RequestOptions::default().with_cancel(cancel))
            .await
            .map_err(NavopError::from)?;
        self.result_ref(&client, &resource, &result.result)
    }

    pub(crate) async fn close_job(
        &self,
        handle: &str,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let (client, job) = self.job(handle)?;
        client
            .close_job_with_options(&job, RequestOptions::default().with_cancel(cancel))
            .await
            .map_err(NavopError::from)?;
        self.jobs
            .lock()
            .map_err(|_| poisoned("job"))?
            .remove(handle);
        Ok(Value::Null)
    }

    // ---- event --------------------------------------------------------

    pub(crate) async fn open_event(
        &self,
        resource: &str,
        kind: String,
        capacity: Option<u32>,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let (client, _) = self.resource(resource)?;
        let result = client
            .open_event_stream_with_options(
                &EventOpenParams {
                    conn_id: None,
                    kind,
                    capacity,
                },
                RequestOptions::default().with_cancel(cancel.clone()),
            )
            .await
            .map_err(NavopError::from)?;
        if cancel.is_cancelled() {
            let _ = client
                .close_event_stream(&EventCloseParams {
                    stream_id: result.stream_id.clone(),
                })
                .await;
            return Err(cancelled("event open cancelled"));
        }
        let handle = self.insert_provider_handle(
            &self.events,
            "event",
            resource,
            client.generation,
            &result.stream_id,
        )?;
        Ok(json!({ "handle": handle }))
    }

    pub(crate) async fn read_event(
        &self,
        handle: &str,
        max_events: Option<u32>,
        wait_ms: Option<u32>,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let (client, stream_id) = self.provider_handle(&self.events, "event", handle)?;
        let result = client
            .read_event_stream_with_options(
                &EventReadParams {
                    stream_id,
                    max_events,
                    wait_ms,
                },
                RequestOptions::default().with_cancel(cancel),
            )
            .await
            .map_err(NavopError::from)?;
        serde_json::to_value(result).map_err(protocol)
    }

    pub(crate) async fn close_event(
        &self,
        handle: &str,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let (client, stream_id) = self.provider_handle(&self.events, "event", handle)?;
        client
            .close_event_stream_with_options(
                &EventCloseParams {
                    stream_id: stream_id.clone(),
                },
                RequestOptions::default().with_cancel(cancel),
            )
            .await
            .map_err(NavopError::from)?;
        self.forget_matching(&self.events, handle, &stream_id);
        Ok(Value::Null)
    }

    // ---- blob ---------------------------------------------------------

    pub(crate) async fn read_blob(
        &self,
        handle: &str,
        max_bytes: Option<u32>,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let max_bytes = max_bytes
            .filter(|value| (1..=MAX_BLOB_CHUNK_BYTES).contains(value))
            .map(Some)
            .ok_or_else(|| {
                NavopError::new(
                    ErrorCode::InvalidArgument,
                    format!("maxBytes must be between 1 and {MAX_BLOB_CHUNK_BYTES}"),
                )
            });

        let max_bytes = max_bytes?;
        let (client, blob_id) = self.provider_handle(&self.blobs, "blob", handle)?;
        let result = client
            .read_blob_with_options(
                &BlobReadParams { blob_id, max_bytes },
                RequestOptions::default().with_cancel(cancel),
            )
            .await
            .map_err(NavopError::from)?;
        Ok(json!({
            "data": result.data,
            "bytesRead": result.bytes_read,
            "done": result.done,
        }))
    }

    pub(crate) async fn close_blob(
        &self,
        handle: &str,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let (client, blob_id) = self.provider_handle(&self.blobs, "blob", handle)?;
        client
            .close_blob_with_options(
                &BlobCloseParams {
                    blob_id: blob_id.clone(),
                },
                RequestOptions::default().with_cancel(cancel),
            )
            .await
            .map_err(NavopError::from)?;
        self.forget_matching(&self.blobs, handle, &blob_id);
        Ok(Value::Null)
    }

    // ---- runtime ------------------------------------------------------

    pub(crate) fn runtime_info(&self, alias: &str) -> Result<Value, NavopError> {
        let client = self.client(alias)?;
        Ok(json!({
            "backend": alias,
            "runtimeId": client.runtime_id,
            "generation": client.generation,
        }))
    }

    // ---- 内部 ---------------------------------------------------------

    /// 把 provider 的 `ResultRef` 落成页面句柄。
    fn result_ref(
        &self,
        client: &ManagedUniversalPluginClient,
        resource: &str,
        result: &ResultRef,
    ) -> Result<Value, NavopError> {
        match result {
            ResultRef::Inline { value } => Ok(json!({ "kind": "inline", "value": value })),
            ResultRef::Blob { id } => Ok(json!({
                "kind": "blob",
                "handle": self.insert_provider_handle(
                    &self.blobs, "blob", resource, client.generation, id,
                )?,
            })),
            ResultRef::EventStream { id } => Ok(json!({
                "kind": "event_stream",
                "handle": self.insert_provider_handle(
                    &self.events, "event", resource, client.generation, id,
                )?,
            })),
        }
    }

    fn client(&self, alias: &str) -> Result<ManagedUniversalPluginClient, NavopError> {
        let runtime_id = self.backends.get(alias).ok_or_else(|| {
            NavopError::new(
                ErrorCode::BackendNotFound,
                format!("unknown backend alias `{alias}`"),
            )
        })?;
        self.service
            .universal_plugin_client(runtime_id)
            .map_err(service_error)
    }

    fn resource(&self, handle: &str) -> Result<(ManagedUniversalPluginClient, String), NavopError> {
        let record = self
            .resources
            .lock()
            .map_err(|_| poisoned("resource"))?
            .get(handle)
            .cloned()
            .ok_or_else(|| invalid_handle("resource", handle))?;
        let client = self.checked_client(&record)?;
        Ok((client, record.provider_id))
    }

    fn job(
        &self,
        handle: &str,
    ) -> Result<(ManagedUniversalPluginClient, JobActivationHandle), NavopError> {
        let record = self
            .jobs
            .lock()
            .map_err(|_| poisoned("job"))?
            .get(handle)
            .cloned()
            .ok_or_else(|| invalid_handle("job", handle))?;
        // job 句柄绑定启动它的那个 provider 进程；进程重启后句柄作废。
        let client = self
            .service
            .universal_plugin_client(&record.runtime_id)
            .map_err(service_error)?;
        if client.generation != record.provider.generation {
            return Err(stale_handle("job", handle));
        }
        Ok((client, record.provider))
    }

    fn checked_client(
        &self,
        record: &ProviderHandle,
    ) -> Result<ManagedUniversalPluginClient, NavopError> {
        let client = self
            .service
            .universal_plugin_client(&record.runtime_id)
            .map_err(service_error)?;
        if client.generation != record.generation {
            return Err(stale_handle("provider", &record.provider_id));
        }
        Ok(client)
    }

    fn insert_provider_handle(
        &self,
        registry: &Mutex<HashMap<String, ProviderHandle>>,
        kind: &str,
        resource: &str,
        generation: u64,
        provider_id: &str,
    ) -> Result<String, NavopError> {
        let runtime_id = self
            .resources
            .lock()
            .map_err(|_| poisoned("resource"))?
            .get(resource)
            .map(|entry| entry.runtime_id.clone())
            .ok_or_else(|| invalid_handle("resource", resource))?;
        let handle = new_handle(kind);
        registry.lock().map_err(|_| poisoned(kind))?.insert(
            handle.clone(),
            ProviderHandle {
                runtime_id,
                generation,
                provider_id: provider_id.to_owned(),
                resource: Some(resource.to_owned()),
                owned: true,
            },
        );
        Ok(handle)
    }

    fn provider_handle(
        &self,
        registry: &Mutex<HashMap<String, ProviderHandle>>,
        kind: &str,
        handle: &str,
    ) -> Result<(ManagedUniversalPluginClient, String), NavopError> {
        let record = registry
            .lock()
            .map_err(|_| poisoned(kind))?
            .get(handle)
            .cloned()
            .ok_or_else(|| invalid_handle(kind, handle))?;
        let client = self.checked_client(&record)?;
        Ok((client, record.provider_id))
    }

    fn forget_matching(
        &self,
        registry: &Mutex<HashMap<String, ProviderHandle>>,
        handle: &str,
        provider_id: &str,
    ) {
        if let Ok(mut records) = registry.lock()
            && records
                .get(handle)
                .is_some_and(|record| record.provider_id == provider_id)
        {
            records.remove(handle);
        }
    }

    /// 关闭派生自该 resource 的 blob / event / job 句柄；其他 resource 的不动。
    async fn close_children(&self, resource: &str) {
        let jobs = self
            .jobs
            .lock()
            .map(|mut jobs| {
                let (scoped, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *jobs)
                    .into_iter()
                    .partition(|(_, job)| job.resource == resource);
                jobs.extend(rest);
                scoped.into_iter().map(|(_, job)| job).collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let blobs = drain_scoped(&self.blobs, resource);
        let events = drain_scoped(&self.events, resource);
        close_all(&self.service, Vec::new(), blobs, events, jobs).await;
    }

    // ---- 借来的资源 + 资源工作台 ---------------------------------------

    /// 把外层连接会话持有的主资源借给页面：句柄能 invoke，但**不归本会话所有**
    /// （`owned: false`），页面关掉时不调 provider 的 close。
    pub(crate) fn register_borrowed_resource(
        &self,
        client: &ManagedUniversalPluginClient,
        result: ResourceOpenResult,
    ) -> Result<Value, NavopError> {
        let handle = new_handle("resource");
        self.resources
            .lock()
            .map_err(|_| poisoned("resource"))?
            .insert(
                handle.clone(),
                ProviderHandle {
                    runtime_id: client.runtime_id.clone(),
                    generation: client.generation,
                    provider_id: result.resource_id,
                    resource: None,
                    owned: false,
                },
            );
        Ok(json!({
            "handle": handle,
            "capabilities": result.capabilities,
            "metadata": result.metadata,
        }))
    }

    /// 连接页拥有的主资源：页面关闭时随会话回收（与嵌入页"借来的"相反）。
    pub(crate) fn register_connection_resource(
        &self,
        client: &ManagedUniversalPluginClient,
        result: ResourceOpenResult,
    ) -> Result<String, NavopError> {
        let handle = new_handle("resource");
        self.resources
            .lock()
            .map_err(|_| poisoned("resource"))?
            .insert(
                handle.clone(),
                ProviderHandle {
                    runtime_id: client.runtime_id.clone(),
                    generation: client.generation,
                    provider_id: result.resource_id,
                    resource: None,
                    owned: true,
                },
            );
        Ok(handle)
    }

    pub(crate) fn set_workbench(
        &self,
        descriptor: RegisteredResourceWorkbenchContribution,
        handle: ResourceSessionHandle,
        page_context: Value,
    ) {
        if let Ok(mut slot) = self.workbench.lock() {
            *slot = Some(std::sync::Arc::new(WorkbenchContext {
                descriptor,
                handle,
                page_context,
            }));
        }
    }

    pub(crate) fn workbench_current(&self) -> Result<Value, NavopError> {
        Ok(self.workbench()?.page_context.clone())
    }

    /// 派发一个工作台操作。`current()` 给的上下文原样进 `BindingContext`，
    /// 与原生工作台同源（两边解析 `source: connection` 必须得到同一个值）。
    pub(crate) async fn workbench_dispatch(
        &self,
        operation_id: &str,
        input: Value,
        confirmed: bool,
        cancel: CancellationToken,
    ) -> Result<Value, NavopError> {
        let workbench = self.workbench()?;
        let page_context = &workbench.page_context;
        let page_id = page_context
            .get("pageId")
            .and_then(Value::as_str)
            .unwrap_or("nsh")
            .to_owned();
        let context = BindingContext {
            input,
            route: page_context.get("route").cloned().unwrap_or(Value::Null),
            selection: page_context
                .get("selection")
                .cloned()
                .unwrap_or(Value::Null),
            paging: page_context
                .get("paging")
                .cloned()
                .unwrap_or_else(|| json!({"page": 1, "limit": 50, "cursor": null})),
            parent: Value::Null,
            connection: page_context
                .get("connection")
                .cloned()
                .unwrap_or(Value::Null),
        };
        let operation = workbench
            .descriptor
            .operations
            .get(operation_id)
            .ok_or_else(|| {
                NavopError::new(
                    ErrorCode::InvalidArgument,
                    format!("unknown workbench operation `{operation_id}`"),
                )
            })?;
        let is_job = matches!(
            operation.mode,
            extension_runtime::extension::manifest::ResourceWorkbenchOperationMode::Job
        );
        let scope = workbench
            .handle
            .scope_with_cancel(page_id, 0, cancel.clone());
        let descriptor = workbench.descriptor.clone();
        let result = if is_job {
            dispatch_job_scoped(&scope, &descriptor, operation_id, &context, confirmed).await
        } else {
            dispatch_invoke_scoped(&scope, &descriptor, operation_id, &context, confirmed).await
        }
        .map_err(crate::host_error::workbench_error)?;
        Ok(result)
    }

    fn workbench(&self) -> Result<std::sync::Arc<WorkbenchContext>, NavopError> {
        self.workbench
            .lock()
            .map_err(|_| poisoned("workbench"))?
            .clone()
            .ok_or_else(|| {
                NavopError::new(
                    ErrorCode::InvalidArgument,
                    "navop.workbench 只在资源工作台嵌入的页面里可用",
                )
            })
    }
}

impl Drop for NshSession {
    fn drop(&mut self) {
        self.cancel();
        let resources = take(&self.resources);
        let blobs = take(&self.blobs);
        let events = take(&self.events);
        let jobs = take_jobs(&self.jobs);
        if resources.is_empty() && blobs.is_empty() && events.is_empty() && jobs.is_empty() {
            return;
        }
        let service = self.service.clone();
        // Drop 里不能 await：把关闭交给 tokio，并设上限。
        self.tokio.spawn(async move {
            let _ = tokio::time::timeout(
                CLOSE_ALL_TIMEOUT,
                close_all(&service, resources, blobs, events, jobs),
            )
            .await;
        });
    }
}

async fn close_all(
    service: &UniversalPluginService,
    resources: Vec<ProviderHandle>,
    blobs: Vec<ProviderHandle>,
    events: Vec<ProviderHandle>,
    jobs: Vec<JobHandle>,
) {
    for job in jobs {
        if let Ok(client) = service.universal_plugin_client(&job.runtime_id) {
            let _ = client.close_job(&job.provider).await;
        }
    }
    for blob in blobs {
        if let Ok(client) = service.universal_plugin_client(&blob.runtime_id) {
            let _ = client
                .client()
                .close_blob(&BlobCloseParams {
                    blob_id: blob.provider_id,
                })
                .await;
        }
    }
    for event in events {
        if let Ok(client) = service.universal_plugin_client(&event.runtime_id) {
            let _ = client
                .client()
                .close_event_stream(&EventCloseParams {
                    stream_id: event.provider_id,
                })
                .await;
        }
    }
    for resource in resources.into_iter().filter(|resource| resource.owned) {
        if let Ok(client) = service.universal_plugin_client(&resource.runtime_id) {
            let _ = client
                .client()
                .close_resource(&ResourceCloseParams {
                    resource_id: resource.provider_id,
                })
                .await;
        }
    }
}

fn take(registry: &Mutex<HashMap<String, ProviderHandle>>) -> Vec<ProviderHandle> {
    registry
        .lock()
        .map(|mut records| std::mem::take(&mut *records))
        .unwrap_or_default()
        .into_values()
        .collect()
}

fn take_jobs(registry: &Mutex<HashMap<String, JobHandle>>) -> Vec<JobHandle> {
    registry
        .lock()
        .map(|mut records| std::mem::take(&mut *records))
        .unwrap_or_default()
        .into_values()
        .collect()
}

fn drain_scoped(
    registry: &Mutex<HashMap<String, ProviderHandle>>,
    resource: &str,
) -> Vec<ProviderHandle> {
    let Ok(mut records) = registry.lock() else {
        return Vec::new();
    };
    let (scoped, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *records)
        .into_iter()
        .partition(|(_, record)| record.resource.as_deref() == Some(resource));
    records.extend(rest);
    scoped.into_iter().map(|(_, record)| record).collect()
}

fn new_handle(kind: &str) -> String {
    format!("{kind}-{}", Uuid::new_v4())
}

fn cancelled(what: &str) -> NavopError {
    NavopError::new(ErrorCode::RequestCancelled, what)
}

fn invalid_handle(kind: &str, handle: &str) -> NavopError {
    NavopError::new(
        ErrorCode::InvalidHandle,
        format!("invalid {kind} handle `{handle}`"),
    )
}

fn stale_handle(kind: &str, handle: &str) -> NavopError {
    NavopError::new(
        ErrorCode::StaleHandle,
        format!("stale {kind} handle `{handle}` after provider restart"),
    )
}

fn poisoned(registry: &str) -> NavopError {
    NavopError::new(
        ErrorCode::RuntimeUnavailable,
        format!("nsh {registry} registry poisoned"),
    )
}

fn protocol(error: serde_json::Error) -> NavopError {
    NavopError::new(ErrorCode::ProtocolError, error.to_string())
}
