use std::sync::Arc;

use extension_host::CancellationToken;
use extension_protocol::job::JobStartParams;
use gpui_shell::{HostAsyncTask, HostError, HostModule, HostObject, HostValue};

use super::{
    resource::task::{host_error, spawn_provider_task},
    session::ShellMountSession,
    value::{host_to_json, json_to_host},
};

pub(super) fn job_module(session: Arc<ShellMountSession>) -> HostModule {
    HostModule::new("navop.job")
        .declarations(
            r#"
            export function start(resource: string, method: string, params?: unknown): Promise<{ handle: string; state: string }>;
            export function status(handle: string): Promise<{ state: string; progressPercent?: number; message?: string }>;
            export function cancel(handle: string): Promise<void>;
            export function result(handle: string): Promise<unknown>;
            export function close(handle: string): Promise<void>;
            "#,
        )
        .cancellable_async_function("start", start_job(Arc::clone(&session)))
        .cancellable_async_function("status", status_job(Arc::clone(&session)))
        .cancellable_async_function("cancel", cancel_job(Arc::clone(&session)))
        .cancellable_async_function("result", result_job(Arc::clone(&session)))
        .cancellable_async_function("close", close_job(session))
}

fn start_job(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let resource = arguments.string(0)?.to_owned();
        let method = arguments.string(1)?.to_owned();
        let params = arguments
            .get(2)
            .map(host_to_json)
            .transpose()?
            .unwrap_or(serde_json::Value::Null);
        let (alias, client, resource_id) = session.resource(&resource)?;
        let task_session = Arc::clone(&session);
        let cancel = CancellationToken::new();
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                let job = client
                    .start_job(&JobStartParams {
                        resource_id: Some(resource_id),
                        method,
                        params,
                    })
                    .await
                    .map_err(host_error)?;
                let state = json_to_host(
                    &serde_json::to_value(job_state(&job))
                        .map_err(|e| HostError::new(e.to_string()))?,
                )?;
                let handle = task_session.register_job(alias, job);
                Ok(HostObject::new()
                    .field("handle", handle)
                    .field("state", state)
                    .into())
            },
            cancel,
        ))
    }
}

fn status_job(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let handle = arguments.string(0)?.to_owned();
        let (_, client, job) = session.job(&handle)?;
        let cancel = CancellationToken::new();
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                let status = client.job_status(&job).await.map_err(host_error)?;
                json_to_host(
                    &serde_json::to_value(status).map_err(|e| HostError::new(e.to_string()))?,
                )
            },
            cancel,
        ))
    }
}

fn cancel_job(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let (_, client, job) = session.job(arguments.string(0)?)?;
        let cancel = CancellationToken::new();
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                client.cancel_job(&job).await.map_err(host_error)?;
                Ok(HostValue::Null)
            },
            cancel,
        ))
    }
}

fn result_job(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let (alias, client, job) = session.job(arguments.string(0)?)?;
        let generation = client.generation;
        let task_session = Arc::clone(&session);
        let cancel = CancellationToken::new();
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                let result = client.job_result(&job).await.map_err(host_error)?;
                task_session.result_ref(alias, generation, &result.result)
            },
            cancel,
        ))
    }
}

fn close_job(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let handle = arguments.string(0)?.to_owned();
        let (_, client, job) = session.job(&handle)?;
        let task_session = Arc::clone(&session);
        let cancel = CancellationToken::new();
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                client.close_job(&job).await.map_err(host_error)?;
                task_session.close_job_record(&handle);
                Ok(HostValue::Null)
            },
            cancel,
        ))
    }
}

fn job_state(handle: &extension_plugin_adapter::JobActivationHandle) -> &str {
    let _ = handle;
    "running"
}
