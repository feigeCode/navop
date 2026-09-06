use std::sync::Arc;

use extension_host::{CancellationToken, RequestOptions};
use extension_plugin_adapter::ManagedUniversalPluginClient;
use extension_protocol::resource::{
    ResourceCloseParams, ResourceInvokeParams, ResourceOpenParams, ResourcePingParams,
};
use gpui_shell::{HostAsyncTask, HostError, HostModule, HostValue};

use self::task::{host_error, spawn_provider_task};
use super::{session::ShellMountSession, value::host_to_json};

pub(super) fn resource_module(session: Arc<ShellMountSession>) -> HostModule {
    HostModule::new("navop.resource")
        .declarations(
            r#"
            export type ResultRef =
              | { kind: "inline"; value: unknown }
              | { kind: "blob"; handle: string }
              | { kind: "event_stream"; handle: string };
            export function open(backend: string, resourceType: string, config: unknown): Promise<{ handle: string; capabilities: string[]; metadata: unknown }>;
            export function invoke(handle: string, method: string, params?: unknown): Promise<ResultRef>;
            export function ping(handle: string): Promise<void>;
            export function close(handle: string): Promise<void>;
            "#,
        )
        .cancellable_async_function("open", open_resource(Arc::clone(&session)))
        .cancellable_async_function("invoke", invoke_resource(Arc::clone(&session)))
        .cancellable_async_function("ping", ping_resource(Arc::clone(&session)))
        .cancellable_async_function("close", close_resource(session))
}

fn open_resource(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let alias = arguments.string(0)?.to_owned();
        let resource_type = arguments.string(1)?.to_owned();
        let config = host_to_json(arguments.value(2)?)?;
        let client = session.client(&alias)?;
        let cancel = CancellationToken::new();
        let request_cancel = cancel.clone();
        let task_session = Arc::clone(&session);
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                let result = client
                    .client()
                    .open_resource_with_options(
                        &ResourceOpenParams {
                            resource_type,
                            config,
                            metadata: None,
                        },
                        RequestOptions::default().with_cancel(request_cancel.clone()),
                    )
                    .await
                    .map_err(host_error)?;
                if request_cancel.is_cancelled() {
                    compensate_open(&client, &result.resource_id).await;
                    return Err(HostError::new("resource open cancelled"));
                }
                task_session.register_resource(alias, &client, result)
            },
            cancel,
        ))
    }
}

async fn compensate_open(client: &ManagedUniversalPluginClient, resource_id: &str) {
    let _ = client
        .client()
        .close_resource(&ResourceCloseParams {
            resource_id: resource_id.to_string(),
        })
        .await;
}

fn invoke_resource(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let handle = arguments.string(0)?.to_owned();
        let method = arguments.string(1)?.to_owned();
        let params = arguments
            .get(2)
            .map(host_to_json)
            .transpose()?
            .unwrap_or(serde_json::Value::Null);
        let (alias, client, resource_id) = session.resource(&handle)?;
        let generation = client.generation;
        let cancel = CancellationToken::new();
        let request_cancel = cancel.clone();
        let task_session = Arc::clone(&session);
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                let result = client
                    .client()
                    .invoke_resource_with_options(
                        &ResourceInvokeParams {
                            resource_id,
                            method,
                            params,
                        },
                        RequestOptions::default().with_cancel(request_cancel),
                    )
                    .await
                    .map_err(host_error)?;
                task_session.result_ref(alias, generation, &result.result)
            },
            cancel,
        ))
    }
}

fn ping_resource(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let (_, client, resource_id) = session.resource(arguments.string(0)?)?;
        let cancel = CancellationToken::new();
        let request_cancel = cancel.clone();
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                client
                    .client()
                    .ping_resource_with_options(
                        &ResourcePingParams { resource_id },
                        RequestOptions::default().with_cancel(request_cancel),
                    )
                    .await
                    .map_err(host_error)?;
                Ok(HostValue::Null)
            },
            cancel,
        ))
    }
}

fn close_resource(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let handle = arguments.string(0)?.to_owned();
        let (_, client, resource_id) = session.resource(&handle)?;
        let cancel = CancellationToken::new();
        let request_cancel = cancel.clone();
        let task_session = Arc::clone(&session);
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                client
                    .client()
                    .close_resource_with_options(
                        &ResourceCloseParams {
                            resource_id: resource_id.clone(),
                        },
                        RequestOptions::default().with_cancel(request_cancel),
                    )
                    .await
                    .map_err(host_error)?;
                task_session.close_resource_record(&handle, &resource_id);
                Ok(HostValue::Null)
            },
            cancel,
        ))
    }
}

pub(super) mod task;
