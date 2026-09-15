use extension_host::CancellationToken;
use extension_plugin_adapter::{
    BindingContext, ResourceSessionHandle, dispatch_invoke_scoped, dispatch_job_scoped,
};
use gpui_shell::HostModule;

use super::{
    error::{ErrorCode, navop_error, workbench_error},
    resource::task::spawn_provider_task,
    value::{host_to_json, json_to_host},
};

pub(super) fn workbench_module(
    session: ResourceSessionHandle,
    descriptor: extension_runtime::RegisteredResourceWorkbenchContribution,
    page_context: serde_json::Value,
    root: CancellationToken,
    tokio: tokio::runtime::Handle,
) -> HostModule {
    let current_context = page_context.clone();
    let page_id = page_context
        .get("pageId")
        .and_then(|v| v.as_str())
        .unwrap_or("shell")
        .to_owned();
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
                paging: page_context
                    .get("paging")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"page": 1, "limit": 50, "cursor": null})),
                parent: serde_json::Value::Null,
                // 与原生工作台同源:宿主把连接上下文放进 `page_context`,
                // 这里原样取出。两边解析 `source: connection` 必须得到同一个值,
                // 否则同一个 manifest 在 shell 页与原生页行为不一致。
                connection: page_context
                    .get("connection")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            };
            let operation = descriptor.operations.get(&operation_id).ok_or_else(|| {
                navop_error(
                    ErrorCode::InvalidArgument,
                    format!("unknown workbench operation `{operation_id}`"),
                )
            })?;
            let is_job = matches!(
                operation.mode,
                extension_runtime::extension::manifest::ResourceWorkbenchOperationMode::Job
            );
            let cancel = root.child();
            let scope = session.scope_with_cancel(page_id.clone(), 0, cancel.clone());
            let descriptor = descriptor.clone();
            Ok(spawn_provider_task(
                &tokio,
                async move {
                    let result = if is_job {
                        dispatch_job_scoped(&scope, &descriptor, &operation_id, &context, confirmed)
                            .await
                    } else {
                        dispatch_invoke_scoped(
                            &scope,
                            &descriptor,
                            &operation_id,
                            &context,
                            confirmed,
                        )
                        .await
                    }
                    .map_err(workbench_error)?;
                    json_to_host(&result)
                },
                cancel,
            ))
        })
}
