//! Workbench operation dispatch: typed parameter binding and result decoding.
//!
//! 本模块不依赖 GPUI。所有 workbench 页面(tree/collection/json/query)的
//! provider 调用都经过这里,保证权限/参数/结果契约只有一份实现。

use extension_protocol::blob::BlobReadParams;
use extension_protocol::resource::{ResourceInvokeParams, ResourceInvokeResult};
use extension_protocol::result_ref::ResultRef;

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
}

/// 命名操作的执行输入:绑定的参数值已经过类型检查。
#[derive(Debug, Clone)]
pub struct WorkbenchRequest {
    pub operation_id: String,
    pub params: serde_json::Value,
}

/// 参数绑定上下文:literal/input/route/selection 四种来源。
#[derive(Debug, Clone, Default)]
pub struct BindingContext {
    pub input: serde_json::Value,
    pub route: serde_json::Value,
    pub selection: serde_json::Value,
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
            S::Paging => None,
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
                picked
            }
        };
        params.insert(name.clone(), value);
    }
    Ok(WorkbenchRequest {
        operation_id: operation_id.to_string(),
        params: serde_json::Value::Object(params),
    })
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

/// 对指定会话执行一次命名 invoke 操作并解码结果。
pub async fn dispatch_invoke(
    session: &ResourceSessionHandle,
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
    context: &BindingContext,
) -> Result<serde_json::Value, WorkbenchDispatchError> {
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
    decode_result(client, &result).await
}

/// scope 变体:页面作用域内执行,便于后续挂 request revision/cancellation。
pub async fn dispatch_invoke_scoped(
    scope: &ResourceScope,
    workbench: &extension_runtime::RegisteredResourceWorkbenchContribution,
    operation_id: &str,
    context: &BindingContext,
) -> Result<serde_json::Value, WorkbenchDispatchError> {
    dispatch_invoke(&scope.session(), workbench, operation_id, context).await
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
}
