use gpui::{App, Window};
use one_core::storage::StoredConnection;
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

    fn notify(
        &mut self,
        level: NotifyLevel,
        message: &str,
        window: &mut Window,
        cx: &mut App,
    );
}
