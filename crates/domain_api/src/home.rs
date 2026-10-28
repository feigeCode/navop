use gpui::{App, Window};
use one_core::storage::{StoredConnection, Workspace};
use one_core::tab_container::{TabItem, TabOpenMode};

/// 应用壳暴露给领域的 Tab 能力；领域不能直接持有 `HomePage`。
pub trait TabHost {
    fn open_tab(
        &mut self,
        item: TabItem,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut App,
    ) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyLevel {
    Info,
    Warning,
    Error,
}

/// 领域打开连接/表单时能看到的只读上下文与通知入口。
pub trait HomeContext: TabHost {
    fn connections(&self) -> &[StoredConnection];

    /// 按 id 查工作区；领域打开工作区标签时需要它，但不该直接读 `HomePage`。
    fn workspace(&self, id: i64) -> Option<Workspace>;

    /// 按当前产品的排序设置排列工作区连接。
    fn sort_connections(&self, connections: &mut [StoredConnection], cx: &App);

    /// 当前产品的数据库标签打开方式。
    fn database_open_mode(&self, cx: &App) -> crate::DatabaseTabMode;

    fn notify(
        &mut self,
        level: NotifyLevel,
        message: &str,
        window: &mut Window,
        cx: &mut App,
    );
}
