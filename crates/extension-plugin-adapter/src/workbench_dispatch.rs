//! Workbench operation dispatch: typed parameter binding and result decoding.
//!
//! 本模块不依赖 GPUI。所有 workbench 页面(tree/collection/json/query)的
//! provider 调用都经过这里,保证权限/参数/结果契约只有一份实现。

use extension_protocol::blob::BlobReadParams;
use extension_protocol::resource::{ResourceInvokeParams, ResourceInvokeResult};
use extension_protocol::result_ref::ResultRef;
use extension_runtime::extension::manifest::ResourceWorkbenchEffect;

use crate::{
    ManagedUniversalPluginClient,
    resource_session::{ResourceScope, ResourceSessionHandle},
};

/// 单次 workbench 结果的解码上限:防止声明错误把超大 JSON 拉进 UI。
const MAX_RESULT_JSON_BYTES: u32 = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WorkbenchDispatchError {
    #[error("unknown operation `{0}`")]
    UnknownOperation(String),
    #[error("operation `{operation}` requires missing capability `{capability}`")]
    MissingCapability {
        operation: String,
        capability: String,
    },
    #[error("binding for param `{param}` produced no value")]
    BindingMissing { param: String },
    #[error("binding for `{param}` produced wrong type: expected {expected}")]
    BindingType {
        param: String,
        expected: &'static str,
    },
    #[error("result contract violation for `{operation}`: {reason}")]
    ResultContract { operation: String, reason: String },
    #[error("provider call failed: {0}")]
    Provider(String),
    #[error("operation `{0}` requires user confirmation before it can run")]
    ConfirmationRequired(String),
}

/// 命名操作的执行输入:绑定的参数值已经过类型检查。
#[derive(Debug, Clone)]
pub struct WorkbenchRequest {
    pub operation_id: String,
    pub params: serde_json::Value,
}

/// 参数绑定上下文:literal/input/route/selection/paging 五种来源。
#[derive(Debug, Clone, Default)]
pub struct BindingContext {
    pub input: serde_json::Value,
    pub route: serde_json::Value,
    pub selection: serde_json::Value,
    pub paging: serde_json::Value,
}

impl BindingContext {
    fn source_value(
        &self,
        source: extension_runtime::extension::manifest::ResourceWorkbenchBindingSource,
    ) -> Option<&serde_json::Value> {
        use extension_runtime::extension::manifest::ResourceWorkbenchBindingSource as S;
        match source {
            S::Literal => None,
            S::Input => Some(&self.input),
            S::Route => Some(&self.route),
            S::Selection => Some(&self.selection),
            S::Paging => Some(&self.paging),
        }
    }
}

fn pointer<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    if path == "/" || path.is_empty() {
        return Some(value);
    }
    // manifest 声明的路径是 `/name` 风格;serde_json pointer 接受
    // `/name` 或 `name`,统一归一为带前导斜杠的形式。
    let normalized = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    value.pointer(&normalized)
}

/// 产出调用参数:按 manifest 声明把绑定上下文投影为 provider params。
pub fn build_request(
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
    context: &BindingContext,
) -> Result<WorkbenchRequest, WorkbenchDispatchError> {
    let operation = workbench
        .operations
        .get(operation_id)
        .ok_or_else(|| WorkbenchDispatchError::UnknownOperation(operation_id.to_string()))?;
    let mut params = serde_json::Map::new();
    for (name, binding) in &operation.params {
        let value = match binding.source {
            extension_runtime::extension::manifest::ResourceWorkbenchBindingSource::Literal => {
                binding.value.clone().unwrap_or(serde_json::Value::Null)
            }
            source => {
                let root = context.source_value(source).ok_or_else(|| {
                    WorkbenchDispatchError::BindingMissing {
                        param: name.clone(),
                    }
                })?;
                let picked = if binding.path.is_empty() {
                    root.clone()
                } else {
                    pointer(root, &binding.path)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null)
                };
                if picked.is_null() {
                    return Err(WorkbenchDispatchError::BindingMissing {
                        param: name.clone(),
                    });
                }
                coerce_binding(picked, binding.value_type).map_err(|expected| {
                    WorkbenchDispatchError::BindingType {
                        param: name.clone(),
                        expected,
                    }
                })?
            }
        };
        params.insert(name.clone(), value);
    }
    Ok(WorkbenchRequest {
        operation_id: operation_id.to_string(),
        params: serde_json::Value::Object(params),
    })
}

/// 按声明类型强制转换绑定值:number/boolean 把字符串/数字转成目标标量。
fn coerce_binding(
    value: serde_json::Value,
    value_type: extension_runtime::extension::manifest::ResourceWorkbenchValueType,
) -> Result<serde_json::Value, &'static str> {
    use extension_runtime::extension::manifest::ResourceWorkbenchValueType as T;
    match value_type {
        T::String => match value {
            serde_json::Value::String(_) => Ok(value),
            other => Ok(serde_json::Value::String(other.to_string())),
        },
        T::Number => match value {
            serde_json::Value::Number(_) => Ok(value),
            serde_json::Value::String(text) => text
                .parse::<f64>()
                .map(|number| serde_json::json!(number))
                .map_err(|_| "number"),
            _ => Err("number"),
        },
        T::Boolean => match value {
            serde_json::Value::Bool(_) => Ok(value),
            serde_json::Value::String(text) => text
                .parse::<bool>()
                .map(serde_json::Value::Bool)
                .map_err(|_| "boolean"),
            _ => Err("boolean"),
        },
        T::Json => Ok(value),
    }
}

/// 校验 operation 的 requires 能力是否全部在会话能力集中。
pub fn ensure_capabilities(
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
    capabilities: &[String],
) -> Result<(), WorkbenchDispatchError> {
    let Some(operation) = workbench.operations.get(operation_id) else {
        return Err(WorkbenchDispatchError::UnknownOperation(
            operation_id.to_string(),
        ));
    };
    for required in &operation.requires {
        if !capabilities.iter().any(|capability| capability == required) {
            return Err(WorkbenchDispatchError::MissingCapability {
                operation: operation_id.to_string(),
                capability: required.clone(),
            });
        }
    }
    Ok(())
}

/// 解码 invoke 结果为 JSON:Inline 直接返回,Blob 有界读取,EventStream 报错。
pub async fn decode_result(
    client: &ManagedUniversalPluginClient,
    result: &ResourceInvokeResult,
) -> Result<serde_json::Value, WorkbenchDispatchError> {
    match &result.result {
        ResultRef::Inline { value } => Ok(value.clone()),
        ResultRef::Blob { id } => read_blob_json(client, id).await,
        ResultRef::EventStream { .. } => Err(WorkbenchDispatchError::ResultContract {
            operation: "invoke".into(),
            reason: "event stream is not a loadable JSON result".into(),
        }),
    }
}

async fn read_blob_json(
    client: &ManagedUniversalPluginClient,
    blob_id: &str,
) -> Result<serde_json::Value, WorkbenchDispatchError> {
    let mut chunks: Vec<u8> = Vec::new();
    let mut remaining = MAX_RESULT_JSON_BYTES;
    loop {
        let read = client
            .read_blob(&BlobReadParams {
                blob_id: blob_id.to_string(),
                max_bytes: Some(remaining.min(256 * 1024)),
            })
            .await
            .map_err(|error| WorkbenchDispatchError::Provider(error.to_string()))?;
        let bytes = base64_decode(&read.data);
        chunks.extend_from_slice(&bytes);
        if read.done {
            break;
        }
        if bytes.is_empty() {
            return Err(WorkbenchDispatchError::ResultContract {
                operation: "blob-read".into(),
                reason: "blob read stalled without progress".into(),
            });
        }
        remaining = remaining.saturating_sub(bytes.len() as u32);
        if remaining == 0 {
            return Err(WorkbenchDispatchError::ResultContract {
                operation: "blob-read".into(),
                reason: "result exceeded the host decode limit".into(),
            });
        }
    }
    serde_json::from_slice(&chunks).map_err(|error| WorkbenchDispatchError::ResultContract {
        operation: "blob-read".into(),
        reason: format!("blob bytes are not valid JSON: {error}"),
    })
}

fn base64_decode(data: &str) -> Vec<u8> {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    BASE64.decode(data).unwrap_or_default()
}

/// 判断 operation 的副作用等级是否需要宿主确认。
/// read 直接放行;write/destructive/unknown 一律需确认。
pub fn requires_confirmation(effect: ResourceWorkbenchEffect) -> bool {
    !matches!(effect, ResourceWorkbenchEffect::Read)
}

/// 返回命名操作的 effect 等级;未知操作返回 None。
pub fn operation_effect(
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
) -> Option<ResourceWorkbenchEffect> {
    workbench
        .operations
        .get(operation_id)
        .map(|operation| operation.effect)
}

/// 对指定会话执行一次命名 invoke 操作并解码结果。
/// `confirmed` 为 true 时跳过副作用确认;非 read 操作未确认时返回 ConfirmationRequired。
pub async fn dispatch_invoke(
    session: &ResourceSessionHandle,
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
    context: &BindingContext,
    confirmed: bool,
) -> Result<serde_json::Value, WorkbenchDispatchError> {
    let result =
        dispatch_invoke_result(session, workbench, operation_id, context, confirmed).await?;
    decode_result(session.client(), &result).await
}

pub async fn dispatch_invoke_result(
    session: &ResourceSessionHandle,
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
    context: &BindingContext,
    confirmed: bool,
) -> Result<ResourceInvokeResult, WorkbenchDispatchError> {
    guard_effect(workbench, operation_id, confirmed)?;
    let request = build_request(workbench, operation_id, context)?;
    let client = session.client();
    ensure_capabilities(workbench, operation_id, session.capabilities())?;
    let resource_id = session
        .resource_id()
        .await
        .map_err(|error| WorkbenchDispatchError::Provider(error.to_string()))?;
    let result = client
        .client()
        .invoke_resource(&ResourceInvokeParams {
            resource_id,
            method: workbench
                .operations
                .get(operation_id)
                .map(|operation| operation.method.clone())
                .unwrap_or_default(),
            params: request.params,
        })
        .await
        .map_err(|error| WorkbenchDispatchError::Provider(error.to_string()))?;
    Ok(result)
}

/// scope 变体:页面作用域内执行,便于后续挂 request revision/cancellation。
pub async fn dispatch_invoke_scoped(
    scope: &ResourceScope,
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
    context: &BindingContext,
    confirmed: bool,
) -> Result<serde_json::Value, WorkbenchDispatchError> {
    let result =
        dispatch_invoke_result_scoped(scope, workbench, operation_id, context, confirmed).await?;
    decode_result(scope.session().client(), &result).await
}

pub async fn dispatch_invoke_result_scoped(
    scope: &ResourceScope,
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
    context: &BindingContext,
    confirmed: bool,
) -> Result<ResourceInvokeResult, WorkbenchDispatchError> {
    let session = scope.session();
    let cancellation = scope.cancellation();
    guard_effect(workbench, operation_id, confirmed)?;
    let request = build_request(workbench, operation_id, context)?;
    ensure_capabilities(workbench, operation_id, session.capabilities())?;
    let resource_id = session
        .resource_id()
        .await
        .map_err(|error| WorkbenchDispatchError::Provider(error.to_string()))?;
    let operation = workbench
        .operations
        .get(operation_id)
        .ok_or_else(|| WorkbenchDispatchError::UnknownOperation(operation_id.to_string()))?;
    let result = session
        .client()
        .client()
        .invoke_resource_with_options(
            &ResourceInvokeParams {
                resource_id,
                method: operation.method.clone(),
                params: request.params,
            },
            extension_host::RequestOptions::default().with_cancel(cancellation),
        )
        .await
        .map_err(|error| WorkbenchDispatchError::Provider(error.to_string()))?;
    Ok(result)
}

/// 副作用门控:非 read 操作未确认时 fail closed。
fn guard_effect(
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
    confirmed: bool,
) -> Result<(), WorkbenchDispatchError> {
    let Some(operation) = workbench.operations.get(operation_id) else {
        return Err(WorkbenchDispatchError::UnknownOperation(
            operation_id.to_string(),
        ));
    };
    if !confirmed && requires_confirmation(operation.effect) {
        return Err(WorkbenchDispatchError::ConfirmationRequired(
            operation_id.to_string(),
        ));
    }
    Ok(())
}

/// job 模式:启动命名 job 操作,轮询到终态,读取结果并关闭。
/// 返回解码后的 JSON 结果;Cancelled/Failed 转为 Provider 错误。
pub async fn dispatch_job(
    session: &ResourceSessionHandle,
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
    context: &BindingContext,
    confirmed: bool,
) -> Result<serde_json::Value, WorkbenchDispatchError> {
    dispatch_job_with_cancel(session, workbench, operation_id, context, confirmed, None).await
}

async fn dispatch_job_with_cancel(
    session: &ResourceSessionHandle,
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
    context: &BindingContext,
    confirmed: bool,
    cancellation: Option<extension_host::CancellationToken>,
) -> Result<serde_json::Value, WorkbenchDispatchError> {
    use extension_protocol::job::JobState;

    const POLL_INTERVAL_MS: u64 = 150;
    const MAX_POLL_ATTEMPTS: u32 = 2000;

    guard_effect(workbench, operation_id, confirmed)?;
    let request = build_request(workbench, operation_id, context)?;
    let client = session.client();
    ensure_capabilities(workbench, operation_id, session.capabilities())?;
    let operation = workbench
        .operations
        .get(operation_id)
        .ok_or_else(|| WorkbenchDispatchError::UnknownOperation(operation_id.to_string()))?;
    let resource_id = session
        .resource_id()
        .await
        .map_err(|error| WorkbenchDispatchError::Provider(error.to_string()))?;

    let handle = client
        .start_job(&extension_protocol::job::JobStartParams {
            resource_id: Some(resource_id),
            method: operation.method.clone(),
            params: request.params,
        })
        .await
        .map_err(|error| WorkbenchDispatchError::Provider(error.to_string()))?;

    let outcome: Result<(), WorkbenchDispatchError> = 'poll: {
        for _ in 0..MAX_POLL_ATTEMPTS {
            let status = client
                .job_status(&handle)
                .await
                .map_err(|error| WorkbenchDispatchError::Provider(error.to_string()))?;
            match status.state {
                JobState::Succeeded => break 'poll Ok(()),
                JobState::Failed => {
                    break 'poll Err(WorkbenchDispatchError::Provider(
                        status.message.unwrap_or_else(|| "job failed".into()),
                    ));
                }
                JobState::Cancelled => {
                    break 'poll Err(WorkbenchDispatchError::Provider("job cancelled".into()));
                }
                JobState::Queued | JobState::Running => {}
            }
            if let Some(cancellation) = &cancellation {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)) => {}
                    _ = cancellation.cancelled() => {
                        let _ = client.cancel_job(&handle).await;
                        let _ = client.close_job(&handle).await;
                        break 'poll Err(WorkbenchDispatchError::Provider("job cancelled with scope".into()));
                    }
                }
            } else {
                tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
        }
        // 上限保护:不无限轮询。
        Err(WorkbenchDispatchError::Provider(
            "job did not reach a terminal state within the host poll budget".into(),
        ))
    };

    let result = match outcome {
        Ok(()) => client
            .job_result(&handle)
            .await
            .map_err(|error| WorkbenchDispatchError::Provider(error.to_string())),
        Err(error) => {
            let _ = client.cancel_job(&handle).await;
            let _ = client.close_job(&handle).await;
            return Err(error);
        }
    }?;
    let decoded = decode_result(
        client,
        &extension_protocol::resource::ResourceInvokeResult {
            result: result.result,
        },
    )
    .await;

    let _ = client.close_job(&handle).await;
    decoded
}

/// scope 变体的 job 调度。
pub async fn dispatch_job_scoped(
    scope: &ResourceScope,
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
    context: &BindingContext,
    confirmed: bool,
) -> Result<serde_json::Value, WorkbenchDispatchError> {
    let session = scope.session();
    dispatch_job_with_cancel(
        &session,
        workbench,
        operation_id,
        context,
        confirmed,
        Some(scope.cancellation()),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_runtime::RegisteredResourceWorkbenchContribution;
    use extension_runtime::extension::manifest::{
        ResourceWorkbenchBinding, ResourceWorkbenchBindingSource, ResourceWorkbenchEffect,
        ResourceWorkbenchOperation, ResourceWorkbenchOperationMode, ResourceWorkbenchValueType,
    };
    use std::collections::BTreeMap;

    fn workbench() -> RegisteredResourceWorkbenchContribution {
        let mut operations = BTreeMap::new();
        operations.insert(
            "indexInfo".to_string(),
            ResourceWorkbenchOperation {
                mode: ResourceWorkbenchOperationMode::Invoke,
                method: "elasticsearch/index/get".into(),
                requires: vec!["elasticsearch/index/get".into()],
                effect: ResourceWorkbenchEffect::Read,
                params: [(
                    "name".to_string(),
                    ResourceWorkbenchBinding {
                        source: ResourceWorkbenchBindingSource::Route,
                        path: "/name".into(),
                        value_type: ResourceWorkbenchValueType::String,
                        value: None,
                    },
                )]
                .into_iter()
                .collect(),
            },
        );
        operations.insert(
            "pagedList".to_string(),
            ResourceWorkbenchOperation {
                mode: ResourceWorkbenchOperationMode::Invoke,
                method: "example/list".into(),
                requires: vec![],
                effect: ResourceWorkbenchEffect::Read,
                params: [(
                    "page".to_string(),
                    ResourceWorkbenchBinding {
                        source: ResourceWorkbenchBindingSource::Paging,
                        path: "/page".into(),
                        value_type: ResourceWorkbenchValueType::Number,
                        value: None,
                    },
                )]
                .into_iter()
                .collect(),
            },
        );
        RegisteredResourceWorkbenchContribution {
            extension_id: "com.example".into(),
            id: "workbench".into(),
            title: "Example".into(),
            connection_ids: vec!["connection".into()],
            runtime_id: "runtime".into(),
            resource_type: "example".into(),
            default_page: "overview".into(),
            operations,
            navigation: vec![],
            tree: vec![],
            pages: vec![],
            status_bar: None,
        }
    }

    #[test]
    fn build_request_binds_route_params() {
        let context = BindingContext {
            route: serde_json::json!({"name": "orders-2026"}),
            ..Default::default()
        };
        let request = build_request(&workbench(), "indexInfo", &context).unwrap();
        assert_eq!(serde_json::json!({"name": "orders-2026"}), request.params);
    }

    #[test]
    fn build_request_binds_paging_params() {
        let context = BindingContext {
            paging: serde_json::json!({"page": 3, "limit": 50, "cursor": null}),
            ..Default::default()
        };
        let request = build_request(&workbench(), "pagedList", &context).unwrap();
        assert_eq!(serde_json::json!({"page": 3}), request.params);
    }

    #[test]
    fn build_request_rejects_paging_param_without_paging_state() {
        let error =
            build_request(&workbench(), "pagedList", &BindingContext::default()).unwrap_err();
        assert!(matches!(
            error,
            WorkbenchDispatchError::BindingMissing { param } if param == "page"
        ));
    }

    #[test]
    fn build_request_rejects_missing_binding() {
        let error =
            build_request(&workbench(), "indexInfo", &BindingContext::default()).unwrap_err();
        assert!(matches!(
            error,
            WorkbenchDispatchError::BindingMissing { param } if param == "name"
        ));
    }

    #[test]
    fn ensure_capabilities_rejects_missing() {
        let error =
            ensure_capabilities(&workbench(), "indexInfo", &["other".to_string()]).unwrap_err();
        assert!(matches!(
            error,
            WorkbenchDispatchError::MissingCapability { .. }
        ));
    }

    #[test]
    fn ensure_capabilities_accepts_declared() {
        ensure_capabilities(
            &workbench(),
            "indexInfo",
            &["elasticsearch/index/get".to_string()],
        )
        .unwrap();
    }

    #[test]
    fn guard_effect_requires_confirmation_for_non_read() {
        let mut wb = workbench();
        wb.operations.insert(
            "deleteIndex".to_string(),
            ResourceWorkbenchOperation {
                mode: ResourceWorkbenchOperationMode::Invoke,
                method: "elasticsearch/index/delete".into(),
                requires: vec!["elasticsearch/index/delete".into()],
                effect: ResourceWorkbenchEffect::Destructive,
                params: BTreeMap::new(),
            },
        );

        assert!(!guard_effect(&wb, "indexInfo", false).is_err());
        assert!(matches!(
            guard_effect(&wb, "deleteIndex", false),
            Err(WorkbenchDispatchError::ConfirmationRequired(_))
        ));
        assert!(guard_effect(&wb, "deleteIndex", true).is_ok());
    }

    #[test]
    fn guard_effect_rejects_unknown_operation() {
        assert!(matches!(
            guard_effect(&workbench(), "missing", false),
            Err(WorkbenchDispatchError::UnknownOperation(_))
        ));
    }

    #[test]
    fn build_request_coerces_number_from_string_input() {
        let mut wb = workbench();
        wb.operations.insert(
            "createTopic".to_string(),
            ResourceWorkbenchOperation {
                mode: ResourceWorkbenchOperationMode::Invoke,
                method: "middleware/topic/create".into(),
                requires: vec!["middleware/topic/create".into()],
                effect: ResourceWorkbenchEffect::Write,
                params: [(
                    "queueCount".to_string(),
                    ResourceWorkbenchBinding {
                        source: ResourceWorkbenchBindingSource::Input,
                        path: "/queueCount".into(),
                        value_type: ResourceWorkbenchValueType::Number,
                        value: None,
                    },
                )]
                .into_iter()
                .collect(),
            },
        );
        let context = BindingContext {
            input: serde_json::json!({"queueCount": "8"}),
            ..Default::default()
        };
        let request = build_request(&wb, "createTopic", &context).unwrap();
        assert_eq!(serde_json::json!({"queueCount": 8.0}), request.params);
    }
}
