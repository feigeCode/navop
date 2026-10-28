use gpui::App;
use std::fmt;

/// 领域标识，用于注册表去重与诊断日志。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DomainId(&'static str);

impl DomainId {
    pub const DATABASE: Self = Self("database");
    pub const SHELL: Self = Self("shell");
    pub const CORE: Self = Self("core");

    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for DomainId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

/// 一个可装配的领域。应用只负责按产品选择 `Domain` 列表。
pub trait Domain: Send + Sync {
    fn id(&self) -> DomainId;

    fn register(&self, registry: &mut super::DomainRegistry);

    fn init(&self, cx: &mut App);
}
