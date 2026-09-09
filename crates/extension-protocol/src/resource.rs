//! 通用资源生命周期和调用协议。
//!
//! `resource/*` 只定义宿主与 provider 之间的机制。Elasticsearch、Kafka、
//! Kubernetes 等领域能力继续使用各自的 namespaced method 和 JSON payload，
//! 不在公共协议中构造一个不断膨胀的领域 union。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::result_ref::ResultRef;

pub type ResourceId = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceOpenParams {
    pub resource_type: String,
    pub config: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceOpenResult {
    pub resource_id: ResourceId,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcePingParams {
    pub resource_id: ResourceId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceInvokeParams {
    pub resource_id: ResourceId,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceInvokeResult {
    pub result: ResultRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceCloseParams {
    pub resource_id: ResourceId,
}
