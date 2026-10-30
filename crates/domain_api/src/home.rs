use gpui::{App, Subscription, Window};
use one_core::storage::{StoredConnection, Workspace};
use one_core::tab_container::{TabItem, TabOpenMode};
use port_forwarding::PortForwardingRuntime;
use std::sync::Arc;

/// 应用壳暴露给领域的 Tab 能力；领域不能直接持有 `HomePage`。
pub trait TabHost {
    fn open_tab(
        &mut self,
        item: TabItem,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut App,
    ) -> bool;

    /// 打开并聚焦内容；终端一类需要键盘焦点的 Tab 使用它。
    fn open_tab_with_focus(&mut self, item: TabItem, window: &mut Window, cx: &mut App) -> bool {
        self.open_tab(item, TabOpenMode::Activate, window, cx)
    }

    /// 按 id 惰性打开 Tab：已存在同 id 标签时直接激活，不执行 `build`。
    /// 数据库、Redis、端口转发等“打开即建立连接”的领域必须用它。
    fn open_tab_lazy(
        &mut self,
        _id: String,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut App,
        build: Box<dyn FnOnce(&mut Window, &mut App) -> TabItem>,
    ) -> bool {
        let item = build(window, cx);
        self.open_tab(item, mode, window, cx)
    }
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

    /// 当前 Tab 容器里同基础标题的下一个可用序号；没有同名 Tab 时返回 None。
    fn next_tab_index(&self, base_title: &str, cx: &App) -> Option<usize>;

    /// 领域在打开 Tab 时建立的订阅交给宿主保管，随宿主一起释放。
    fn keep_subscription(&mut self, subscription: Subscription);

    /// 解析连接引用的凭据；缺失钥匙串时由宿主给出提示并生成临时参数。
    fn resolve_connection_credentials(
        &mut self,
        connection: &StoredConnection,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<StoredConnection>;

    /// 应用级端口转发运行时；端口转发 Tab 与宿主共享同一组隧道。
    fn port_forwarding_runtime(&self) -> Option<Arc<tokio::sync::Mutex<PortForwardingRuntime>>>;

    fn notify(&mut self, level: NotifyLevel, message: &str, window: &mut Window, cx: &mut App);
}
