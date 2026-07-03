//! 资源上下文。
//!
//! 描述"当前会话可以操作哪些资源"(SSH 主机、MySQL 实例、Redis、终端……)。
//! 这里刻意保持**与具体资源 crate 解耦**:只持有资源的标识、类型与可读标签,
//! 真正的连接句柄由工具实现在执行时按 [`ResourceId`] 自行获取。这样 runtime
//! 内核无需依赖 `db` / `ssh` 等 crate,可独立编译与测试。

use serde::{Deserialize, Serialize};
use std::fmt;

pub type ResourceCapability = tool_runtime::ResourceCapability;

/// 资源的唯一标识(通常对应 onetcli 中的 connection_id)。
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceId(String);

impl ResourceId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ResourceId({})", self.0)
    }
}

impl From<&str> for ResourceId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for ResourceId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// 资源类别。`Other` 兜底未来新增的资源类型,避免频繁改枚举。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Ssh,
    Mysql,
    Postgres,
    Sqlite,
    Redis,
    Mongo,
    Terminal,
    Other(String),
}

impl ResourceKind {
    pub fn as_str(&self) -> &str {
        match self {
            ResourceKind::Ssh => "ssh",
            ResourceKind::Mysql => "mysql",
            ResourceKind::Postgres => "postgres",
            ResourceKind::Sqlite => "sqlite",
            ResourceKind::Redis => "redis",
            ResourceKind::Mongo => "mongo",
            ResourceKind::Terminal => "terminal",
            ResourceKind::Other(s) => s,
        }
    }

    pub fn to_runtime_resource_kind(&self) -> tool_runtime::ResourceKind {
        match self {
            ResourceKind::Ssh => tool_runtime::ResourceKind::Ssh,
            ResourceKind::Mysql => tool_runtime::ResourceKind::Mysql,
            ResourceKind::Postgres => tool_runtime::ResourceKind::Postgres,
            ResourceKind::Sqlite => tool_runtime::ResourceKind::Sqlite,
            ResourceKind::Redis => tool_runtime::ResourceKind::Redis,
            ResourceKind::Mongo => tool_runtime::ResourceKind::Mongo,
            ResourceKind::Terminal => tool_runtime::ResourceKind::Terminal,
            ResourceKind::Other(value) => tool_runtime::ResourceKind::Other(value.clone()),
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 对单个资源的引用。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRef {
    pub id: ResourceId,
    pub kind: ResourceKind,
    /// 人类可读标签(如 "prod-db (10.0.0.1:3306)"),用于 prompt 与展示。
    pub label: String,
    /// 可用于匹配用户自然引用的别名,如 hostname、IP、cloud id。
    #[serde(default)]
    pub aliases: Vec<String>,
    /// 资源下的当前细分作用域,如 database/schema/cwd。
    #[serde(default)]
    pub scopes: Vec<ResourceScope>,
    /// 资源能力,用于 runtime target 解析时避免同名/同 host 的不同资源互相匹配。
    #[serde(default)]
    pub capabilities: Vec<ResourceCapability>,
}

impl ResourceRef {
    pub fn new(id: impl Into<ResourceId>, kind: ResourceKind, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind,
            label: label.into(),
            aliases: Vec::new(),
            scopes: Vec::new(),
            capabilities: Vec::new(),
        }
    }

    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        let alias = alias.into();
        if !alias.is_empty() && !self.aliases.contains(&alias) {
            self.aliases.push(alias);
        }
        self
    }

    pub fn with_scope(mut self, scope: ResourceScope) -> Self {
        self.set_scope(scope);
        self
    }

    pub fn with_capability(mut self, capability: ResourceCapability) -> Self {
        if !self.capabilities.contains(&capability) {
            self.capabilities.push(capability);
        }
        self
    }

    pub fn set_scope(&mut self, scope: ResourceScope) {
        if let Some(existing) = self.scopes.iter_mut().find(|item| item.key == scope.key) {
            *existing = scope;
        } else {
            self.scopes.push(scope);
        }
    }

    pub fn to_runtime_resource_ref(&self) -> tool_runtime::ResourceRef {
        let mut resource = tool_runtime::ResourceRef::new(
            self.id.as_str(),
            self.kind.to_runtime_resource_kind(),
            self.label.clone(),
        );
        for alias in &self.aliases {
            resource = resource.with_alias(alias.clone());
        }
        for scope in &self.scopes {
            resource = resource.with_scope(tool_runtime::ResourceScope::new(
                scope.key.clone(),
                scope.label.clone(),
                scope.value.clone(),
            ));
        }
        for capability in &self.capabilities {
            resource = resource.with_capability(capability.clone());
        }
        resource
    }
}

/// 资源下的细分作用域。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceScope {
    pub key: String,
    pub label: String,
    pub value: String,
}

impl ResourceScope {
    pub fn new(key: impl Into<String>, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value: value.into(),
        }
    }
}

/// 一次会话内的资源集合,以及当前默认聚焦的资源。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResourceContext {
    /// 当前默认聚焦的资源(若工具调用未显式指定 resource,则使用它)。
    pub current: Option<ResourceId>,
    /// 本会话可用的全部资源。
    pub resources: Vec<ResourceRef>,
}

impl ResourceContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn to_runtime_resource_pool(&self) -> tool_runtime::ResourcePool {
        let mut pool = tool_runtime::ResourcePool::new();
        for resource in &self.resources {
            pool = pool.with_resource(resource.to_runtime_resource_ref());
        }
        if let Some(current) = &self.current {
            pool = pool.with_default_target(tool_runtime::ResourceId::new(current.as_str()));
        }
        pool
    }

    /// 追加一个资源;若是第一个资源,自动设为当前聚焦资源。
    pub fn with_resource(mut self, resource: ResourceRef) -> Self {
        if self.current.is_none() {
            self.current = Some(resource.id.clone());
        }
        self.resources.push(resource);
        self
    }

    /// 按 ID 查找资源。
    pub fn get(&self, id: &ResourceId) -> Option<&ResourceRef> {
        self.resources.iter().find(|r| &r.id == id)
    }

    /// 返回当前聚焦的资源。
    pub fn current(&self) -> Option<&ResourceRef> {
        self.current.as_ref().and_then(|id| self.get(id))
    }

    /// 按 ID 查找资源并返回可变引用。
    pub fn get_mut(&mut self, id: &ResourceId) -> Option<&mut ResourceRef> {
        self.resources.iter_mut().find(|r| &r.id == id)
    }

    /// 是否没有任何资源。
    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }

    /// 生成供模型阅读的资源清单描述。
    pub fn describe(&self) -> String {
        if self.resources.is_empty() {
            return "（当前会话没有可操作的资源）".to_string();
        }
        let mut out = String::new();
        for r in &self.resources {
            let marker = if self.current.as_ref() == Some(&r.id) {
                " [当前]"
            } else {
                ""
            };
            out.push_str(&format!(
                "- {} | 类型={} | id={}{}\n",
                r.label, r.kind, r.id, marker
            ));
            if !r.aliases.is_empty() {
                out.push_str(&format!("  - aliases={}\n", r.aliases.join(", ")));
            }
            for scope in &r.scopes {
                out.push_str(&format!(
                    "  - {}={} ({})\n",
                    scope.key, scope.value, scope.label
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_resource_becomes_current() {
        let ctx = ResourceContext::new()
            .with_resource(ResourceRef::new("c1", ResourceKind::Mysql, "prod-db"))
            .with_resource(ResourceRef::new("c2", ResourceKind::Redis, "cache"));
        assert_eq!(ctx.current().unwrap().id, ResourceId::new("c1"));
        assert_eq!(ctx.resources.len(), 2);
        assert!(ctx.get(&ResourceId::new("c2")).is_some());
    }

    #[test]
    fn describe_marks_current() {
        let ctx = ResourceContext::new().with_resource(ResourceRef::new(
            "c1",
            ResourceKind::Ssh,
            "bastion",
        ));
        assert!(ctx.describe().contains("[当前]"));
    }

    #[test]
    fn describe_includes_resource_scopes() {
        let ctx = ResourceContext::new().with_resource(
            ResourceRef::new("c1", ResourceKind::Postgres, "prod")
                .with_scope(ResourceScope::new("database", "Database", "ai_app"))
                .with_scope(ResourceScope::new("schema", "Schema", "public")),
        );

        let description = ctx.describe();

        assert!(description.contains("database=ai_app"));
        assert!(description.contains("schema=public"));
    }

    #[test]
    fn runtime_resource_pool_includes_aliases() {
        let ctx = ResourceContext::new().with_resource(
            ResourceRef::new("c1", ResourceKind::Ssh, "prod-a").with_alias("10.2.4.54"),
        );

        let pool = ctx.to_runtime_resource_pool();

        assert_eq!("prod-a", pool.resolve_target("10.2.4.54").unwrap().label);
        assert!(ctx.describe().contains("aliases=10.2.4.54"));
    }

    #[test]
    fn runtime_resource_pool_includes_capabilities() {
        let ctx = ResourceContext::new().with_resource(
            ResourceRef::new("c1", ResourceKind::Mysql, "prod-db")
                .with_capability(tool_runtime::ResourceCapability::DatabaseQuery),
        );

        let pool = ctx.to_runtime_resource_pool();
        let resource = pool.resolve_target("prod-db").unwrap();

        assert!(
            resource
                .capabilities
                .contains(&tool_runtime::ResourceCapability::DatabaseQuery)
        );
    }
}
