//! Native resource workbench primitives.
//!
//! Renderer state only: 页面只保存导航/加载/结果状态和会话 handle,
//! 所有 provider I/O 经 `extension_plugin_adapter::workbench_dispatch`
//! 的命名操作入口,连接主会话由宿主 tab 唯一持有。

mod collection_table;
pub mod custom_page_host;
pub mod layout;
mod message_event;
mod nav_tree;
pub mod query_page;
pub mod route_binding;
pub mod terminal_host;

pub use layout::{
    BottomContent, CenterContent, NavContent, RegionId, ResolvedLayout, ResolvedTreeRoot,
    SideContent, TreeChildRow,
};

pub use custom_page_host::{
    CustomPageHost, DirtyGuardDecision, GlobalCustomPageHost, MountHandle, PageRenderer,
    ShellMountError, ShellPageMount, ShellPageMountRequest, custom_page_host, resolve_renderer,
};
pub use terminal_host::{
    GlobalTerminalHost, TerminalHost, TerminalMount, TerminalMountError, TerminalMountRequest,
    interpolate_route, interpolate_with_session, terminal_host,
};

use collection_table::{CollectionTableDelegate, RowActionView, build_table_state};
use extension_plugin_adapter::{
    BindingContext, EventStreamBatch, ResourceSessionHandle, WorkbenchDispatchError,
    dispatch_invoke_result_scoped, dispatch_invoke_scoped as dispatch_invoke,
    dispatch_job_scoped as dispatch_job,
};
use extension_runtime::RegisteredResourceWorkbenchContribution;
use extension_runtime::extension::manifest::{
    ResourceWorkbenchForm, ResourceWorkbenchInputType, ResourceWorkbenchPage,
    ResourceWorkbenchPaginationKind, ResourceWorkbenchPrimitive,
    ResourceWorkbenchStatusFormat as F, ResourceWorkbenchTable, ResourceWorkbenchTerminal,
    ResourceWorkbenchViewer, ResourceWorkbenchViewerFormat,
};
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, Size, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    spinner::Spinner,
    table::{DataTable, TableState},
    tag::Tag,
    v_flex,
};
use message_event::MessageEvent;
use nav_tree::TreeChildrenState;

struct ActiveShellMount {
    key: String,
    host: std::rc::Rc<dyn CustomPageHost>,
    mount: ShellPageMount,
}

/// 已挂载的终端页面:按 (页面, 路由) 定位,路由变化时重挂。
struct ActiveTerminalMount {
    page_id: String,
    route_key: String,
    host: std::rc::Rc<dyn TerminalHost>,
    mount: TerminalMount,
}

/// 单个页面的加载状态。
pub enum PageState {
    Idle,
    Loading,
    Loaded(serde_json::Value),
    Failed(String),
}

/// 待确认的写操作:query 页 execute 或 collection 行操作。
#[derive(Debug, Clone)]
enum PendingConfirm {
    Query(String),
    RowAction {
        operation: String,
        row: serde_json::Value,
    },
}

/// 正在执行的行操作:定位到具体一行,只在该行的按钮上显示进行态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunningRowAction {
    pub operation: String,
    pub row_key: String,
}

/// collection 页表格的缓存:按 (页面, 页面世代, 加载态) 重建,
/// 避免每次重绘重建实体,同时保证 load 完成时确实刷新成新数据。
struct CollectionTableView {
    page_id: String,
    generation: u64,
    loading: bool,
    table: Entity<TableState<CollectionTableDelegate>>,
}

/// 挂载计数器,用于丢弃迟到结果(旧页面/旧请求的返回不得覆盖新状态)。
static NEXT_MOUNT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// events 页单帧最多渲染的事件行数。
///
/// 保留缓冲可以到 1000 条(`MAX_RETAINED_EVENTS`),但一帧渲染 1000 行会拖慢
/// 高频事件流(每秒数次通知)的重绘;最近的 500 条对「盯实时」已经足够。
const MAX_RENDERED_EVENTS: usize = 500;

/// 页面 stack 中的 table 原语(取首个)。
fn table_of(page: &ResourceWorkbenchPage) -> Option<&ResourceWorkbenchTable> {
    page.stack.iter().find_map(|primitive| match primitive {
        ResourceWorkbenchPrimitive::Table(table) => Some(table),
        _ => None,
    })
}

/// 页面 stack 中的 form 原语(取首个)。
fn form_of(page: &ResourceWorkbenchPage) -> Option<&ResourceWorkbenchForm> {
    page.stack.iter().find_map(|primitive| match primitive {
        ResourceWorkbenchPrimitive::Form(form) => Some(form),
        _ => None,
    })
}

/// 页面 stack 中的 viewer 原语(取首个)。
fn viewer_of(page: &ResourceWorkbenchPage) -> Option<&ResourceWorkbenchViewer> {
    page.stack.iter().find_map(|primitive| match primitive {
        ResourceWorkbenchPrimitive::Viewer(viewer) => Some(viewer),
        _ => None,
    })
}

/// 页面是否声明 viewer 的纯文本呈现(`format: text`)。
///
/// 缺少这个判断时 viewer 页面一律走 JSON 树视图,声明的 `text` 被静默忽略,
/// 属于"schema 承诺 > renderer 兑现"。
fn viewer_prefers_text(page: &ResourceWorkbenchPage) -> bool {
    viewer_of(page).is_some_and(|viewer| viewer.format == ResourceWorkbenchViewerFormat::Text)
}

/// 页面 stack 中的 terminal 原语(取首个)。
fn terminal_of(page: &ResourceWorkbenchPage) -> Option<&ResourceWorkbenchTerminal> {
    page.stack.iter().find_map(|primitive| match primitive {
        ResourceWorkbenchPrimitive::Terminal(terminal) => Some(terminal),
        _ => None,
    })
}

/// 页面是否为终端页(含 terminal 原语)。
fn is_terminal_page(page: &ResourceWorkbenchPage) -> bool {
    terminal_of(page).is_some()
}

/// 页面是否含 stream 原语(消费 load 返回的事件流)。
fn has_stream(page: &ResourceWorkbenchPage) -> bool {
    page.stack
        .iter()
        .any(|primitive| matches!(primitive, ResourceWorkbenchPrimitive::Stream))
}

/// 页面是否含 tasks 原语。
fn has_tasks(page: &ResourceWorkbenchPage) -> bool {
    page.stack
        .iter()
        .any(|primitive| matches!(primitive, ResourceWorkbenchPrimitive::Tasks))
}

pub struct NativeResourceWorkbench {
    descriptor: RegisteredResourceWorkbenchContribution,
    session: ResourceSessionHandle,
    layout: ResolvedLayout,
    selected_page: String,
    route: serde_json::Value,
    paging: serde_json::Value,
    /// 连接的**非敏感**配置(`ExtensionConnectionParams::config` 原样),
    /// 供 `source: connection` 绑定取值。
    ///
    /// 由宿主在创建连接 tab 时注入:工作台自身拿不到连接记录,也不该自己去查。
    /// 取 `config` 而不是整条 `StoredConnection`,是因为 `config` 与 `secrets`
    /// 按 `ExtensionConnectionParams::validate()` 不可能有同名键——密码/token
    /// 天然不在这里,不需要靠"过滤敏感键"这种会漏的黑名单。没有连接上下文时
    /// 是 `Null`。
    connection: serde_json::Value,
    page_state: PageState,
    /// 页面实例世代:每次导航或重新加载都推进。
    ///
    /// 所有异步回写先比对它,不匹配即丢弃。**取消请求不能代替归属检查**:
    /// 取消与回调真正到达之间有窗口期,迟到的回调会把旧页面的结果写进新页面
    /// (结果串页),或者把新页面的运行态留在原地(Run 一直转)。
    ///
    /// 推进点必须在**导航入口**而不是 `load_current_page` 里:目标页没有 `load`
    /// 时后者会提前返回,世代不变,旧页面的迟到回调就有了写进新页面的机会。
    page_generation: u64,
    focus_handle: FocusHandle,
    tokio: tokio::runtime::Handle,
    /// query 页面输入状态(按页面 id 保存,切换页面不丢失草稿)。
    query_inputs: std::collections::BTreeMap<String, Entity<query_page::QueryInputState>>,
    /// query 页面执行状态。
    query_result: Option<Result<serde_json::Value, String>>,
    query_running: bool,
    /// json 模板页面(overview/detail)的 JSON 值视图,按页面 id 保存。
    json_views: std::collections::BTreeMap<String, Entity<json_view::JsonValueView>>,
    /// 待用户确认的危险操作 id(native 页面确认条)。
    pending_confirm: Option<PendingConfirm>,
    /// 行操作执行状态:正在执行的 operation 与行标识(渲染为该行按钮的进行态)。
    row_action_running: Option<RunningRowAction>,
    /// 行操作失败信息(渲染在页面头部下方)。
    row_action_error: Option<String>,
    /// collection 页表格缓存。
    collection_table: Option<CollectionTableView>,
    shell_mount: Option<ActiveShellMount>,
    /// 区域级 Shell 挂载(right/bottom 等常驻区域,按 RegionId 缓存)。
    region_shell_mounts: std::collections::BTreeMap<&'static str, ActiveShellMount>,
    /// 左侧树数据缓存:节点键(根 id / 父键+行键)→ 子节点数据。
    ///
    /// 与 `tree_expanded` 分开是刻意的:折叠只改展开集合,**不删缓存**,
    /// 否则"展开 → 折叠 → 展开"会重复请求同一层。
    tree_children: std::collections::BTreeMap<String, TreeChildrenState>,
    /// 左侧树展开集合。只表达"这层是不是打开的",不含数据。
    tree_expanded: std::collections::BTreeSet<String>,
    /// 树节点级加载代次:节点键 → 当前在途轮次。旧轮次的迟到响应比不中就丢弃。
    ///
    /// 缓存不再随折叠失效,所以代次是唯一的作废手段(刷新时整表清空)。
    tree_load_seq: std::collections::BTreeMap<String, u64>,
    tree_load_generation: u64,
    /// 刷新后待恢复的展开键。节点键是结构化的,清空缓存后深层键要等父层
    /// 重新加载完才能解析,所以恢复是分批的(见 `nav_tree::resume_tree_expansions`)。
    tree_pending_expand: std::collections::BTreeSet<String>,
    /// terminal 模板页面已挂载的终端。
    terminal_mount: Option<ActiveTerminalMount>,
    terminal_error: Option<String>,
    /// 进入终端页前的来源 (page_id, route),供终端控制台的返回按钮使用。
    terminal_return: Option<(String, serde_json::Value)>,
    /// 底部状态栏数据(由 `statusBar.operation` 提供)。
    status_bar: Option<Result<serde_json::Value, String>>,
    status_bar_loading: bool,
    renderer_error: Option<String>,
    task_error: Option<String>,
    /// 当前**页面级**请求的取消令牌(load/query/行操作共用一个槽位)。
    ///
    /// 页面导航会取消它——这几个请求的结果按页面归属。状态栏是工作台级的,
    /// 用独立的 `status_bar_cancel`,否则切页会把还没回来的状态栏请求一起掐掉。
    active_request_cancel: Option<extension_host::CancellationToken>,
    /// 状态栏请求的取消令牌:只在重新拉取与视图销毁时取消,导航不碰。
    status_bar_cancel: Option<extension_host::CancellationToken>,
    /// 状态栏世代:手动刷新可以连点,迟到的回调不能覆盖新结果。
    status_bar_generation: u64,
    event_batches: Vec<serde_json::Value>,
    event_dropped: u64,
    event_closed: bool,
    event_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageSelection {
    pub page_id: String,
    pub route: serde_json::Value,
}

/// 渲染用的页面状态快照(避免渲染期间持有 self 借用)。
enum PageStateSnapshot {
    Idle,
    Loading,
    Loaded(serde_json::Value),
    Failed(String),
}

/// query 页面结果区应该展示什么。
#[derive(Debug, Clone, PartialEq)]
enum QueryResultSource {
    /// 用户本次执行的结果(失败也是结果)。
    Query(Result<serde_json::Value, String>),
    /// 页面 `load` 的结果:声明了 `load` 的查询页打开即应有初始结果集。
    PageLoad(Result<serde_json::Value, String>),
    /// 初始结果还在路上。
    Loading,
    /// 还没有任何结果可展示。
    Empty,
}

/// 决定 query 页面结果区的来源。
///
/// 用户执行的结果优先;没执行过且页面声明了 `load` 时,回落到 `load` 的结果。
///
/// 没有 `load` 的页面**必须**保持空状态:两个结果存在不同字段里
/// (`query_result` / `page_state`),而 `begin_page_transition` 只清前者不清后者 ——
/// 不卡 `has_load` 这一道,打开一个没有 `load` 的查询页就会看到上一个页面的残留结果。
fn query_result_source(
    query_result: &Option<Result<serde_json::Value, String>>,
    has_load: bool,
    page_state: &PageStateSnapshot,
) -> QueryResultSource {
    if let Some(result) = query_result {
        return QueryResultSource::Query(result.clone());
    }
    if !has_load {
        return QueryResultSource::Empty;
    }
    match page_state {
        PageStateSnapshot::Loaded(value) => QueryResultSource::PageLoad(Ok(value.clone())),
        PageStateSnapshot::Failed(error) => QueryResultSource::PageLoad(Err(error.clone())),
        // Idle 说明首屏那次 load 还没发生;Loading 说明正在进行。
        PageStateSnapshot::Loading => QueryResultSource::Loading,
        PageStateSnapshot::Idle => QueryResultSource::Empty,
    }
}

impl NativeResourceWorkbench {
    fn page_state_snapshot(&self) -> PageStateSnapshot {
        match &self.page_state {
            PageState::Idle => PageStateSnapshot::Idle,
            PageState::Loading => PageStateSnapshot::Loading,
            PageState::Loaded(value) => PageStateSnapshot::Loaded(value.clone()),
            PageState::Failed(error) => PageStateSnapshot::Failed(error.clone()),
        }
    }

    /// `connection` 是宿主解析出的**非敏感**连接配置(见字段文档)。
    ///
    /// 做成构造函数参数而不是后来的 setter:构造函数内部就会发起首次
    /// `load`,若连接上下文晚一步注入,首屏那次调用拿到的就是 `Null`。
    pub fn new(
        descriptor: RegisteredResourceWorkbenchContribution,
        session: ResourceSessionHandle,
        connection: serde_json::Value,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected_page = descriptor.default_page.clone();
        let layout = ResolvedLayout::resolve(&descriptor);
        let mut this = Self {
            descriptor,
            session,
            layout,
            selected_page,
            route: serde_json::Value::Null,
            paging: serde_json::json!({"page": 1, "limit": 50, "cursor": null}),
            connection,
            page_state: PageState::Idle,
            page_generation: 0,
            focus_handle: cx.focus_handle(),
            tokio: one_core::gpui_tokio::Tokio::handle(cx),
            query_inputs: Default::default(),
            query_result: None,
            query_running: false,
            json_views: Default::default(),
            pending_confirm: None,
            row_action_running: None,
            row_action_error: None,
            collection_table: None,
            shell_mount: None,
            region_shell_mounts: Default::default(),
            tree_children: Default::default(),
            tree_expanded: Default::default(),
            tree_load_seq: Default::default(),
            tree_load_generation: 0,
            tree_pending_expand: Default::default(),
            terminal_mount: None,
            terminal_error: None,
            terminal_return: None,
            status_bar: None,
            status_bar_loading: false,
            renderer_error: None,
            task_error: None,
            active_request_cancel: None,
            status_bar_cancel: None,
            status_bar_generation: 0,
            event_batches: Vec::new(),
            event_dropped: 0,
            event_closed: false,
            event_error: None,
            _subscriptions: Vec::new(),
        };
        this._subscriptions
            .push(cx.on_release(|this, cx| this.dispose_all_shell_mounts(cx)));
        this._subscriptions
            .push(cx.on_release(|this, cx| this.dispose_terminal_mount(cx)));
        this._subscriptions.push(cx.on_release(|this, _cx| {
            this.cancel_active_request();
            this.cancel_status_bar_request();
        }));
        this.load_current_page(cx);
        this.load_status_bar(cx);
        this
    }

    pub fn selection(&self) -> PageSelection {
        PageSelection {
            page_id: self.selected_page.clone(),
            route: self.route.clone(),
        }
    }

    fn paging_context(&self) -> serde_json::Value {
        self.paging.clone()
    }

    fn cancel_active_request(&mut self) {
        if let Some(cancel) = self.active_request_cancel.take() {
            cancel.cancel();
        }
    }

    fn cancel_status_bar_request(&mut self) {
        if let Some(cancel) = self.status_bar_cancel.take() {
            cancel.cancel();
        }
    }

    /// 导航时的取值来源:route 取当前页,selection/parent 由调用点给出,
    /// connection 一律来自宿主注入的连接上下文。
    ///
    /// 集中在这里构造而不是各调用点自己拼 `BindingContext`/`RouteSources`:
    /// 漏一个来源不会报错,只会静默丢弃(`source: connection` 就是这么在
    /// 路由侧被忽略过一轮)。
    fn route_sources<'a>(
        &'a self,
        selection: &'a serde_json::Value,
        parent: &'a serde_json::Value,
    ) -> route_binding::RouteSources<'a> {
        route_binding::RouteSources {
            route: &self.route,
            selection,
            parent,
            connection: &self.connection,
        }
    }

    /// 没有行上下文的导航(links / tabs):只透传 route 与 connection。
    fn route_sources_without_row(&self) -> route_binding::RouteSources<'_> {
        route_binding::RouteSources::without_row_context(&self.route, &self.connection)
    }

    /// 推进页面世代并复位瞬态状态。**每个导航入口都要调用**。
    ///
    /// 世代必须在这里无条件推进,不能指望 `load_current_page`:目标页没有
    /// `load` 时它会提前返回(不推进世代),旧页面的迟到回调就会写进新页面。
    fn begin_page_transition(&mut self) {
        self.page_generation = self.page_generation.wrapping_add(1);
        self.cancel_active_request();
        self.event_batches.clear();
        self.event_dropped = 0;
        self.event_closed = false;
        self.event_error = None;
        self.query_result = None;
        // 运行态是**页面级**瞬态:旧页面的请求已被上面的取消打断,它的回调
        // 因为世代不匹配不会再来清这个标志,必须在这里复位,否则回到该页
        // 会一直显示"运行中"。
        self.query_running = false;
        self.json_views.clear();
        self.collection_table = None;
        self.pending_confirm = None;
        self.row_action_running = None;
        self.row_action_error = None;
        self.terminal_error = None;
    }

    fn collection_page(&self, page: &ResourceWorkbenchPage) -> Option<u64> {
        table_of(page)
            .filter(|table| table.pagination.kind != ResourceWorkbenchPaginationKind::None)
            .and_then(|_| self.paging.get("page").and_then(serde_json::Value::as_u64))
    }

    /// cursor 分页:上次 load 返回的 nextCursor;None 表示页码式或没有更多。
    fn collection_cursor(&self, page: &ResourceWorkbenchPage) -> Option<String> {
        let table = table_of(page)?;
        if table.pagination.kind != ResourceWorkbenchPaginationKind::Cursor {
            return None;
        }
        self.paging
            .get("cursor")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }

    fn change_collection_page(&mut self, delta: i64, cx: &mut Context<Self>) {
        let Some(page) = self.current_page().cloned() else {
            return;
        };
        let Some(current) = self.collection_page(&page) else {
            return;
        };
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as u64)
        };
        if next == 0 || next == current {
            return;
        }
        let is_cursor_kind = self.collection_cursor(&page).is_some()
            || table_of(&page).is_some_and(|table| {
                table.pagination.kind == ResourceWorkbenchPaginationKind::Cursor
            });
        let existing_cursor = self.paging.get("cursor").cloned();
        if let Some(object) = self.paging.as_object_mut() {
            object.insert("page".into(), serde_json::json!(next));
            // cursor 分页向后翻沿用上次返回的 nextCursor;向前翻回到起点。
            let cursor = if delta.is_negative() || !is_cursor_kind {
                serde_json::Value::Null
            } else {
                existing_cursor.unwrap_or(serde_json::Value::Null)
            };
            object.insert("cursor".into(), cursor);
        }
        self.load_current_page(cx);
    }

    /// 从 collection load 结果读回 nextCursor,推进 cursor 分页状态。
    fn advance_collection_cursor(&mut self, value: &serde_json::Value) {
        let Some(page) = self.current_page() else {
            return;
        };
        if table_of(page)
            .is_none_or(|table| table.pagination.kind != ResourceWorkbenchPaginationKind::Cursor)
        {
            return;
        }
        let next_cursor = ["/nextCursor", "/pageInfo/nextCursor", "/cursor"]
            .iter()
            .find_map(|path| value.pointer(path))
            .and_then(|cursor| cursor.as_str())
            .map(str::to_owned);
        if let Some(object) = self.paging.as_object_mut() {
            object.insert(
                "cursor".into(),
                match next_cursor {
                    Some(cursor) => serde_json::json!(cursor),
                    None => serde_json::Value::Null,
                },
            );
        }
    }

    /// 进入终端页前记录来源页;离开终端页时清除。
    /// 终端控制台隐藏了侧边栏,返回按钮是唯一的原路返回入口。
    fn track_terminal_return(&mut self, target_page_id: &str) {
        let target_is_terminal = self
            .descriptor
            .pages
            .iter()
            .any(|page| page.id == target_page_id && is_terminal_page(page));
        if target_is_terminal {
            let already_terminal = self.current_page().map(is_terminal_page).unwrap_or(false);
            if !already_terminal {
                self.terminal_return = Some((self.selected_page.clone(), self.route.clone()));
            }
        } else {
            self.terminal_return = None;
        }
    }

    pub fn select_page(&mut self, page_id: impl Into<String>, cx: &mut Context<Self>) {
        let page_id = page_id.into();
        if !self.descriptor.pages.iter().any(|page| page.id == page_id) {
            return;
        }
        self.track_terminal_return(&page_id);
        self.begin_page_transition();
        self.dispose_shell_mount(cx);
        self.dispose_terminal_mount(cx);
        self.selected_page = page_id;
        self.route = serde_json::Value::Null;
        self.paging = serde_json::json!({"page": 1, "limit": 50, "cursor": null});
        self.load_current_page(cx);
    }

    pub fn navigate(
        &mut self,
        page_id: impl Into<String>,
        route: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let page_id = page_id.into();
        if !self.descriptor.pages.iter().any(|page| page.id == page_id) {
            return;
        }
        self.track_terminal_return(&page_id);
        self.begin_page_transition();
        self.dispose_shell_mount(cx);
        self.dispose_terminal_mount(cx);
        self.selected_page = page_id;
        self.route = route;
        self.paging = serde_json::json!({"page": 1, "limit": 50, "cursor": null});
        self.load_current_page(cx);
    }

    fn current_page(&self) -> Option<&ResourceWorkbenchPage> {
        self.descriptor
            .pages
            .iter()
            .find(|page| page.id == self.selected_page)
    }

    /// 回收页面级 Shell 挂载;区域级挂载(右侧栏等)跨页面常驻。
    fn dispose_shell_mount(&mut self, cx: &mut App) {
        if let Some(active) = self.shell_mount.take() {
            active.host.dispose(active.mount, cx);
        }
        self.renderer_error = None;
    }

    /// 回收全部 Shell 挂载(页面级 + 区域级);仅工作台销毁时调用。
    fn dispose_all_shell_mounts(&mut self, cx: &mut App) {
        self.dispose_shell_mount(cx);
        for (_, active) in std::mem::take(&mut self.region_shell_mounts) {
            active.host.dispose(active.mount, cx);
        }
    }

    /// 回收终端页面挂载:只释放终端实体,不触碰连接主会话。
    fn dispose_terminal_mount(&mut self, cx: &mut App) {
        if let Some(active) = self.terminal_mount.take() {
            active.host.dispose(active.mount, cx);
        }
    }

    /// 当前路由的稳定键,用于判断终端是否需按路由变化重挂。
    fn route_key(&self) -> String {
        serde_json::to_string(&self.route).unwrap_or_default()
    }

    /// 按 manifest 的 terminal 声明启动终端并挂载。
    /// 已挂载且 (页面, 路由) 未变时复用现有终端,避免每次重绘重启进程。
    fn mount_terminal_page(
        &mut self,
        page: &ResourceWorkbenchPage,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui::AnyView> {
        let Some(declaration) = terminal_of(page) else {
            self.terminal_error = Some("page declares no terminal primitive".into());
            return None;
        };
        let Some(host) = terminal_host(cx) else {
            self.terminal_error = Some("terminal component is unavailable in this build".into());
            return None;
        };
        let route_key = self.route_key();
        if let Some(active) = &self.terminal_mount {
            if active.page_id == page.id && active.route_key == route_key {
                return Some(active.mount.view.clone());
            }
        }
        self.dispose_terminal_mount(cx);
        // session metadata 由 provider 在 resource/open 时声明,terminal 声明可
        // 通过 {{session.docker_host}} 等占位符引用,保证与查询/操作同一目标。
        let session_metadata = self.session.metadata().unwrap_or(serde_json::Value::Null);
        let request = TerminalMountRequest {
            title: page.title.clone(),
            command: declaration
                .command
                .as_deref()
                .map(|command| interpolate_with_session(command, &self.route, &session_metadata)),
            args: declaration
                .args
                .iter()
                .map(|arg| interpolate_with_session(arg, &self.route, &session_metadata))
                .collect(),
            env: declaration
                .env
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        interpolate_with_session(value, &self.route, &session_metadata),
                    )
                })
                .collect(),
            working_dir: declaration
                .working_dir
                .as_deref()
                .map(|dir| interpolate_with_session(dir, &self.route, &session_metadata)),
            operation: declaration
                .operation
                .as_ref()
                .map(|operation| operation.operation.clone()),
        };
        match host.mount(request, window, cx) {
            Ok(mount) => {
                let view = mount.view.clone();
                self.terminal_error = None;
                self.terminal_mount = Some(ActiveTerminalMount {
                    page_id: page.id.clone(),
                    route_key,
                    host,
                    mount,
                });
                Some(view)
            }
            Err(error) => {
                self.terminal_error = Some(error.to_string());
                None
            }
        }
    }

    /// 拉取底部状态栏数据(bottom status source 的 operation)。
    fn load_status_bar(&mut self, cx: &mut Context<Self>) {
        let Some(BottomContent::Status { operation, .. }) =
            self.layout.bottom.as_ref().map(|bottom| &bottom.content)
        else {
            return;
        };
        let operation = operation.clone();
        if !self.descriptor.operations.contains_key(&operation) {
            self.status_bar = Some(Err(format!("unknown status operation `{operation}`")));
            cx.notify();
            return;
        }
        self.status_bar_loading = true;
        // 世代在发请求**之前**推进:上一次请求的迟到回调据此判定已被顶替。
        self.status_bar_generation = self.status_bar_generation.wrapping_add(1);
        let status_generation = self.status_bar_generation;
        cx.notify();

        let mount_id = NEXT_MOUNT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let scope = self.session.scope("__status_bar__".to_string(), mount_id);
        let workbench = self.descriptor.clone();
        let context = BindingContext {
            input: serde_json::Value::Null,
            // 状态栏跨页面常驻,不重挂也不随导航重拉 ⇒ 任何一页的 route 都不是
            // 它的权威上下文,保持 Null;`connection` 则与页面无关,照常提供。
            route: serde_json::Value::Null,
            selection: serde_json::Value::Null,
            paging: serde_json::json!({"page": 1, "limit": 50, "cursor": null}),
            parent: serde_json::Value::Null,
            connection: self.connection.clone(),
        };
        self.cancel_status_bar_request();
        self.status_bar_cancel = Some(scope.cancellation());
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = tokio
                .spawn(async move {
                    dispatch_invoke(&scope, &workbench, &operation, &context, false).await
                })
                .await
                .unwrap_or_else(|join_error| {
                    Err(WorkbenchDispatchError::Provider(join_error.to_string()))
                });
            let _ = this.update(cx, |this, cx| {
                if this.status_bar_generation != status_generation {
                    return;
                }
                this.status_bar_loading = false;
                this.status_bar = Some(result.map_err(|error| error.to_string()));
                cx.notify();
            });
        })
        .detach();
    }

    fn mount_shell_page(
        &mut self,
        page: &ResourceWorkbenchPage,
        view_id: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui::AnyView> {
        let key = format!("page::{}", page.id);
        if let Some(active) = &self.shell_mount {
            if active.key == key {
                return Some(active.mount.view.clone());
            }
        }
        self.dispose_shell_mount(cx);
        let Some(host) = custom_page_host(cx) else {
            self.renderer_error = Some("Shell renderer is unavailable in this build".into());
            return None;
        };
        let request = ShellPageMountRequest {
            extension_id: self.descriptor.extension_id.clone(),
            view_id: view_id.to_string(),
            page_context: serde_json::json!({
                "pageId": page.id,
                "route": self.route,
                // 与 `BindingContext.connection` 同源:Shell 的 navop.workbench.dispatch
                // 也按这份上下文解析 `source: connection`,两边必须是同一份值。
                "connection": self.connection,
                "capabilities": self.session.capabilities(),
            }),
            resource_type: self.descriptor.resource_type.clone(),
            session: Some(self.session.clone()),
            workbench: self.descriptor.clone(),
        };
        match host.mount(request, window, cx) {
            Ok(mount) => {
                let view = mount.view.clone();
                self.shell_mount = Some(ActiveShellMount { key, host, mount });
                Some(view)
            }
            Err(error) => {
                self.renderer_error = Some(error.to_string());
                None
            }
        }
    }

    /// 区域级 Shell 挂载(right/bottom/left/center 的 shell source):
    /// 按 RegionId 常驻缓存,页面导航不重挂;上下文带 regionId。
    fn mount_region_shell(
        &mut self,
        region: RegionId,
        view_id: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui::AnyView> {
        let key = region.as_str();
        if let Some(active) = self.region_shell_mounts.get(key) {
            return Some(active.mount.view.clone());
        }
        let host = custom_page_host(cx)?;
        let request = ShellPageMountRequest {
            extension_id: self.descriptor.extension_id.clone(),
            view_id: view_id.to_string(),
            page_context: serde_json::json!({
                "regionId": region.as_str(),
                "pageId": self.selected_page,
                "route": self.route,
                "connection": self.connection,
                "capabilities": self.session.capabilities(),
            }),
            resource_type: self.descriptor.resource_type.clone(),
            session: Some(self.session.clone()),
            workbench: self.descriptor.clone(),
        };
        let mount = host.mount(request, window, cx).ok()?;
        let view = mount.view.clone();
        self.region_shell_mounts.insert(
            key,
            ActiveShellMount {
                key: key.to_string(),
                host,
                mount,
            },
        );
        Some(view)
    }

    /// 触发当前页面的 load 操作。tasks 页面读宿主任务,不发 provider 请求。
    fn load_current_page(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.current_page() else {
            self.page_state = PageState::Failed("unknown workbench page".into());
            cx.notify();
            return;
        };
        let Some(action) = page.load.clone() else {
            self.page_state = PageState::Idle;
            cx.notify();
            return;
        };
        let is_stream = has_stream(page);
        // 重新加载同一页时也要推进世代:行操作成功后会回到这里刷新列表,
        // 上一次 load 的迟到回调必须被作废。
        self.page_generation = self.page_generation.wrapping_add(1);
        let generation = self.page_generation;
        // 同一次推进也作废了本页在飞的 query —— 它的回调会被世代检查丢掉,
        // 不会再来清运行态,所以在这里补上,否则"运行中"会永远转下去。
        self.query_running = false;
        self.page_state = PageState::Loading;
        cx.notify();

        let mount_id = NEXT_MOUNT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let scope = self.session.scope(self.selected_page.clone(), mount_id);
        self.active_request_cancel = Some(scope.cancellation());
        let workbench = self.descriptor.clone();
        let context = BindingContext {
            input: serde_json::Value::Null,
            route: self.route.clone(),
            selection: serde_json::Value::Null,
            paging: self.paging_context(),
            parent: serde_json::Value::Null,
            connection: self.connection.clone(),
        };
        let operation = action.operation;
        let tokio = self.tokio.clone();
        if is_stream {
            let this_scope = scope;
            let this = cx.entity().downgrade();
            let workbench_for_event = workbench.clone();
            let context_for_event = context.clone();
            // provider dispatch 必须落在应用 Tokio runtime 上:直接在这里 await
            // 会让 client 内部的 tokio::time::timeout 在 GPUI foreground executor
            // 上构造 Sleep 而 panic("no reactor running")。
            let dispatch_scope = this_scope.clone();
            cx.spawn(async move |_, cx| {
                let result = tokio
                    .spawn(async move {
                        dispatch_invoke_result_scoped(
                            &dispatch_scope,
                            &workbench_for_event,
                            &operation,
                            &context_for_event,
                            false,
                        )
                        .await
                    })
                    .await
                    .unwrap_or_else(|join_error| {
                        Err(WorkbenchDispatchError::Provider(join_error.to_string()))
                    });
                let stream_id = match result {
                    Ok(extension_protocol::resource::ResourceInvokeResult {
                        result: extension_protocol::result_ref::ResultRef::EventStream { id },
                    }) => id,
                    Ok(_) => {
                        let _ = this.update(cx, |this, cx| {
                            if this.page_generation != generation {
                                return;
                            }
                            this.event_error =
                                Some("events operation did not return an event stream".into());
                            this.page_state = PageState::Failed(
                                "events operation did not return an event stream".into(),
                            );
                            cx.notify();
                        });
                        return;
                    }
                    Err(error) => {
                        let _ = this.update(cx, |this, cx| {
                            if this.page_generation != generation {
                                return;
                            }
                            this.event_error = Some(error.to_string());
                            this.page_state = PageState::Failed(error.to_string());
                            cx.notify();
                        });
                        return;
                    }
                };
                let stream = extension_protocol::event_stream::EventOpenResult { stream_id };
                // subscribe_events 内部 tokio::spawn 出 pull loop,同样必须在应用
                // Tokio runtime 上调用,否则 spawn 会在 GPUI foreground 上 panic。
                let subscribe_scope = this_scope.clone();
                let subscription = tokio
                    .spawn(async move {
                        subscribe_scope.subscribe_events(
                            &stream,
                            extension_plugin_adapter::EventStreamSubscriptionConfig::default(),
                        )
                    })
                    .await;
                let mut subscription = match subscription {
                    Ok(subscription) => subscription,
                    Err(join_error) => {
                        let _ = this.update(cx, |this, cx| {
                            if this.page_generation != generation {
                                return;
                            }
                            this.event_error = Some(join_error.to_string());
                            this.page_state = PageState::Failed(join_error.to_string());
                            cx.notify();
                        });
                        return;
                    }
                };
                while let Some(batch) = subscription.recv().await {
                    let mut stale = false;
                    let _ = this.update(cx, |this, cx| {
                        // 事件流的消费循环是本次 load 启动的;**切页不会自动让
                        // provider 停止推送**,所以每一批都要判归属,不能只判
                        // 第一批。不匹配就跳出循环释放订阅。
                        if this.page_generation != generation {
                            stale = true;
                            return;
                        }
                        match batch {
                            Ok(EventStreamBatch {
                                events,
                                dropped_count,
                                closed,
                            }) => {
                                this.event_batches.extend(events);
                                const MAX_RETAINED_EVENTS: usize = 1000;
                                if this.event_batches.len() > MAX_RETAINED_EVENTS {
                                    let overflow = this.event_batches.len() - MAX_RETAINED_EVENTS;
                                    this.event_batches.drain(..overflow);
                                    this.event_dropped += overflow as u64;
                                }
                                this.event_dropped += dropped_count;
                                this.event_closed = closed;
                            }
                            Err(error) => this.event_error = Some(error.to_string()),
                        }
                        cx.notify();
                    });
                    if stale {
                        return;
                    }
                }
            })
            .detach();
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = tokio
                .spawn(async move {
                    dispatch_invoke(&scope, &workbench, &operation, &context, false).await
                })
                .await
                .unwrap_or_else(|join_error| {
                    Err(WorkbenchDispatchError::Provider(join_error.to_string()))
                });
            let _ = this.update(cx, |this, cx| {
                if this.page_generation != generation {
                    return;
                }
                this.page_state = match result {
                    Ok(value) => {
                        this.advance_collection_cursor(&value);
                        PageState::Loaded(value)
                    }
                    Err(error) => PageState::Failed(error.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// 执行 query 页面的命名操作(job 或 invoke)。
    fn execute_query(&mut self, cx: &mut Context<Self>) {
        if self.query_running {
            return;
        }
        let Some(page) = self.current_page().cloned() else {
            return;
        };
        let Some(action) = form_of(&page).map(|form| form.submit.clone()) else {
            return;
        };
        if !self.descriptor.operations.contains_key(&action.operation) {
            self.query_result = Some(Err("unknown operation".into()));
            cx.notify();
            return;
        };
        // 危险操作先请求确认;确认通过后带 confirmed=true 重新执行。
        let effect =
            extension_plugin_adapter::operation_effect(&self.descriptor, &action.operation);
        if effect.is_some_and(extension_plugin_adapter::requires_confirmation) {
            self.pending_confirm = Some(PendingConfirm::Query(action.operation.clone()));
            cx.notify();
            return;
        }
        self.run_query(action.operation, &page, false, cx);
    }

    /// 用户确认危险操作后执行。
    fn confirm_pending(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_confirm.take() else {
            return;
        };
        match pending {
            PendingConfirm::Query(operation) => {
                let Some(page) = self.current_page().cloned() else {
                    return;
                };
                if !form_of(&page).is_some_and(|form| form.submit.operation == operation) {
                    cx.notify();
                    return;
                }
                self.run_query(operation, &page, true, cx);
            }
            PendingConfirm::RowAction { operation, row } => {
                self.run_row_action(operation, row, true, cx);
            }
        }
    }

    fn cancel_pending(&mut self, cx: &mut Context<Self>) {
        self.pending_confirm = None;
        cx.notify();
    }

    fn run_query(
        &mut self,
        operation: String,
        page: &extension_runtime::extension::manifest::ResourceWorkbenchPage,
        confirmed: bool,
        cx: &mut Context<Self>,
    ) {
        let is_job = self
            .descriptor
            .operations
            .get(&operation)
            .is_some_and(|op| {
                matches!(
                    op.mode,
                    extension_runtime::extension::manifest::ResourceWorkbenchOperationMode::Job
                )
            });
        // 组装 input 值:从输入 state 读取声明字段的当前值。
        let mut input = serde_json::Map::new();
        let input_state = match self.query_inputs.get(&self.selected_page) {
            Some(state) => state.clone(),
            None => {
                self.query_result = Some(Err("query inputs are not initialized".into()));
                cx.notify();
                return;
            }
        };
        let values = input_state.read(cx).values(cx);
        let Some(form) = form_of(page) else {
            self.query_result = Some(Err("page has no form primitive".into()));
            cx.notify();
            return;
        };
        for field in &form.inputs {
            let text = values.get(&field.id).cloned().unwrap_or_default();
            let trimmed = text.trim();
            if field.required && trimmed.is_empty() {
                self.query_result = Some(Err(format!("`{}` is required", field.id)));
                cx.notify();
                return;
            }
            // 字段声明的类型必须在这里兑现:`type: json` 的输入是一段**文本**,
            // 得解析成真 JSON 对象再交给 provider,否则对象契约拿到的是
            // `"{\"a\":1}"` 这种字符串。空的可选字段按类型分别处理 ——
            // 空串对 string 是合法值,对 number/boolean/json 不是,后者直接跳过
            // 该参数而不是塞一个必然转换失败的空串(required 且空的情况上面已返回)。
            if trimmed.is_empty() && field.value_type != ResourceWorkbenchInputType::String {
                continue;
            }
            let value = match extension_plugin_adapter::parse_form_input(
                &field.id,
                &text,
                field.value_type,
            ) {
                Ok(value) => value,
                Err(message) => {
                    self.query_result = Some(Err(message));
                    cx.notify();
                    return;
                }
            };
            input.insert(field.id.clone(), value);
        }

        self.query_running = true;
        self.query_result = None;
        cx.notify();

        self.page_generation = self.page_generation.wrapping_add(1);
        let generation = self.page_generation;
        let mount_id = NEXT_MOUNT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let scope = self.session.scope(self.selected_page.clone(), mount_id);
        self.active_request_cancel = Some(scope.cancellation());
        let workbench = self.descriptor.clone();
        let context = BindingContext {
            input: serde_json::Value::Object(input),
            route: self.route.clone(),
            selection: serde_json::Value::Null,
            paging: self.paging_context(),
            parent: serde_json::Value::Null,
            connection: self.connection.clone(),
        };
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = tokio
                .spawn(async move {
                    if is_job {
                        dispatch_job(&scope, &workbench, &operation, &context, confirmed).await
                    } else {
                        dispatch_invoke(&scope, &workbench, &operation, &context, confirmed).await
                    }
                })
                .await
                .unwrap_or_else(|join_error| {
                    Err(WorkbenchDispatchError::Provider(join_error.to_string()))
                });
            let _ = this.update(cx, |this, cx| {
                if this.page_generation != generation {
                    return;
                }
                this.query_running = false;
                this.query_result = Some(match result {
                    Ok(value) => Ok(value),
                    Err(error) => Err(error.to_string()),
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// collection 行点击:按 open 声明构造目标页 route 并导航。
    fn open_collection_row(&mut self, row: serde_json::Value, cx: &mut Context<Self>) {
        let Some(page) = self.current_page() else {
            return;
        };
        let Some(open) = table_of(page).and_then(|table| table.open.as_ref()) else {
            return;
        };
        let route = route_binding::build_route(
            &open.route,
            &self.route_sources(&row, &serde_json::Value::Null),
        );
        self.navigate(open.page_id.clone(), route, cx);
    }

    /// collection 行操作:以该行为 selection 执行命名操作,成功后刷新列表。
    /// 非 read effect 需要用户确认(与 query 页共用确认条)。
    fn run_row_action(
        &mut self,
        operation: String,
        row: serde_json::Value,
        confirmed: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.descriptor.operations.contains_key(&operation) {
            self.row_action_error = Some(format!("unknown operation `{operation}`"));
            cx.notify();
            return;
        }
        if !confirmed {
            let effect = extension_plugin_adapter::operation_effect(&self.descriptor, &operation);
            if effect.is_some_and(extension_plugin_adapter::requires_confirmation) {
                self.pending_confirm = Some(PendingConfirm::RowAction {
                    operation: operation.clone(),
                    row,
                });
                cx.notify();
                return;
            }
        }
        self.row_action_running = Some(RunningRowAction {
            operation: operation.clone(),
            row_key: self
                .current_page()
                .and_then(table_of)
                .map(|table| collection_table::row_key(&row, &table.key_paths))
                .unwrap_or_default(),
        });
        self.row_action_error = None;
        cx.notify();

        // 不给世代加一:行操作**不改变页面归属**,它只是当前页的一次动作,
        // 加一反而会让本次 load 的迟到回调被自己作废。归属由下面回调里的
        // `page_generation != generation` 判定 —— 任何导航/重载都会推进世代。
        let generation = self.page_generation;
        let page_id = self.selected_page.clone();
        let mount_id = NEXT_MOUNT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let scope = self.session.scope(page_id.clone(), mount_id);
        self.active_request_cancel = Some(scope.cancellation());
        let workbench = self.descriptor.clone();
        let context = BindingContext {
            input: serde_json::Value::Null,
            route: self.route.clone(),
            selection: row,
            paging: self.paging_context(),
            parent: serde_json::Value::Null,
            connection: self.connection.clone(),
        };
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = tokio
                .spawn(async move {
                    dispatch_invoke(&scope, &workbench, &operation, &context, true).await
                })
                .await
                .unwrap_or_else(|join_error| {
                    Err(WorkbenchDispatchError::Provider(join_error.to_string()))
                });
            let _ = this.update(cx, |this, cx| {
                // 页面已切换或已重新加载:丢弃迟到结果。
                if this.page_generation != generation || this.selected_page != page_id {
                    return;
                }
                this.row_action_running = None;
                match &result {
                    Ok(_) => {
                        this.row_action_error = None;
                        // 操作改变了 provider 状态,重新拉取列表。
                        this.load_current_page(cx);
                    }
                    Err(error) => {
                        this.row_action_error = Some(error.to_string());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// 左侧导航区域:按 layout.left 内容源分派(list/tree/shell/none)。
    fn render_nav(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let region = self.layout.left.clone()?;
        let width = region.width;
        let theme = cx.theme().clone();
        let frame = |list: gpui::Div| {
            list.w(px(width as f32))
                .h_full()
                .flex_shrink_0()
                .gap_1()
                .px_2()
                .py_3()
                .bg(theme.sidebar)
                .border_r_1()
                .border_color(theme.sidebar_border)
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_xs()
                        .font_medium()
                        .text_color(theme.muted_foreground)
                        .child("Pages"),
                )
        };
        match &region.content {
            NavContent::List { entries } => {
                let mut list = v_flex().w_full().gap_0p5();
                for (page_id, title) in entries {
                    let is_selected = page_id == &self.selected_page;
                    let page_id = page_id.clone();
                    list = list.child(
                        h_flex()
                            .id(page_id.clone())
                            .w_full()
                            .min_w_0()
                            .px_2()
                            .py_1()
                            .rounded(theme.radius)
                            .cursor_pointer()
                            .text_sm()
                            .text_color(if is_selected {
                                theme.sidebar_accent_foreground
                            } else {
                                theme.sidebar_foreground
                            })
                            .when(is_selected, |this| {
                                this.bg(theme.sidebar_accent).font_medium()
                            })
                            .when(!is_selected, |this| {
                                let hover = theme.list_hover;
                                this.hover(move |this| this.bg(hover))
                            })
                            .child(div().min_w_0().truncate().child(title.clone()))
                            .on_click(cx.listener(
                                move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                                    this.select_page(page_id.clone(), cx);
                                },
                            )),
                    );
                }
                Some(frame(v_flex()).child(list).into_any_element())
            }
            NavContent::Tree { .. } => {
                let tree = self.render_nav_tree(cx);
                Some(frame(v_flex()).child(tree).into_any_element())
            }
            NavContent::Shell { view_id } => {
                let view = self.mount_region_shell(RegionId::Left, view_id, window, cx)?;
                Some(
                    v_flex()
                        .w(px(region.width as f32))
                        .h_full()
                        .flex_shrink_0()
                        .overflow_hidden()
                        .bg(theme.sidebar)
                        .border_r_1()
                        .border_color(theme.sidebar_border)
                        .child(
                            div()
                                .id("nav-shell-region")
                                .size_full()
                                .min_w_0()
                                .min_h_0()
                                .child(view),
                        )
                        .into_any_element(),
                )
            }
            NavContent::None => None,
        }
    }

    fn render_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let Some(page) = self.current_page().cloned() else {
            return div().p_4().child("Unknown page").into_any_element();
        };
        let renderer = resolve_renderer(&page.renderer, custom_page_host(cx).is_some());
        if let PageRenderer::Shell { view_id } = renderer {
            if let Some(view) = self.mount_shell_page(&page, &view_id, window, cx) {
                let body = div()
                    .id("shell-page-body")
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(view)
                    .into_any_element();
                return self
                    .render_shell_page_frame(&page, body, cx)
                    .into_any_element();
            }
            if page.renderer.fallback.as_deref() != Some("native") {
                let error = self
                    .renderer_error
                    .clone()
                    .unwrap_or_else(|| "Shell renderer is unavailable".into());
                let body = div().p_4().child(error).into_any_element();
                return self
                    .render_shell_page_frame(&page, body, cx)
                    .into_any_element();
            }
        }
        if has_tasks(&page) {
            return self.render_tasks_page(&page, cx).into_any_element();
        }
        if has_stream(&page) {
            return self.render_events_page(&page, cx).into_any_element();
        }
        if form_of(&page).is_some() && table_of(&page).is_none() {
            return self.render_query_page(&page, window, cx).into_any_element();
        }
        // terminal 原语由 render() 直接接管,不进入工作台框架。

        let theme = cx.theme().clone();
        let state = self.page_state_snapshot();
        let is_collection = table_of(&page).is_some();
        let busy =
            self.row_action_running.is_some() || matches!(self.page_state, PageState::Loading);

        // 工具栏:标题 + 行数徽章 + 路由上下文(右侧为页面动作)。
        let mut toolbar = h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .px_4()
            .py_2p5()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_base()
                    .font_semibold()
                    .child(page.title.clone()),
            );
        if let (Some(table), PageStateSnapshot::Loaded(value)) = (table_of(&page), &state) {
            let count = collection_table::items_of(table, value).len();
            toolbar = toolbar.child(
                Tag::secondary()
                    .with_size(Size::Small)
                    .child(count.to_string()),
            );
        }
        if let Some(summary) = self.route_summary() {
            toolbar = toolbar.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(summary),
            );
        }
        if let Some(page_number) = self.collection_page(&page) {
            // cursor 分页:没有 nextCursor 时禁用 Next,防止空翻页。
            let has_more = match self.collection_cursor(&page) {
                Some(_) => true,
                None => table_of(&page).is_some_and(|table| {
                    table.pagination.kind != ResourceWorkbenchPaginationKind::Cursor
                }),
            };
            toolbar = toolbar
                .child(
                    Button::new("page-previous")
                        .with_size(Size::Small)
                        .ghost()
                        .icon(IconName::ArrowLeft)
                        .disabled(page_number <= 1)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.change_collection_page(-1, cx);
                        })),
                )
                .child(
                    Tag::secondary()
                        .with_size(Size::Small)
                        .child(format!("Page {page_number}")),
                )
                .child(
                    Button::new("page-next")
                        .with_size(Size::Small)
                        .ghost()
                        .icon(IconName::ArrowRight)
                        .disabled(!has_more)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.change_collection_page(1, cx);
                        })),
                );
        }
        toolbar = toolbar.child(div().flex_1());
        // detail 页 links(如 Index → Mapping)。
        for (link_index, link) in page.links.iter().enumerate() {
            let target = link.page_id.clone();
            let route = route_binding::build_route(&link.route, &self.route_sources_without_row());
            toolbar = toolbar.child(
                Button::new(gpui::SharedString::from(format!("page-link-{link_index}")))
                    .with_size(Size::Small)
                    .ghost()
                    .icon(IconName::ArrowRight)
                    .label(link.title.clone())
                    .on_click(cx.listener(
                        move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                            this.navigate(target.clone(), route.clone(), cx);
                        },
                    )),
            );
        }
        // collection 页刷新:重新执行页面 load 操作。
        if is_collection && page.load.is_some() {
            toolbar = toolbar.child(
                Button::new("page-refresh")
                    .with_size(Size::Small)
                    .ghost()
                    .icon(IconName::RotateCw)
                    .tooltip("Refresh")
                    .loading(busy)
                    .on_click(cx.listener(
                        |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                            this.load_current_page(cx);
                        },
                    )),
            );
        }

        // 提示条:shell 回退 / 行操作失败 / 危险操作确认。
        let mut alerts: Vec<AnyElement> = Vec::new();
        if let Some(error) = self.renderer_error.clone() {
            alerts.push(alert_bar(
                AlertTone::Warning,
                IconName::TriangleAlert,
                format!("Shell renderer fallback: {error}"),
                &theme,
            ));
        }
        if let Some(error) = self.row_action_error.clone() {
            alerts.push(alert_bar(
                AlertTone::Danger,
                IconName::CircleX,
                format!("Action failed: {error}"),
                &theme,
            ));
        }
        if matches!(self.pending_confirm, Some(PendingConfirm::RowAction { .. })) {
            alerts.push(self.render_confirm_bar("row-action-confirm", cx));
        }
        // 主体:collection 走表格卡片,其余原语维持自带滚动的全幅视图。
        let body: AnyElement = match state {
            PageStateSnapshot::Loading if is_collection => self.render_collection(
                table_of(&page).expect("table primitive"),
                &page.id,
                &serde_json::Value::Null,
                window,
                cx,
            ),
            PageStateSnapshot::Loading => loading_state(&theme),
            PageStateSnapshot::Failed(error) => self.render_failure(error, cx),
            PageStateSnapshot::Idle => empty_state(
                IconName::Inbox,
                "Nothing to show",
                "This view has no data to load.",
                &theme,
            ),
            PageStateSnapshot::Loaded(value) => {
                if is_collection {
                    self.render_collection(
                        table_of(&page).expect("table primitive"),
                        &page.id,
                        &value,
                        window,
                        cx,
                    )
                } else if viewer_prefers_text(&page) {
                    // viewer 声明了 format: text,就按纯文本呈现,
                    // 不再退回 JSON 树视图。
                    render_text_view(&page.id, &value, &theme)
                } else {
                    self.render_json_value(&page.id, value, window, cx)
                }
            }
        };
        let body_container = if is_collection {
            div()
                .id("page-body")
                .flex_1()
                .min_h_0()
                .min_w_0()
                .mx_3()
                .mb_3()
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(theme.border)
                .overflow_hidden()
                .child(body)
        } else {
            div()
                .id("page-body")
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .child(body)
        };
        let mut page_view = v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(toolbar);
        if let Some(strip) = self.render_tab_strip(&page, cx) {
            page_view = page_view.child(strip);
        }
        if !alerts.is_empty() {
            page_view = page_view.child(v_flex().w_full().gap_2().px_4().py_2().children(alerts));
        }
        page_view.child(body_container).into_any_element()
    }

    /// shell 渲染器页面的外壳:页头 + tab 条 + 内容。
    /// 覆盖页面与原生页面共用同一套导航骨架,否则 shell 页会丢掉 tab 条,
    /// 用户在 tab 之间跳转时导航会突然消失。
    fn render_shell_page_frame(
        &self,
        page: &ResourceWorkbenchPage,
        body: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let mut header = h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .px_4()
            .py_2p5()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_base()
                    .font_semibold()
                    .child(page.title.clone()),
            );
        if let Some(summary) = self.route_summary() {
            header = header.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(summary),
            );
        }
        let mut view = v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(header);
        if let Some(strip) = self.render_tab_strip(page, cx) {
            view = view.child(strip);
        }
        view.child(
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .child(body),
        )
        .into_any_element()
    }

    /// 页面 tab 条:Docker Desktop 式下划线标签,当前页高亮。
    /// tabs 来自 layout.center.tabGroups 中当前页所属的组(v2:组只声明
    /// 一份,页面经 `tabGroupId` 引用),渲染时按 `pageId == 当前页 id` 判定选中。
    fn render_tab_strip(
        &self,
        page: &ResourceWorkbenchPage,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let tabs = self.layout.tab_group_for(page)?;
        if tabs.is_empty() {
            return None;
        }
        let theme = cx.theme().clone();
        let mut strip = h_flex()
            .id("page-tabs")
            .w_full()
            .min_w_0()
            .items_end()
            .gap_5()
            .px_4()
            .border_b_1()
            .border_color(theme.border);
        for tab in tabs {
            let active = tab.page_id == page.id;
            let target = tab.page_id.clone();
            let route = route_binding::build_route(&tab.route, &self.route_sources_without_row());
            let label_color = if active {
                theme.foreground
            } else {
                theme.muted_foreground
            };
            // 未选中项用透明下划线占位,保证所有 tab 基线一致。
            let underline = if active {
                theme.primary
            } else {
                theme.border.opacity(0.0)
            };
            strip = strip.child(
                v_flex()
                    .id(gpui::SharedString::from(format!("page-tab-{}", tab.id)))
                    .gap_1p5()
                    .cursor_pointer()
                    .child(
                        div()
                            .text_sm()
                            .text_color(label_color)
                            .when(active, |this| this.font_medium())
                            .child(tab.title.clone()),
                    )
                    .child(div().h(px(2.)).w_full().rounded_full().bg(underline))
                    .on_click(cx.listener(
                        move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                            this.navigate(target.clone(), route.clone(), cx);
                        },
                    )),
            );
        }
        Some(strip.into_any_element())
    }

    /// terminal 模板页面:独立的全区域「终端控制台」。
    /// 不走工作台框架(无侧边栏、无底部状态栏),顶栏 = 返回 + 标题 + 命令,
    /// 其下保留页面的 tab 条(如有),终端铺满剩余空间。
    fn render_terminal_page(
        &mut self,
        page: &ResourceWorkbenchPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let body: AnyElement = match self.mount_terminal_page(page, window, cx) {
            Some(view) => div()
                .id("terminal-body")
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .bg(theme.background)
                .child(view)
                .into_any_element(),
            None => {
                let error = self
                    .terminal_error
                    .clone()
                    .unwrap_or_else(|| "terminal is unavailable".into());
                empty_state(
                    IconName::TriangleAlert,
                    "Terminal unavailable",
                    error,
                    &theme,
                )
            }
        };
        let mut header = h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.background);
        // 返回来源页:终端页没有侧边栏,这是唯一的原路返回入口。
        if let Some((return_page, return_route)) = self.terminal_return.clone() {
            header = header.child(
                Button::new("terminal-back")
                    .with_size(Size::Small)
                    .ghost()
                    .icon(IconName::ArrowLeft)
                    .tooltip("Back")
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        let page_id = return_page.clone();
                        let route = return_route.clone();
                        this.navigate(page_id, route, cx);
                    })),
            );
        }
        header = header.child(
            div()
                .min_w_0()
                .truncate()
                .text_base()
                .font_semibold()
                .child(page.title.clone()),
        );
        if let Some(terminal) = terminal_of(page) {
            let session_metadata = self.session.metadata().unwrap_or(serde_json::Value::Null);
            let command = match terminal.command.as_deref() {
                Some(command) => {
                    let args = terminal
                        .args
                        .iter()
                        .map(|arg| interpolate_with_session(arg, &self.route, &session_metadata))
                        .collect::<Vec<_>>();
                    std::iter::once(interpolate_with_session(
                        command,
                        &self.route,
                        &session_metadata,
                    ))
                    .chain(args)
                    .collect::<Vec<_>>()
                    .join(" ")
                }
                None => terminal
                    .operation
                    .as_ref()
                    .map(|operation| format!("operation · {}", operation.operation))
                    .unwrap_or_default(),
            };
            header = header.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(command),
            );
        } else {
            header = header.child(div().flex_1());
        }
        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(header)
            .children(self.render_tab_strip(page, cx))
            .child(body)
            .into_any_element()
    }

    /// 底部状态栏:bottom status source 的 items 声明驱动,
    /// 数据由单一 operation 提供(纯 JSON Pointer 投影,无业务字段硬编码)。
    fn render_status_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let region = self.layout.bottom.as_ref()?;
        let BottomContent::Status { items, .. } = &region.content else {
            return None;
        };
        let theme = cx.theme().clone();
        let value = match &self.status_bar {
            Some(Ok(value)) => Some(value),
            _ => None,
        };
        let lookup = |path: &str| value.and_then(|value| value.pointer(path));

        let mut bar = h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_3()
            .px_4()
            .border_t_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted_foreground);

        for item in items {
            let raw = lookup(&item.path);
            let text = match item.format {
                F::Bytes => raw
                    .and_then(serde_json::Value::as_i64)
                    .map(format_bytes)
                    .unwrap_or_else(|| "-".into()),
                F::Percent => raw
                    .and_then(serde_json::Value::as_f64)
                    .map(|value| format!("{value:.2}%"))
                    .unwrap_or_else(|| "-".into()),
                F::Number => raw
                    .map(|value| match value {
                        serde_json::Value::Number(number) => number.to_string(),
                        other => plain_text(other),
                    })
                    .unwrap_or_else(|| "-".into()),
                F::Version => raw
                    .and_then(serde_json::Value::as_str)
                    .map(|version| format!("v{version}"))
                    .unwrap_or_default(),
                F::Pair => {
                    let current = raw.and_then(serde_json::Value::as_i64).unwrap_or_default();
                    let other = item
                        .other_path
                        .as_deref()
                        .and_then(lookup)
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or_default();
                    format!("{current}/{other}")
                }
                F::BooleanUp => {
                    let up = raw.and_then(serde_json::Value::as_bool).unwrap_or(false);
                    let dot = if up { theme.success } else { theme.danger };
                    bar = bar.child(div().size(px(8.)).rounded_full().bg(dot)).child(
                        div()
                            .text_color(if up { theme.foreground } else { theme.danger })
                            .child(format!(
                                "{} {}",
                                item.label,
                                if up { "running" } else { "unavailable" }
                            )),
                    );
                    continue;
                }
                F::Raw => raw.map(plain_text).unwrap_or_default(),
            };
            if text.is_empty() {
                continue;
            }
            bar = bar.child(
                h_flex()
                    .gap_1()
                    .child(div().child(item.label.clone()))
                    .child(div().text_color(theme.foreground).child(text)),
            );
        }

        bar = bar.child(div().flex_1());
        if let Some(error) = match &self.status_bar {
            Some(Err(error)) => Some(error.clone()),
            _ => None,
        } {
            bar = bar.child(div().text_color(theme.danger).child(error));
        } else if value.is_none() {
            bar = bar.child(div().child("Loading usage…"));
        }

        // 右:手动刷新。
        bar = bar.child(
            Button::new("status-refresh")
                .with_size(Size::XSmall)
                .ghost()
                .icon(IconName::RotateCw)
                .tooltip("Refresh usage")
                .loading(self.status_bar_loading)
                .on_click(cx.listener(
                    |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                        this.load_status_bar(cx);
                    },
                )),
        );
        Some(bar.into_any_element())
    }

    fn render_query_page(
        &mut self,
        page: &ResourceWorkbenchPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        // 懒初始化输入 state(保留已输入草稿)。
        let inputs = form_of(page)
            .map(|form| form.inputs.clone())
            .unwrap_or_default();
        let input_state = self
            .query_inputs
            .entry(self.selected_page.clone())
            .or_insert_with(|| cx.new(|cx| query_page::QueryInputState::new(&inputs, window, cx)))
            .clone();
        let theme = cx.theme().clone();
        let source = query_result_source(
            &self.query_result,
            page.load.is_some(),
            &self.page_state_snapshot(),
        );
        let result_view: AnyElement = match source {
            QueryResultSource::Query(result) | QueryResultSource::PageLoad(result) => {
                match result {
                    Ok(value) => self.render_embedded_json(value, window, cx),
                    Err(error) => self.render_failure(error, cx),
                }
            }
            QueryResultSource::Loading => loading_state(&theme),
            QueryResultSource::Empty => empty_state(
                IconName::Info,
                "No result yet",
                "Run the operation to see its output here.",
                &theme,
            ),
        };
        let mut view = v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2p5()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_base()
                            .font_semibold()
                            .child(page.title.clone()),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .child(input_state)
                    .child(
                        h_flex().w_full().justify_end().child(
                            Button::new("query-execute")
                                .with_size(Size::Small)
                                .primary()
                                .icon(IconName::Play)
                                .label("Run")
                                .loading(self.query_running)
                                .on_click(cx.listener(
                                    move |this: &mut Self,
                                          _event: &gpui::ClickEvent,
                                          _window,
                                          cx| {
                                        this.execute_query(cx);
                                    },
                                )),
                        ),
                    ),
            );
        // 危险操作确认条:确认/取消后继续或放弃本次写操作。
        if self.pending_confirm.is_some() {
            view = view.child(
                h_flex()
                    .w_full()
                    .px_4()
                    .pb_2()
                    .child(self.render_confirm_bar("query-confirm", cx)),
            );
        }
        view.child(
            div()
                .id("query-result")
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_y_scroll()
                .child(result_view),
        )
    }

    /// 危险操作确认条:warning 提示 + Confirm/Cancel,query 与 collection 行操作共用。
    fn render_confirm_bar(&self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.warning.opacity(0.45))
            .bg(theme.warning.opacity(0.12))
            .child(
                Icon::new(IconName::TriangleAlert)
                    .size_4()
                    .text_color(theme.warning),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child("This operation may change data. Continue?"),
            )
            .child(
                Button::new(gpui::SharedString::from(format!("{id}-confirm")))
                    .with_size(Size::Small)
                    .danger()
                    .label("Confirm")
                    .on_click(cx.listener(
                        |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                            this.confirm_pending(cx);
                        },
                    )),
            )
            .child(
                Button::new(gpui::SharedString::from(format!("{id}-cancel")))
                    .with_size(Size::Small)
                    .label("Cancel")
                    .on_click(cx.listener(
                        |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                            this.cancel_pending(cx);
                        },
                    )),
            )
            .into_any_element()
    }

    /// 加载失败态:图标 + 原因 + 重试。
    fn render_failure(&self, error: String, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                Icon::new(IconName::TriangleAlert)
                    .size_6()
                    .text_color(theme.danger),
            )
            .child(
                div()
                    .text_sm()
                    .font_medium()
                    .child("Could not load this page"),
            )
            .child(
                div()
                    .max_w(px(520.))
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(error),
            )
            .child(
                Button::new("page-retry")
                    .with_size(Size::Small)
                    .icon(IconName::RotateCw)
                    .label("Retry")
                    .on_click(cx.listener(
                        |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                            this.load_current_page(cx);
                        },
                    )),
            )
            .into_any_element()
    }

    /// 当前路由的摘要(如 `id: abc123`),用于工具栏上下文。
    fn route_summary(&self) -> Option<String> {
        let serde_json::Value::Object(entries) = &self.route else {
            return None;
        };
        if entries.is_empty() {
            return None;
        }
        Some(
            entries
                .iter()
                .map(|(key, value)| format!("{key}: {}", plain_text(value)))
                .collect::<Vec<_>>()
                .join("   "),
        )
    }

    fn cancel_task(&mut self, job_id: String, cx: &mut Context<Self>) {
        let session = self.session.clone();
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = tokio
                .spawn(async move { session.cancel_task(&job_id).await })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.task_error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error.to_string()),
                    Err(error) => Some(error.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn render_tasks_page(
        &mut self,
        page: &ResourceWorkbenchPage,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let tasks = self.session.task_snapshots();
        let body: AnyElement = if tasks.is_empty() {
            empty_state(
                IconName::Inbox,
                "No tasks",
                "Long running operations started from this connection appear here.",
                &theme,
            )
        } else {
            let mut list = v_flex().w_full();
            for (index, task) in tasks.into_iter().enumerate() {
                let cancellable = matches!(
                    task.state,
                    extension_protocol::job::JobState::Queued
                        | extension_protocol::job::JobState::Running
                );
                let state_label = format!("{:?}", task.state);
                let job_id = task.job_id.clone();
                let mut row = h_flex()
                    .id(("task-row", index))
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.table_row_border)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(theme.mono_font_family.clone())
                            .text_sm()
                            .child(task.job_id.clone()),
                    )
                    .child(Tag::secondary().with_size(Size::Small).child(state_label))
                    .child(div().flex_1());
                if cancellable {
                    row = row.child(
                        Button::new(gpui::SharedString::from(format!("cancel-task-{index}")))
                            .with_size(Size::Small)
                            .ghost()
                            .icon(IconName::Close)
                            .label("Cancel")
                            .on_click(cx.listener(
                                move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                                    this.cancel_task(job_id.clone(), cx);
                                },
                            )),
                    );
                }
                list = list.child(row);
            }
            list.into_any_element()
        };
        let mut view = v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2p5()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_base()
                            .font_semibold()
                            .child(page.title.clone()),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("refresh-tasks")
                            .with_size(Size::Small)
                            .ghost()
                            .icon(IconName::RotateCw)
                            .tooltip("Refresh")
                            .on_click(cx.listener(
                                |_this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                                    cx.notify();
                                },
                            )),
                    ),
            );
        if let Some(error) = self.task_error.clone() {
            view = view.child(h_flex().w_full().px_4().py_2().child(alert_bar(
                AlertTone::Danger,
                IconName::CircleX,
                error,
                &theme,
            )));
        }
        view.child(
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .child(body),
        )
    }

    /// events 模板页:消费 load operation 返回的事件流。
    /// 事件批次由 scope-owned subscription 持续写入;这里只渲染当前快照。
    fn render_events_page(
        &mut self,
        page: &ResourceWorkbenchPage,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let mut header = h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .px_4()
            .py_2p5()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_base()
                    .font_semibold()
                    .child(page.title.clone()),
            )
            .child(div().flex_1());
        if !self.event_batches.is_empty() {
            header = header.child(
                Tag::secondary()
                    .with_size(Size::Small)
                    .child(format!("{} events", self.event_batches.len())),
            );
        }
        if self.event_dropped > 0 {
            header = header.child(
                Tag::secondary()
                    .with_size(Size::Small)
                    .child(format!("{} dropped", self.event_dropped)),
            );
        }
        if self.event_closed {
            header = header.child(Tag::secondary().with_size(Size::Small).child("Closed"));
        }
        header = header
            .child(
                Button::new("events-clear")
                    .with_size(Size::Small)
                    .ghost()
                    .label("Clear")
                    .disabled(self.event_batches.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.event_batches.clear();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("events-stop")
                    .with_size(Size::Small)
                    .ghost()
                    .label("Stop")
                    .disabled(self.event_closed)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_active_request();
                        this.event_closed = true;
                        cx.notify();
                    })),
            );
        let mut view = v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(header);
        // 实时页与原生/Shell 页共用同一套 tab 条:切到发布/订阅时导航不会突然消失。
        if let Some(strip) = self.render_tab_strip(page, cx) {
            view = view.child(strip);
        }
        if let Some(error) = self.event_error.clone() {
            view = view.child(h_flex().w_full().px_4().py_2().child(alert_bar(
                AlertTone::Danger,
                IconName::CircleX,
                error,
                &theme,
            )));
        }
        let body: AnyElement = if self.event_batches.is_empty() {
            empty_state(
                IconName::Inbox,
                "Waiting for events",
                "Events from this connection will appear here.",
                &theme,
            )
        } else {
            let mut list = v_flex().w_full();
            for (index, event) in self
                .event_batches
                .iter()
                .rev()
                .take(MAX_RENDERED_EVENTS)
                .enumerate()
            {
                // 中间件消息模型渲染成可读行;其余事件(如运维事件)回落原始 JSON,
                // 保证 events 模板对任意扩展都可用。
                list = list.child(match MessageEvent::parse(event) {
                    Some(message) => message_event_row(&message, index, &theme),
                    None => div()
                        .id(("event-row", index))
                        .w_full()
                        .min_w_0()
                        .px_4()
                        .py_1p5()
                        .border_b_1()
                        .border_color(theme.table_row_border)
                        .font_family(theme.mono_font_family.clone())
                        .text_xs()
                        .child(plain_text(event))
                        .into_any_element(),
                });
            }
            list.into_any_element()
        };
        view.child(
            div()
                .id("events-body")
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_y_scroll()
                .child(body),
        )
    }
}

/// 提示条语义色。
#[derive(Clone, Copy, PartialEq, Eq)]
enum AlertTone {
    Warning,
    Danger,
}

/// 页面内提示条:图标 + 文案,描边染色而不是实心色块。
fn alert_bar(
    tone: AlertTone,
    icon: IconName,
    message: String,
    theme: &gpui_component::Theme,
) -> AnyElement {
    let accent = match tone {
        AlertTone::Warning => theme.warning,
        AlertTone::Danger => theme.danger,
    };
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .rounded(theme.radius)
        .border_1()
        .border_color(accent.opacity(0.45))
        .bg(accent.opacity(0.1))
        .child(Icon::new(icon).size_4().text_color(accent))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .text_sm()
                .text_color(theme.foreground)
                .child(message),
        )
        .into_any_element()
}

/// viewer 原语 `format: text` 的呈现:字符串原样显示,其他 JSON 值
/// pretty 打印。声明了 text 就不再退回 JSON 树视图。
fn render_text_view(
    page_id: &str,
    value: &serde_json::Value,
    theme: &gpui_component::Theme,
) -> AnyElement {
    let text = match value {
        serde_json::Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    };
    div()
        .id(SharedString::from(format!("page-text-view-{page_id}")))
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_y_scrollbar()
        .p_4()
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_sm()
                .child(text),
        )
        .into_any_element()
}

/// 加载态:转圈 + 说明。
fn loading_state(theme: &gpui_component::Theme) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            Spinner::new()
                .with_size(Size::Medium)
                .color(theme.muted_foreground),
        )
        .child(
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Loading…"),
        )
        .into_any_element()
}

/// 空态:图标 + 标题 + 说明。
fn empty_state(
    icon: IconName,
    title: impl Into<SharedString>,
    hint: impl Into<SharedString>,
    theme: &gpui_component::Theme,
) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(
            Icon::new(icon)
                .size_8()
                .text_color(theme.muted_foreground.opacity(0.6)),
        )
        .child(
            div()
                .text_sm()
                .font_medium()
                .text_color(theme.foreground)
                .child(title.into()),
        )
        .child(
            div()
                .max_w(px(420.))
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(hint.into()),
        )
        .into_any_element()
}

/// 路由/表格值的纯文本形式。
fn plain_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => "-".into(),
        other => other.to_string(),
    }
}

/// 一条实时消息事件的紧凑两行视图。
///
/// 首行:topic + QoS/Retain 徽章 + 右对齐的时间与消息 ID;
/// 次行:消息体(单行截断)。
///
/// 消息体**截断而不换行**:events 列表是持续追加的流,长 JSON 体换行会把列表
/// 冲散、并让滚动位置频繁跳变;要看完整内容用 message 查询页或消息详情。
fn message_event_row(
    event: &MessageEvent,
    index: usize,
    theme: &gpui_component::Theme,
) -> AnyElement {
    let mono = theme.mono_font_family.clone();
    let mut header = h_flex().w_full().min_w_0().items_center().gap_2().child(
        div()
            .min_w_0()
            .truncate()
            .text_sm()
            .font_medium()
            .text_color(theme.foreground)
            .child(event.topic.clone()),
    );
    if let Some(qos) = &event.qos {
        header = header.child(Tag::secondary().with_size(Size::Small).child(qos.clone()));
    }
    if event.retain {
        header = header.child(Tag::warning().with_size(Size::Small).child("Retain"));
    }
    header = header.child(div().flex_1());
    if let Some(timestamp) = &event.timestamp {
        header = header.child(
            div()
                .flex_shrink_0()
                .font_family(mono.clone())
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(timestamp.clone()),
        );
    }
    if let Some(message_id) = &event.message_id {
        header = header.child(
            div()
                .flex_shrink_0()
                .font_family(mono.clone())
                .text_xs()
                .text_color(theme.muted_foreground.opacity(0.7))
                .child(message_id.clone()),
        );
    }
    let body_color = if event.body.is_some() {
        theme.foreground
    } else {
        theme.muted_foreground
    };
    let mut row = v_flex()
        .id(("event-row", index))
        .w_full()
        .min_w_0()
        .gap_1()
        .px_4()
        .py_2()
        .border_b_1()
        .border_color(theme.table_row_border)
        .child(header)
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_family(mono)
                .text_xs()
                .text_color(body_color)
                .child(
                    event
                        .body
                        .clone()
                        .unwrap_or_else(|| "no payload text".into()),
                ),
        );
    if !event.extra.is_empty() {
        let summary = event
            .extra
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("  ");
        row = row.child(
            div()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(summary),
        );
    }
    row.into_any_element()
}

/// 人类可读的字节数:1.2 GB / 340.5 MB / 12.0 KB。
fn format_bytes(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes.max(0) as f64;
    if value >= GB {
        format!("{:.2} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{} B", bytes.max(0))
    }
}

impl NativeResourceWorkbench {
    /// query 结果等内嵌场景:复用页面级 JSON 值视图(独立于页面 load 状态)。
    fn render_embedded_json(
        &mut self,
        value: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("{}::result", self.selected_page);
        self.render_json_value(&key, value, window, cx)
    }

    /// json 模板页面渲染:复用 json_view 的通用 JSON 值视图(树 / 原始编辑器)。
    /// 惰性创建页面级实体;数据刷新时仅在树模式下同步新值,保留用户切换的
    /// Raw 编辑内容不被打断。
    fn render_json_value(
        &mut self,
        page_id: &str,
        value: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let needs_reset = match self.json_views.get(page_id) {
            Some(view) => {
                let view = view.read(cx);
                view.mode() == json_view::JsonDisplayMode::Tree && view.snapshot() != Some(&value)
            }
            None => true,
        };
        if needs_reset {
            if let Some(existing) = self.json_views.get(page_id).cloned() {
                existing.update(cx, |view, cx| view.set_value(value.clone(), window, cx));
            } else {
                let view = cx.new(|cx| json_view::JsonValueView::new(value.clone(), window, cx));
                self.json_views.insert(page_id.to_string(), view);
            }
        }
        let view = self.json_views[page_id].clone();
        div()
            .id("page-json-view")
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(view)
            .into_any_element()
    }

    /// collection 渲染:交给 `gpui-component` 的 DataTable(主题化表头 / 斑马纹 /
    /// 行悬停 / 列缩放 / 本地排序 / 骨架加载),行操作按钮在操作列内渲染。
    fn render_collection(
        &mut self,
        table: &ResourceWorkbenchTable,
        page_id: &str,
        value: &serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collection = table.clone();
        let loading = matches!(self.page_state, PageState::Loading);
        let items = if loading {
            Vec::new()
        } else {
            collection_table::items_of(&collection, value)
        };
        // 表格实体按 (页面, 数据版本) 缓存:首次加载给骨架屏,已有数据时
        // 保留旧行(由工具栏刷新按钮表达进行中),避免每次操作都闪一次骨架。
        let stale = self.collection_table.as_ref().is_none_or(|cached| {
            cached.page_id != page_id
                || (!loading && (cached.loading || cached.generation != self.page_generation))
        });
        if stale {
            let actions = collection
                .actions
                .iter()
                .map(|action| {
                    let destructive = matches!(
                        extension_plugin_adapter::operation_effect(
                            &self.descriptor,
                            &action.operation
                        ),
                        Some(
                            extension_runtime::extension::manifest::ResourceWorkbenchEffect::Destructive
                        )
                    );
                    RowActionView::from_manifest(action, destructive)
                })
                .collect();
            let delegate = CollectionTableDelegate::new(
                &collection,
                items,
                actions,
                cx.entity().downgrade(),
                loading,
            );
            let table = build_table_state(delegate, window, cx);
            self.collection_table = Some(CollectionTableView {
                page_id: page_id.to_string(),
                generation: self.page_generation,
                loading,
                table,
            });
        }
        let Some(cached) = self.collection_table.as_ref() else {
            return div().into_any_element();
        };
        DataTable::new(&cached.table)
            .stripe(true)
            .bordered(false)
            .scrollbar_visible(true, true)
            .with_size(Size::Small)
            .into_any_element()
    }
}

impl EventEmitter<()> for NativeResourceWorkbench {}

impl Focusable for NativeResourceWorkbench {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NativeResourceWorkbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        // root Shell 覆盖:整个工作台主体交给 JS 视图,无 native 区域。
        // 失败时显示错误态(root 级无 fallback,不伪装成可用 native 工作台)。
        if let Some(view_id) = self.layout.root_shell.clone() {
            let mut body = match self.mount_region_shell(RegionId::Root, &view_id, window, cx) {
                Some(view) => div()
                    .id("root-shell-body")
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(view)
                    .into_any_element(),
                None => empty_state(
                    IconName::TriangleAlert,
                    "Workbench view unavailable",
                    "This workbench is provided by an extension view that failed to load.",
                    &theme,
                ),
            };
            if let Some((return_page, return_route)) = self.terminal_return.clone() {
                body = v_flex()
                    .size_full()
                    .min_h_0()
                    .child(
                        h_flex()
                            .w_full()
                            .px_3()
                            .py_1()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                Button::new("workbench-shell-back")
                                    .with_size(Size::Small)
                                    .ghost()
                                    .icon(IconName::ArrowLeft)
                                    .label("Back")
                                    .on_click(cx.listener(move |this, _event, _window, cx| {
                                        let page_id = return_page.clone();
                                        let route = return_route.clone();
                                        this.navigate(page_id, route, cx);
                                    })),
                            ),
                    )
                    .child(body)
                    .into_any_element();
            }
            return div()
                .id("resource-workbench")
                .track_focus(&self.focus_handle)
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .child(body);
        }
        // 区域组合:h_flex = left + 中列(center + bottom) + right。
        let left = self.render_nav(window, cx);
        let center = self.layout.center.clone();
        let page = match center.as_ref().map(|center| &center.content) {
            Some(CenterContent::Shell { view_id }) => {
                let view_id = view_id.clone();
                self.mount_region_shell(RegionId::Center, &view_id, window, cx)
                    .map(|view| {
                        div()
                            .id("center-shell-region")
                            .size_full()
                            .min_w_0()
                            .min_h_0()
                            .child(view)
                            .into_any_element()
                    })
                    .unwrap_or_else(|| {
                        empty_state(
                            IconName::TriangleAlert,
                            "Center view unavailable",
                            "The center region is provided by an extension view that failed to load.",
                            &theme,
                        )
                    })
            }
            _ => {
                // terminal 模板页在 center 区域内渲染:保留 left/right/bottom
                // 区域。若像早期实现那样提前 return 占满工作台,切到终端页
                // (如 Docker exec)会让左树/右栏整体消失。
                let terminal_page = self
                    .current_page()
                    .filter(|page| is_terminal_page(page))
                    .cloned();
                match terminal_page {
                    Some(page) => self.render_terminal_page(&page, window, cx),
                    None => self.render_page(window, cx).into_any_element(),
                }
            }
        };
        let mut main_column = v_flex().flex_1().min_w_0().min_h_0().child(page);
        // bottom:status(宿主渲染)或 shell(JS 视图)。
        let bottom_region = self.layout.bottom.clone();
        if let Some(bottom) = bottom_region.as_ref() {
            match &bottom.content {
                BottomContent::Status { .. } => {
                    if let Some(status_bar) = self.render_status_bar(cx) {
                        main_column = main_column.child(status_bar);
                    }
                }
                BottomContent::Shell { view_id } => {
                    let view_id = view_id.clone();
                    let height = bottom.height;
                    if let Some(view) =
                        self.mount_region_shell(RegionId::Bottom, &view_id, window, cx)
                    {
                        main_column = main_column.child(
                            h_flex()
                                .w_full()
                                .h(px(height as f32))
                                .min_h_0()
                                .flex_shrink_0()
                                .border_t_1()
                                .border_color(theme.border)
                                .overflow_hidden()
                                .child(
                                    div()
                                        .id("bottom-shell-region")
                                        .size_full()
                                        .min_w_0()
                                        .min_h_0()
                                        .child(view),
                                ),
                        );
                    }
                }
                BottomContent::None => {}
            }
        }
        let mut root = div()
            .id("resource-workbench")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .children(left);
        root = root.child(main_column);
        let side_region = self.layout.right.clone();
        if let Some(side) = side_region.as_ref() {
            if let SideContent::Shell { view_id } = &side.content {
                let width = side.width;
                if let Some(view) = self.mount_region_shell(RegionId::Right, view_id, window, cx) {
                    root = root.child(
                        v_flex()
                            .w(px(width as f32))
                            .h_full()
                            .flex_shrink_0()
                            .overflow_hidden()
                            .bg(theme.sidebar)
                            .border_l_1()
                            .border_color(theme.sidebar_border)
                            .child(
                                div()
                                    .id("right-shell-region")
                                    .size_full()
                                    .min_w_0()
                                    .min_h_0()
                                    .child(view),
                            ),
                    );
                }
            }
        }
        root
    }
}

#[cfg(test)]
mod workbench_render_structure_tests {
    /// 终端模板页必须在区域组合内渲染:左树/右栏/底栏要随终端页一起保留。
    /// 回归:早期实现让终端页在 render 顶部提前 return 占满工作台,切到
    /// Docker exec 这类 terminal 页时左树与右栏会整体消失。
    #[test]
    fn terminal_pages_render_inside_the_region_composition() {
        let source = include_str!("lib.rs");
        let render_impl = source
            .split("impl Render for NativeResourceWorkbench")
            .nth(1)
            .expect("workbench Render impl exists");
        let (before_regions, from_regions) = render_impl
            .split_once("// 区域组合")
            .expect("region composition marker exists");
        assert!(
            !before_regions.contains("self.render_terminal_page"),
            "terminal page must not early-return before region composition"
        );
        assert!(
            from_regions.contains("self.render_terminal_page"),
            "terminal page must be rendered inside the center region"
        );
    }

    /// 取 `marker` 之后、外层函数体结束之前的那段源码。
    fn body_after(source: &str, marker: &str) -> String {
        let after = source
            .split(marker)
            .nth(1)
            .unwrap_or_else(|| panic!("`{marker}` exists"));
        // 函数体以 4 空格缩进的 `}` 收尾;体内嵌套块是 8 空格,不会误切。
        after
            .split("\n    }\n")
            .next()
            .expect("function body terminates")
            .to_string()
    }

    /// 拼接要搜索的字面量。
    ///
    /// 这些测试用 `include_str!("lib.rs")` 扫自己所在的文件,直接写字面量会让
    /// 断言命中测试自身的源码(要么假通过,要么必然失败)。拆成片段在运行期拼接,
    /// 被测源码里就不会出现连续的目标串。
    fn needle(parts: &[&str]) -> String {
        parts.concat()
    }

    /// 每个导航入口都要**无条件**推进页面世代。
    ///
    /// 回归(旧的「加载修订号」计数时代):世代只在 `load_current_page` 里推进,而
    /// 目标页没有 `load` 时它会提前返回 —— 世代不变,旧页面的迟到回调因此
    /// 通过校验,把结果与错误写进新页面(结果串页)。
    #[test]
    fn every_navigation_entry_point_advances_the_page_generation() {
        let source = include_str!("lib.rs");

        let transition = body_after(source, "fn begin_page_transition");
        assert!(
            transition.contains(&needle(&[
                "self.page_generation = self.page_generation.",
                "wrapping_add(1)"
            ])),
            "begin_page_transition must advance the generation"
        );
        assert!(
            transition.contains(&needle(&["self.query_running = ", "false"])),
            "begin_page_transition must clear the per-page run state: \
             the cancelled request's callback returns early on the generation check \
             and would otherwise never clear it"
        );

        for entry in ["pub fn select_page", "pub fn navigate"] {
            let body = body_after(source, entry);
            assert!(
                body.contains("self.begin_page_transition()"),
                "{entry} must advance the generation instead of resetting state by hand"
            );
        }
    }

    /// 所有异步回写都要按**发起时的世代**判归属。
    ///
    /// 取消请求不能代替归属检查:取消与回调真正到达之间有窗口期。
    #[test]
    fn every_async_writeback_checks_its_page_generation() {
        let source = include_str!("lib.rs");
        let guard = needle(&["this.page_generation != ", "generation"]);
        assert!(
            !source.contains(&needle(&["load_", "revision"])),
            "the counter must be named for the page instance generation, \
             not for a data revision: the old name invited exactly the confusion \
             that let a stale callback pass the check"
        );
        // 回写点:load 完成、query 完成、行操作完成,以及 stream 的四个分支。
        let guards = source.matches(&guard).count();
        assert!(
            guards >= 6,
            "expected every async writeback to check its page generation, found {guards}"
        );

        // stream 消费循环最容易只判第一批:切页不会让 provider 停止推送。
        // 循环体到它所在 spawn 块结束为止(`.detach()` 收口),不按字符数截断 ——
        // 窗口太短会在没人改动行为的情况下假失败。
        let after_loop = source
            .split("while let Some(batch) = subscription.recv().await")
            .nth(1)
            .expect("event stream consumption loop exists");
        let loop_body = after_loop
            .split(".detach()")
            .next()
            .expect("the stream task terminates");
        assert!(
            loop_body.contains(&guard),
            "each event batch must be checked against the generation"
        );
        assert!(
            loop_body.contains(&needle(&["if stale ", "{"])),
            "a stale batch must break out of the loop and release the subscription"
        );
    }

    /// `source: connection` 必须在每一个 `BindingContext` 构造点填真实值。
    ///
    /// 回归:`connection` 加进契约后,6 个构造点里 5 个补了字段但都填 `Null`,
    /// 于是"扩展声明合法、调用永远拿不到值"。
    #[test]
    fn every_binding_context_carries_the_injected_connection() {
        let source = include_str!("lib.rs");
        assert!(
            !source.contains(&needle(&["connection: serde_json::Value::", "Null"])),
            "no binding context may hard-code a null connection: the host injects it"
        );
        let injected = needle(&["connection: self.connection", ".clone()"]);
        assert!(
            source.matches(&injected).count() >= 3,
            "load / query / row-action contexts must all carry the connection"
        );
        // route 侧来源与 shell 页上下文同样要带上,否则同一个 manifest
        // 在"参数绑定"和"路由绑定"两边行为不一致。
        assert!(source.contains(&needle(&["connection: &self.", "connection"])));
        assert!(source.contains(&needle(&["\"connection\": self.", "connection"])));
    }

    /// 树 lazy 展开的上下文也必须带连接:它走 `..Default::default()`,
    /// 少写一行不会编译失败,只会让 `source: connection` 报 BindingMissing。
    #[test]
    fn tree_children_context_carries_the_injected_connection() {
        let source = include_str!("nav_tree.rs");
        let context = body_after(source, "let context = BindingContext {");
        assert!(
            context.contains(&needle(&["connection: self.connection", ".clone()"])),
            "tree children load must carry the connection context"
        );
    }
}

/// query 页结果区来源的决策。
///
/// 单独抽成纯函数是因为这里要合流两份状态,而两者生命周期不同:
/// `query_result` 每次导航都被清,`page_state` 不会。
#[cfg(test)]
mod query_result_source_tests {
    use super::*;

    fn loaded(value: serde_json::Value) -> PageStateSnapshot {
        PageStateSnapshot::Loaded(value)
    }

    /// 用户这次执行的结果永远优先于 load 的结果。
    #[test]
    fn a_user_run_overrides_the_load_result() {
        let source = query_result_source(
            &Some(Ok(serde_json::json!({"ran": true}))),
            true,
            &loaded(serde_json::json!({"loaded": true})),
        );
        assert_eq!(
            QueryResultSource::Query(Ok(serde_json::json!({"ran": true}))),
            source
        );
    }

    /// 声明了 `load` 的查询页,打开即应以 load 结果为初始结果集。
    #[test]
    fn a_load_declaring_page_shows_the_load_result_first() {
        let source = query_result_source(&None, true, &loaded(serde_json::json!({"hits": 3})));
        assert_eq!(
            QueryResultSource::PageLoad(Ok(serde_json::json!({"hits": 3}))),
            source
        );
    }

    /// 没有 `load` 的查询页必须保持空状态。
    ///
    /// 回归:`page_state` 是跨页共享字段,`begin_page_transition` 只清 `query_result`。
    /// 不卡这一道,从上个页面切过来就会看到别人的 load 结果。
    #[test]
    fn a_page_without_load_stays_empty_even_with_a_stale_page_state() {
        let source = query_result_source(&None, false, &loaded(serde_json::json!({"stale": 1})));
        assert_eq!(QueryResultSource::Empty, source);
    }

    /// load 失败要出现在结果区,而不是被吞成"还没有结果"。
    #[test]
    fn a_failed_load_surfaces_in_the_result_area() {
        let source = query_result_source(&None, true, &PageStateSnapshot::Failed("boom".into()));
        assert_eq!(QueryResultSource::PageLoad(Err("boom".into())), source);
    }

    /// 初始 load 还在路上时显示加载态,而不是"运行一次才有结果"的误导提示。
    #[test]
    fn an_in_flight_load_shows_the_loading_state() {
        let source = query_result_source(&None, true, &PageStateSnapshot::Loading);
        assert_eq!(QueryResultSource::Loading, source);
    }

    /// 还没开始 load 的页面仍然是空状态。
    #[test]
    fn an_idle_load_declaring_page_stays_empty() {
        let source = query_result_source(&None, true, &PageStateSnapshot::Idle);
        assert_eq!(QueryResultSource::Empty, source);
    }
}
