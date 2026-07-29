//! 工具 trait 与注册表。

use crate::error::ToolError;
use crate::resource::ResourceContext;
use crate::tools::invocation::ToolInvocation;
use crate::tools::observation::ToolObservation;
use crate::tools::spec::{ToolName, ToolSpec};
use async_trait::async_trait;
use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fmt;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolRegistryError {
    duplicate_tool_names: Vec<String>,
}

impl ToolRegistryError {
    pub fn duplicate_tool_names(&self) -> Vec<String> {
        self.duplicate_tool_names.clone()
    }
}

impl fmt::Display for ToolRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "duplicate agent tool names: {}",
            self.duplicate_tool_names.join(", ")
        )
    }
}

impl Error for ToolRegistryError {}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册工具(按 [`Tool::name`] 为键),返回自身便于链式调用。
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> &mut Self {
        self.try_register(tool)
            .expect("agent tool names must be unique");
        self
    }

    /// 注册工具；名称冲突时保持原注册表不变并返回错误。
    pub fn try_register(
        &mut self,
        tool: Arc<dyn Tool>,
    ) -> Result<&mut Self, ToolRegistryError> {
        let name = tool.name();
        if self.tools.contains_key(&name) {
            return Err(ToolRegistryError {
                duplicate_tool_names: vec![name.to_string()],
            });
        }
        self.tools.insert(name, tool);
        Ok(self)
    }

    /// 链式构造:注册并返回 `Self`。
    pub fn with_tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.register(tool);
        self
    }

    pub fn extend(&mut self, other: ToolRegistry) -> &mut Self {
        self.try_extend(other)
            .expect("agent tool names must be unique");
        self
    }

    /// 原子合并注册表；任一名称冲突时不合并任何工具。
    pub fn try_extend(
        &mut self,
        other: ToolRegistry,
    ) -> Result<&mut Self, ToolRegistryError> {
        let duplicate_tool_names = other
            .tools
            .keys()
            .filter(|name| self.tools.contains_key(*name))
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if !duplicate_tool_names.is_empty() {
            return Err(ToolRegistryError {
                duplicate_tool_names,
            });
        }
        self.tools.extend(other.tools);
        Ok(self)
    }

    pub fn get(&self, name: &ToolName) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn contains(&self, name: &ToolName) -> bool {
        self.tools.contains_key(name)
    }

    pub fn names(&self) -> Vec<ToolName> {
        let mut names = self.tools.keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }

    /// 收集所有工具在当前资源上下文下的规格。
    pub fn specs(&self, resources: &ResourceContext) -> Vec<ToolSpec> {
        let mut tools = self.tools.iter().collect::<Vec<_>>();
        tools.sort_by(|(left, _), (right, _)| left.cmp(right));
        tools
            .into_iter()
            .map(|(name, tool)| {
                let mut spec = tool.spec(resources);
                spec.name = name.clone();
                spec
            })
            .collect()
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

#[cfg(test)]
mod tests {
    use super::{Tool, ToolRegistry};
    use crate::error::ToolError;
    use crate::resource::ResourceContext;
    use crate::tools::{ToolInvocation, ToolName, ToolObservation, ToolSpec};
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;

    struct NamedTool {
        name: ToolName,
    }

    impl NamedTool {
        fn new(name: &str) -> Arc<Self> {
            Arc::new(Self {
                name: ToolName::new(name),
            })
        }
    }

    #[async_trait]
    impl Tool for NamedTool {
        fn name(&self) -> ToolName {
            self.name.clone()
        }

        fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
            ToolSpec::new(
                self.name.clone(),
                format!("{} description", self.name),
                json!({ "type": "object" }),
            )
        }

        async fn execute(
            &self,
            _invocation: ToolInvocation,
        ) -> Result<ToolObservation, ToolError> {
            Err(ToolError::Execution("not used by registry tests".to_string()))
        }
    }

    #[test]
    fn try_register_rejects_duplicates_without_overwriting() {
        let original = NamedTool::new("sample.echo");
        let replacement = NamedTool::new("sample_echo");
        let mut registry = ToolRegistry::new();
        registry
            .try_register(original.clone())
            .expect("first tool should register");

        let error = match registry.try_register(replacement) {
            Ok(_) => panic!("sanitized duplicate must fail closed"),
            Err(error) => error,
        };

        assert_eq!(vec!["sample_echo".to_string()], error.duplicate_tool_names());
        assert!(Arc::ptr_eq(
            &registry
                .get(&ToolName::new("sample.echo"))
                .expect("original tool must remain registered"),
            &(original as Arc<dyn Tool>),
        ));
        assert_eq!(1, registry.len());
    }

    #[test]
    fn try_extend_is_atomic_when_any_name_conflicts() {
        let original = NamedTool::new("sample.echo");
        let mut registry = ToolRegistry::new();
        registry
            .try_register(original.clone())
            .expect("original tool should register");

        let unique = NamedTool::new("unique.tool");
        let duplicate = NamedTool::new("sample_echo");
        let mut other = ToolRegistry::new();
        other
            .try_register(unique)
            .expect("unique tool should register in source registry");
        other
            .try_register(duplicate)
            .expect("duplicate only conflicts across registries");

        let error = match registry.try_extend(other) {
            Ok(_) => panic!("extend must reject the entire conflicting registry"),
            Err(error) => error,
        };

        assert_eq!(vec!["sample_echo".to_string()], error.duplicate_tool_names());
        assert_eq!(1, registry.len());
        assert!(registry.contains(&ToolName::new("sample.echo")));
        assert!(!registry.contains(&ToolName::new("unique.tool")));
    }

    #[test]
    fn try_extend_merges_unique_registries() {
        let mut registry = ToolRegistry::new();
        registry
            .try_register(NamedTool::new("beta.tool"))
            .expect("beta should register");
        let mut other = ToolRegistry::new();
        other
            .try_register(NamedTool::new("alpha.tool"))
            .expect("alpha should register");

        registry
            .try_extend(other)
            .expect("unique registries should merge");

        assert_eq!(
            vec!["alpha_tool".to_string(), "beta_tool".to_string()],
            registry
                .names()
                .into_iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn names_and_specs_are_sorted_by_public_tool_name() {
        let mut registry = ToolRegistry::new();
        registry
            .try_register(NamedTool::new("zeta.tool"))
            .expect("zeta should register");
        registry
            .try_register(NamedTool::new("alpha.tool"))
            .expect("alpha should register");

        let names = registry
            .names()
            .into_iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        let spec_names = registry
            .specs(&ResourceContext::new())
            .into_iter()
            .map(|spec| spec.name.to_string())
            .collect::<Vec<_>>();

        assert_eq!(vec!["alpha_tool", "zeta_tool"], names);
        assert_eq!(names, spec_names);
    }
}
