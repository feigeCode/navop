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

/// 参数绑定上下文:literal/input/route/selection/paging/parent/connection 七种来源。
#[derive(Debug, Clone, Default)]
pub struct BindingContext {
    pub input: serde_json::Value,
    pub route: serde_json::Value,
    pub selection: serde_json::Value,
    pub paging: serde_json::Value,
    /// 树 lazy 展开时的父节点行数据。
    pub parent: serde_json::Value,
    /// 连接配置字段(来自连接的保存配置)。
    pub connection: serde_json::Value,
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
            S::Parent => Some(&self.parent),
            S::Connection => Some(&self.connection),
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
            // 已是数字的绑定(route/selection/paging)原样透传,保留其整数性。
            serde_json::Value::Number(_) => Ok(value),
            serde_json::Value::String(text) => parse_number(&text).map_err(|_| "number"),
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

/// 表单字段的**文本输入** → 声明的 JSON 类型。
///
/// 与 `coerce_binding` 分开是必要的,两者处理的是不同性质的值:
///
/// - `coerce_binding` 处理 route/selection/paging 这类**已经是 JSON** 的来源,
///   它的 `T::Json` 必须原样透传 —— 那里的值本身可能就是字符串;
/// - 这里处理 query 页的**文本框**,`type: json` 的语义是"用户输入了一段 JSON
///   文本",必须解析成真正的 JSON 值(否则 `{"a":1}` 会以字符串形式发给
///   provider,`{"kind":"object"}` 之类的契约直接失败)。
///
/// 空值不在这里补空串:调用方按 `required` 决定是报错还是跳过该参数。
pub fn parse_input_text(
    text: &str,
    value_type: extension_runtime::extension::manifest::ResourceWorkbenchInputType,
) -> Result<serde_json::Value, &'static str> {
    use extension_runtime::extension::manifest::ResourceWorkbenchInputType as T;
    match value_type {
        T::String => Ok(serde_json::Value::String(text.to_string())),
        // 复用同一条整数/浮点判定:表单里的 "8" 必须落成 JSON 整数。
        T::Number => parse_number(text).map_err(|_| "number"),
        T::Boolean => match text.trim() {
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            _ => Err("boolean"),
        },
        T::Json => serde_json::from_str(text).map_err(|_| "json"),
    }
}

/// 表单字段提交:`parse_input_text` 的**面向用户**包装。
///
/// 与裸 `parse_input_text` 分开,是因为失败信息要说清是哪个字段、错在哪:
/// 表单校验失败是用户可见的错误,不能只报 "number"。
///
/// 空串的处置由调用方负责(见 `resource_view` 的 `run_query`):空串对
/// `string` 是合法值,对另外三种类型不是,调用方会跳过该参数而不是在这里
/// 造一个必然转换失败的空值。
pub fn parse_form_input(
    field_id: &str,
    text: &str,
    value_type: extension_runtime::extension::manifest::ResourceWorkbenchInputType,
) -> Result<serde_json::Value, String> {
    parse_input_text(text, value_type)
        .map_err(|kind| format!("`{field_id}` must be a valid {kind}"))
}

/// 文本 → JSON 数字:整数文本产出整数,其余产出浮点。
///
/// **必须区分整数与浮点**:provider 侧的计数/端口/QoS/分页等参数在契约里是
/// `u32`/`u64`/`i64`,而 serde_json 拒绝把 `1.0` 反序列化进整数类型
/// (`invalid type: floating point 1.0, expected u32`)。query 页的输入值都是
/// **字符串**,若一律按 `f64` 转换,「订阅 QoS」这类整数参数就会在 provider
/// 侧解析失败 —— 所以 `"1"` 必须先尝试整数解析。
fn parse_number(text: &str) -> Result<serde_json::Value, ()> {
    let text = text.trim();
    if let Ok(integer) = text.parse::<i64>() {
        return Ok(serde_json::json!(integer));
    }
    // 超出 i64 上限的无符号值(如 u64::MAX)。
    if let Ok(unsigned) = text.parse::<u64>() {
        return Ok(serde_json::json!(unsigned));
    }
    text.parse::<f64>()
        .map(|number| serde_json::json!(number))
        .map_err(|_| ())
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
        ResourceWorkbenchInputType, ResourceWorkbenchOperation, ResourceWorkbenchOperationMode,
        ResourceWorkbenchValueType,
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
        operations.insert(
            "listPods".to_string(),
            ResourceWorkbenchOperation {
                mode: ResourceWorkbenchOperationMode::Invoke,
                method: "example/pods".into(),
                requires: vec![],
                effect: ResourceWorkbenchEffect::Read,
                params: [(
                    "namespace".to_string(),
                    ResourceWorkbenchBinding {
                        source: ResourceWorkbenchBindingSource::Parent,
                        path: "/name".into(),
                        value_type: ResourceWorkbenchValueType::String,
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
            layout: None,
            pages: vec![],
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
    fn build_request_binds_parent_params() {
        let context = BindingContext {
            parent: serde_json::json!({"name": "default"}),
            ..Default::default()
        };
        let request = build_request(&workbench(), "listPods", &context).unwrap();
        assert_eq!(serde_json::json!({"namespace": "default"}), request.params);
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
        // 整数文本必须落成 JSON 整数:provider 侧 queue_count 是 u32,
        // serde 拒收 `8.0`(见 `parse_number` 的单测)。
        assert_eq!(serde_json::json!({"queueCount": 8}), request.params);
    }

    #[test]
    fn number_input_keeps_integers_integral() {
        // 整数:走 i64/u64 分支,不能变成浮点。
        assert_eq!(serde_json::json!(1), parse_number("1").unwrap());
        assert_eq!(serde_json::json!(0), parse_number("0").unwrap());
        assert_eq!(serde_json::json!(-3), parse_number("-3").unwrap());
        assert_eq!(serde_json::json!(42), parse_number(" 42 ").unwrap());
        assert_eq!(
            serde_json::json!(u64::MAX),
            parse_number("18446744073709551615").unwrap()
        );
        // 真浮点:保持浮点语义。
        assert_eq!(serde_json::json!(2.5), parse_number("2.5").unwrap());
        // 非数字:调用方转成 BindingType 错误。
        assert!(parse_number("abc").is_err());
        assert!(parse_number("").is_err());
    }

    #[test]
    fn integral_number_roundtrips_into_unsigned_contract_fields() {
        // 复现回归:query 页输入 "1" 绑到 provider 侧的 `Option<u32>` 字段
        // (如 middleware/topic/create 的 queue_count)时,一旦产出 `1.0`
        // 就会在 serde 反序列化阶段失败(`invalid type: floating point`),
        // 因此强转结果必须是 JSON 整数(`as_u64()` 为 Some)。
        let coerced = coerce_binding(
            serde_json::Value::String("1".into()),
            ResourceWorkbenchValueType::Number,
        )
        .unwrap();
        assert_eq!(Some(1), coerced.as_u64(), "coerced={coerced}");
    }

    #[test]
    fn json_input_parses_into_real_json_value() {
        // 回归:`type: json` 的字段此前一律以字符串提交,`{"enabled":true}`
        // 到 provider 手里是 `"{\"enabled\":true}"`,对象契约直接失败。
        assert_eq!(
            serde_json::json!({"enabled": true}),
            parse_input_text(r#"{"enabled":true}"#, ResourceWorkbenchInputType::Json).unwrap()
        );
        // 顶层非对象也合法(数组/标量都是 JSON)。
        assert_eq!(
            serde_json::json!([1, 2]),
            parse_input_text("[1,2]", ResourceWorkbenchInputType::Json).unwrap()
        );
        // 非法 JSON 必须报错,而不是被当成字符串放行。
        assert!(parse_input_text("{oops}", ResourceWorkbenchInputType::Json).is_err());
    }

    #[test]
    fn string_input_is_never_parsed_as_json() {
        // `type: string` 的输入哪怕长得像 JSON 也是字面量:自动解析会改变
        // 合法字符串的含义(与 `coerce_binding` 的 `T::Json` 透传同理)。
        assert_eq!(
            serde_json::json!("{\"enabled\":true}"),
            parse_input_text(r#"{"enabled":true}"#, ResourceWorkbenchInputType::String).unwrap()
        );
    }

    #[test]
    fn number_and_boolean_inputs_are_typed() {
        // 表单里的整数文本必须保持整数性(provider 侧是 u32)。
        let parsed = parse_input_text("8", ResourceWorkbenchInputType::Number).unwrap();
        assert_eq!(Some(8), parsed.as_u64(), "parsed={parsed}");
        assert_eq!(
            serde_json::json!(true),
            parse_input_text("true", ResourceWorkbenchInputType::Boolean).unwrap()
        );
        assert_eq!(
            serde_json::json!(false),
            parse_input_text("false", ResourceWorkbenchInputType::Boolean).unwrap()
        );
        // 空值与垃圾值交给调用方转成字段级错误(空的可选字段不提交)。
        assert!(parse_input_text("", ResourceWorkbenchInputType::Number).is_err());
        assert!(parse_input_text("maybe", ResourceWorkbenchInputType::Boolean).is_err());
    }

    #[test]
    fn form_input_errors_name_the_field() {
        // 字段级错误必须能定位到具体输入框,否则用户只知道"某个字段错了"。
        let error = parse_form_input("replicas", "abc", ResourceWorkbenchInputType::Number)
            .expect_err("non-numeric text must not parse");
        assert!(error.contains("replicas"), "error={error}");
        assert!(error.contains("number"), "error={error}");

        let error = parse_form_input("payload", "{oops}", ResourceWorkbenchInputType::Json)
            .expect_err("invalid json must not parse");
        assert!(error.contains("payload") && error.contains("json"), "error={error}");

        assert_eq!(
            serde_json::json!(true),
            parse_form_input("force", "true", ResourceWorkbenchInputType::Boolean).unwrap()
        );
    }
}
