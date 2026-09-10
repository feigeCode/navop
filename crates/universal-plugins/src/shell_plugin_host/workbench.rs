use extension_plugin_adapter::{
    BindingContext, ResourceSessionHandle, dispatch_invoke, dispatch_job,
};
use gpui_shell::{HostError, HostModule};

use super::{
    resource::task::spawn_provider_task,
    value::{host_to_json, json_to_host},
};

pub(super) fn workbench_module(
    session: ResourceSessionHandle,
    descriptor: extension_runtime::RegisteredResourceWorkbenchContribution,
    page_context: serde_json::Value,
    tokio: tokio::runtime::Handle,
) -> HostModule {
    let current_context = page_context.clone();
    HostModule::new("navop.workbench")
        .declarations(
            r#"
            export function current(): unknown;
            export function dispatch(operationId: string, input?: unknown, options?: { confirmed?: boolean }): Promise<unknown>;
            "#,
        )
        .function("current", move |_| json_to_host(&current_context))
        .cancellable_async_function("dispatch", move |arguments| {
            let operation_id = arguments.string(0)?.to_owned();
            let input = arguments
                .get(1)
                .map(host_to_json)
                .transpose()?
                .unwrap_or(serde_json::Value::Null);
            let confirmed = arguments
                .get(2)
                .map(host_to_json)
                .transpose()?
                .and_then(|value| value.get("confirmed").and_then(|v| v.as_bool()))
                .unwrap_or(false);
            let route = page_context
                .get("route")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let selection = page_context
                .get("selection")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let context = BindingContext {
                input,
                route,
                selection,
            };
            let operation = descriptor.operations.get(&operation_id).ok_or_else(|| {
                HostError::new(format!("unknown workbench operation `{operation_id}`"))
            })?;
            let is_job = matches!(
                operation.mode,
                extension_runtime::extension::manifest::ResourceWorkbenchOperationMode::Job
            );
            let session = session.clone();
            let descriptor = descriptor.clone();
            let cancel = extension_host::CancellationToken::new();
            Ok(spawn_provider_task(
                &tokio,
                async move {
                    let result = if is_job {
                        dispatch_job(&session, &descriptor, &operation_id, &context, confirmed).await
                    } else {
                        dispatch_invoke(&session, &descriptor, &operation_id, &context, confirmed)
                            .await
                    }
                    .map_err(|error| HostError::new(error.to_string()))?;
                    json_to_host(&result)
                },
                cancel,
            ))
        })
}
