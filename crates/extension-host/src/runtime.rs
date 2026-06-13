//! 统一的扩展运行时抽象。
//!
//! 提供统一接口，让上层调用者无需区分 IPC 扩展还是 Component 扩展。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::{HostError, HostResult};
use crate::host_api::HostApiProvider;

/// 扩展运行时类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionRuntimeType {
    /// 基于 IPC 的进程扩展。
    Ipc,
    /// 基于 Wasmtime Component Model 的沙箱扩展。
    Component,
}

/// 扩展运行时统一接口。
///
/// 隐藏 IPC 和 Component 的实现差异，提供统一的生命周期和调用接口。
#[async_trait]
pub trait ExtensionRuntime: Send + Sync {
    /// 获取运行时类型。
    fn runtime_type(&self) -> ExtensionRuntimeType;

    /// 获取扩展 ID。
    fn extension_id(&self) -> &str;

    /// 调用扩展的 `activate` 生命周期方法。
    async fn activate(&mut self) -> HostResult<()>;

    /// 调用扩展的命令或方法。
    async fn call(&mut self, method: &str, params: Value) -> HostResult<Value>;

    /// 调用扩展的 `deactivate` 生命周期方法。
    async fn deactivate(&mut self) -> HostResult<()>;

    /// 检查扩展进程是否存活（仅 IPC 扩展有效）。
    fn is_alive(&self) -> bool {
        true
    }
}

/// IPC 扩展运行时。
///
/// 封装 `JsonRpcClientHandle`，通过进程间通信调用扩展。
pub struct IpcExtensionRuntime {
    extension_id: String,
    client: crate::JsonRpcClientHandle,
}

impl IpcExtensionRuntime {
    pub fn new(extension_id: String, client: crate::JsonRpcClientHandle) -> Self {
        Self {
            extension_id,
            client,
        }
    }
}

#[async_trait]
impl ExtensionRuntime for IpcExtensionRuntime {
    fn runtime_type(&self) -> ExtensionRuntimeType {
        ExtensionRuntimeType::Ipc
    }

    fn extension_id(&self) -> &str {
        &self.extension_id
    }

    async fn activate(&mut self) -> HostResult<()> {
        self.client
            .call("extension/activate", Value::Null, Default::default())
            .await
    }

    async fn call(&mut self, method: &str, params: Value) -> HostResult<Value> {
        self.client.call(method, params, Default::default()).await
    }

    async fn deactivate(&mut self) -> HostResult<()> {
        self.client
            .call("extension/deactivate", Value::Null, Default::default())
            .await
    }

    fn is_alive(&self) -> bool {
        !self.client.is_closed()
    }
}

/// Component 扩展运行时。
///
/// 封装 Wasmtime Component 实例，在沙箱中执行扩展。
pub struct ComponentExtensionRuntime {
    extension_id: String,
    // TODO: 持有 ComponentInstance
    // instance: ComponentInstance,
}

impl ComponentExtensionRuntime {
    pub fn new(extension_id: String) -> Self {
        Self { extension_id }
    }
}

#[async_trait]
impl ExtensionRuntime for ComponentExtensionRuntime {
    fn runtime_type(&self) -> ExtensionRuntimeType {
        ExtensionRuntimeType::Component
    }

    fn extension_id(&self) -> &str {
        &self.extension_id
    }

    async fn activate(&mut self) -> HostResult<()> {
        // TODO: 调用 ComponentInstance::activate
        Ok(())
    }

    async fn call(&mut self, method: &str, _params: Value) -> HostResult<Value> {
        // TODO: 根据 method 路由到对应的 WIT export
        Err(HostError::NotImplemented(format!(
            "component runtime call not implemented: {}",
            method
        )))
    }

    async fn deactivate(&mut self) -> HostResult<()> {
        // TODO: 调用 ComponentInstance::deactivate
        Ok(())
    }
}

/// 扩展运行时工厂。
///
/// 根据扩展类型（IPC 或 Component）创建对应的运行时实例。
pub struct ExtensionRuntimeFactory {
    #[allow(dead_code)]
    host_api: Arc<dyn HostApiProvider>,
}

impl ExtensionRuntimeFactory {
    pub fn new(host_api: Arc<dyn HostApiProvider>) -> Self {
        Self { host_api }
    }

    /// 创建 IPC 扩展运行时。
    pub fn create_ipc_runtime(
        &self,
        extension_id: String,
        client: crate::JsonRpcClientHandle,
    ) -> Box<dyn ExtensionRuntime> {
        Box::new(IpcExtensionRuntime::new(extension_id, client))
    }

    /// 创建 Component 扩展运行时（TODO: 完善参数）。
    pub fn create_component_runtime(&self, extension_id: String) -> Box<dyn ExtensionRuntime> {
        Box::new(ComponentExtensionRuntime::new(extension_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_types_are_distinct() {
        assert_ne!(ExtensionRuntimeType::Ipc, ExtensionRuntimeType::Component);
    }

    #[test]
    fn component_runtime_returns_correct_type() {
        let runtime = ComponentExtensionRuntime::new("test-ext".into());
        assert_eq!(runtime.runtime_type(), ExtensionRuntimeType::Component);
        assert_eq!(runtime.extension_id(), "test-ext");
    }
}
