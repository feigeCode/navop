//! 工具 trait 与注册表。

use crate::error::ToolError;
use crate::resource::ResourceContext;
use crate::tools::invocation::ToolInvocation;
use crate::tools::observation::ToolObservation;
use crate::tools::spec::{ToolName, ToolSpec};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// 可被 Runtime 调用的工具。
///
/// 实现者既可以是回显之类的内置工具,也可以是真实的 SQL / SSH 工具。工具自己
/// 决定如何根据 [`ToolInvocation`] 中的资源上下文获取连接并执行。
#[async_trait]
pub trait Tool: Send + Sync {
    /// 工具名称(注册表键 / function 名)。
    fn name(&self) -> ToolName;

    /// 返回工具规格。允许依据当前资源上下文动态生成(例如把可用资源 ID 写进
    /// 参数枚举),因此入参带 `resources`。
    fn spec(&self, resources: &ResourceContext) -> ToolSpec;

    /// 是否支持与其它工具并行执行。第一版统一串行,默认 `false`。
    fn supports_parallel(&self) -> bool {
        false
    }

    /// 执行工具。返回 `Err` 时由 [`ToolRouter`](crate::tools::ToolRouter) 统一
    /// 转换为失败观测写回历史。
    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError>;
}

/// 工具注册表。可克隆(内部是 `Arc` 共享)。
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<ToolName, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册工具(按 [`Tool::name`] 为键),返回自身便于链式调用。
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> &mut Self {
        self.tools.insert(tool.name(), tool);
        self
    }

    /// 链式构造:注册并返回 `Self`。
    pub fn with_tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.register(tool);
        self
    }

    pub fn extend(&mut self, other: ToolRegistry) -> &mut Self {
        for (name, tool) in other.tools {
            self.tools.insert(name, tool);
        }
        self
    }

    pub fn get(&self, name: &ToolName) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn contains(&self, name: &ToolName) -> bool {
        self.tools.contains_key(name)
    }

    pub fn names(&self) -> Vec<ToolName> {
        self.tools.keys().cloned().collect()
    }

    /// 收集所有工具在当前资源上下文下的规格。
    pub fn specs(&self, resources: &ResourceContext) -> Vec<ToolSpec> {
        self.tools.values().map(|t| t.spec(resources)).collect()
    }

    /// 某工具是否支持并行。
    pub fn supports_parallel(&self, name: &ToolName) -> bool {
        self.tools
            .get(name)
            .map(|t| t.supports_parallel())
            .unwrap_or(false)
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
