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

    /// 领域初始化；返回错误时由产品启动流程决定是否退出。
    fn init(&self, cx: &mut App) -> anyhow::Result<()>;

    /// 注册领域自己的快捷键。
    ///
    /// 公共壳在通用绑定之后按注册顺序调用，领域可以在自己的全局绑定量不冲突时
    /// 覆盖顺序要求（例如 SQL 编辑器必须在笔记的全局绑定之后注册）。
    fn refresh_keybindings(&self, _cx: &mut App) {}

    /// 有界关停领域持有的资源；返回的任务会在退出流程中被 await。
    ///
    /// 公共壳不知道领域里有什么，只按注册顺序等待。
    /// 领域自己的键位（例如 SFTP 视图的粘贴快捷键）。
    fn keybindings(&self, _cx: &App) -> Vec<gpui::KeyBinding> {
        Vec::new()
    }

    fn shutdown(&self, _cx: &mut App) -> Option<gpui::Task<()>> {
        None
    }
}
