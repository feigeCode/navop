//! 通用资源插件的 typed RPC facade。
//!
//! 这里仅封装稳定的机制方法。Kafka、Kubernetes 等领域操作仍通过
//! `ResourceInvokeParams::method` 或 `JobStartParams::method` 传递 namespaced
//! method，不在宿主侧扩展成领域枚举。

use extension_protocol::{
    blob::{BlobCloseParams, BlobOpenParams, BlobOpenResult, BlobReadParams, BlobReadResult},
    event_stream::{
        EventCloseParams, EventOpenParams, EventOpenResult, EventReadParams, EventReadResult,
    },
    job::{
        JobCancelParams, JobCloseParams, JobResultParams, JobResultResult, JobStartParams,
        JobStartResult, JobStatusParams, JobStatusResult,
    },
    method,
    resource::{
        ResourceCloseParams, ResourceInvokeParams, ResourceInvokeResult, ResourceOpenParams,
        ResourceOpenResult, ResourcePingParams,
    },
};
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;

use crate::{HostError, HostResult, ProcessRpcSession, RequestOptions};

pub type OpenAuthorizer = Arc<dyn Fn(&ResourceOpenParams) -> HostResult<()> + Send + Sync>;

/// 在一个已协商的进程 session 上调用通用资源、任务、事件与 blob 协议。
#[derive(Clone)]
pub struct UniversalPluginClient {
    session: Arc<ProcessRpcSession>,
    open_authorizer: Option<OpenAuthorizer>,
}

impl UniversalPluginClient {
    pub fn new(session: Arc<ProcessRpcSession>) -> Self {
        Self {
            session,
            open_authorizer: None,
        }
    }

    pub fn with_open_authorizer(mut self, authorizer: OpenAuthorizer) -> Self {
        self.open_authorizer = Some(authorizer);
        self
    }

    pub fn session(&self) -> &Arc<ProcessRpcSession> {
        &self.session
    }

    pub async fn open_resource(
        &self,
        params: &ResourceOpenParams,
    ) -> HostResult<ResourceOpenResult> {
        self.open_resource_with_options(params, RequestOptions::default())
            .await
    }

    pub async fn open_resource_with_options(
        &self,
        params: &ResourceOpenParams,
        options: RequestOptions,
    ) -> HostResult<ResourceOpenResult> {
        if let Some(authorizer) = &self.open_authorizer {
            authorizer(params)?;
        }
        self.request_with_options(method::RESOURCE_OPEN, params, options)
            .await
    }

    pub async fn ping_resource(&self, params: &ResourcePingParams) -> HostResult<()> {
        self.ping_resource_with_options(params, RequestOptions::default())
            .await
    }

    pub async fn ping_resource_with_options(
        &self,
        params: &ResourcePingParams,
        options: RequestOptions,
    ) -> HostResult<()> {
        self.request_with_options(method::RESOURCE_PING, params, options)
            .await
    }

    pub async fn invoke_resource(
        &self,
        params: &ResourceInvokeParams,
    ) -> HostResult<ResourceInvokeResult> {
        self.invoke_resource_with_options(params, RequestOptions::default())
            .await
    }

    pub async fn invoke_resource_with_options(
        &self,
        params: &ResourceInvokeParams,
        options: RequestOptions,
    ) -> HostResult<ResourceInvokeResult> {
        self.request_with_options(method::RESOURCE_INVOKE, params, options)
            .await
    }

    pub async fn close_resource(&self, params: &ResourceCloseParams) -> HostResult<()> {
        self.close_resource_with_options(params, RequestOptions::default())
            .await
    }

    pub async fn close_resource_with_options(
        &self,
        params: &ResourceCloseParams,
        options: RequestOptions,
    ) -> HostResult<()> {
        self.request_with_options(method::RESOURCE_CLOSE, params, options)
            .await
    }

    pub async fn start_job(&self, params: &JobStartParams) -> HostResult<JobStartResult> {
        self.request(method::JOB_START, params).await
    }

    pub async fn job_status(&self, params: &JobStatusParams) -> HostResult<JobStatusResult> {
        self.request(method::JOB_STATUS, params).await
    }

    pub async fn cancel_job(&self, params: &JobCancelParams) -> HostResult<()> {
        self.request(method::JOB_CANCEL, params).await
    }

    pub async fn job_result(&self, params: &JobResultParams) -> HostResult<JobResultResult> {
        self.request(method::JOB_RESULT, params).await
    }

    pub async fn close_job(&self, params: &JobCloseParams) -> HostResult<()> {
        self.request(method::JOB_CLOSE, params).await
    }

    pub async fn open_blob(&self, params: &BlobOpenParams) -> HostResult<BlobOpenResult> {
        self.request(method::BLOB_OPEN, params).await
    }

    pub async fn read_blob(&self, params: &BlobReadParams) -> HostResult<BlobReadResult> {
        self.request(method::BLOB_READ, params).await
    }

    pub async fn close_blob(&self, params: &BlobCloseParams) -> HostResult<()> {
        self.request(method::BLOB_CLOSE, params).await
    }

    pub async fn open_event_stream(&self, params: &EventOpenParams) -> HostResult<EventOpenResult> {
        self.request(method::EVENT_OPEN, params).await
    }

    pub async fn read_event_stream(&self, params: &EventReadParams) -> HostResult<EventReadResult> {
        self.read_event_stream_with_options(params, RequestOptions::default())
            .await
    }

    pub async fn read_event_stream_with_options(
        &self,
        params: &EventReadParams,
        options: RequestOptions,
    ) -> HostResult<EventReadResult> {
        self.request_with_options(method::EVENT_READ, params, options)
            .await
    }

    pub async fn close_event_stream(&self, params: &EventCloseParams) -> HostResult<()> {
        self.request(method::EVENT_CLOSE, params).await
    }

    async fn request<P, R>(&self, method_name: &str, params: &P) -> HostResult<R>
    where
        P: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let negotiated = self.session.session();
        if negotiated.has_method_declarations() && !negotiated.declares_method(method_name) {
            return Err(HostError::NotImplemented(format!(
                "extension did not declare wire method `{method_name}`"
            )));
        }

        self.request_with_options(method_name, params, RequestOptions::default())
            .await
    }

    async fn request_with_options<P, R>(
        &self,
        method_name: &str,
        params: &P,
        options: RequestOptions,
    ) -> HostResult<R>
    where
        P: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let negotiated = self.session.session();
        if negotiated.has_method_declarations() && !negotiated.declares_method(method_name) {
            return Err(HostError::NotImplemented(format!(
                "extension did not declare wire method `{method_name}`"
            )));
        }
        let params = serde_json::to_value(params)?;
        self.session
            .request_value_with_options(method_name, params, options)
            .await
            .and_then(|value| serde_json::from_value(value).map_err(HostError::from))
    }
}

#[cfg(test)]
#[path = "universal_plugin_tests.rs"]
mod tests;
