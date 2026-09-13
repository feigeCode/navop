//! Native resource workbench primitives.
//!
//! Renderer state only: 页面只保存导航/加载/结果状态和会话 handle,
//! 所有 provider I/O 经 `extension_plugin_adapter::workbench_dispatch`
//! 的命名操作入口,连接主会话由宿主 tab 唯一持有。

mod collection_table;
pub mod custom_page_host;
pub mod query_page;
pub mod route_binding;
pub mod terminal_host;

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
use extension_runtime::extension::manifest::{ResourceWorkbenchPage, ResourceWorkbenchTemplate};
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, Size, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    spinner::Spinner,
    table::{DataTable, TableState},
    tag::Tag,
    v_flex,
};

struct ActiveShellMount {
    page_id: String,
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

/// collection 页表格的缓存:按 (页面, 数据版本, 加载态) 重建,
/// 避免每次重绘重建实体,同时保证 load 完成时确实刷新成新数据。
struct CollectionTableView {
    page_id: String,
    revision: u64,
    loading: bool,
    table: Entity<TableState<CollectionTableDelegate>>,
}

/// 挂载计数器,用于丢弃迟到结果(旧页面/旧请求的返回不得覆盖新状态)。
static NEXT_MOUNT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub struct NativeResourceWorkbench {
    descriptor: RegisteredResourceWorkbenchContribution,
    session: ResourceSessionHandle,
    selected_page: String,
    route: serde_json::Value,
    paging: serde_json::Value,
    page_state: PageState,
    load_revision: u64,
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
    active_request_cancel: Option<extension_host::CancellationToken>,
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

impl NativeResourceWorkbench {
    fn page_state_snapshot(&self) -> PageStateSnapshot {
        match &self.page_state {
            PageState::Idle => PageStateSnapshot::Idle,
            PageState::Loading => PageStateSnapshot::Loading,
            PageState::Loaded(value) => PageStateSnapshot::Loaded(value.clone()),
            PageState::Failed(error) => PageStateSnapshot::Failed(error.clone()),
        }
    }

    pub fn new(
        descriptor: RegisteredResourceWorkbenchContribution,
        session: ResourceSessionHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected_page = descriptor.default_page.clone();
        let mut this = Self {
            descriptor,
            session,
            selected_page,
            route: serde_json::Value::Null,
            paging: serde_json::json!({"page": 1, "limit": 50, "cursor": null}),
            page_state: PageState::Idle,
            load_revision: 0,
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
            terminal_mount: None,
            terminal_error: None,
            terminal_return: None,
            status_bar: None,
            status_bar_loading: false,
            renderer_error: None,
            task_error: None,
            active_request_cancel: None,
            event_batches: Vec::new(),
            event_dropped: 0,
            event_closed: false,
            event_error: None,
            _subscriptions: Vec::new(),
        };
        this._subscriptions
            .push(cx.on_release(|this, cx| this.dispose_shell_mount(cx)));
        this._subscriptions
            .push(cx.on_release(|this, cx| this.dispose_terminal_mount(cx)));
        this._subscriptions.push(cx.on_release(|this, _cx| {
            this.cancel_active_request();
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

    fn collection_page(&self, page: &ResourceWorkbenchPage) -> Option<u64> {
        (page.template == ResourceWorkbenchTemplate::Collection)
            .then(|| page.collection.as_ref())
            .flatten()
            .filter(|collection| collection.pagination.kind != "none")
            .and_then(|_| self.paging.get("page").and_then(serde_json::Value::as_u64))
    }

    /// cursor 分页:上次 load 返回的 nextCursor;None 表示页码式或没有更多。
    fn collection_cursor(&self, page: &ResourceWorkbenchPage) -> Option<String> {
        let collection = (page.template == ResourceWorkbenchTemplate::Collection)
            .then(|| page.collection.as_ref())
            .flatten()?;
        if collection.pagination.kind != "cursor" {
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
            || page
                .collection
                .as_ref()
                .is_some_and(|collection| collection.pagination.kind == "cursor");
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
        if page
            .collection
            .as_ref()
            .is_none_or(|collection| collection.pagination.kind != "cursor")
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
        let target_is_terminal = self.descriptor.pages.iter().any(|page| {
            page.id == target_page_id && page.template == ResourceWorkbenchTemplate::Terminal
        });
        if target_is_terminal {
            let already_terminal = self
                .current_page()
                .map(|page| page.template == ResourceWorkbenchTemplate::Terminal)
                .unwrap_or(false);
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
        self.cancel_active_request();
        self.event_batches.clear();
        self.event_dropped = 0;
        self.event_closed = false;
        self.event_error = None;
        self.dispose_shell_mount(cx);
        self.dispose_terminal_mount(cx);
        self.selected_page = page_id;
        self.route = serde_json::Value::Null;
        self.paging = serde_json::json!({"page": 1, "limit": 50, "cursor": null});
        self.query_result = None;
        self.json_views.clear();
        self.collection_table = None;
        self.pending_confirm = None;
        self.row_action_running = None;
        self.row_action_error = None;
        self.terminal_error = None;
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
        self.cancel_active_request();
        self.event_batches.clear();
        self.event_dropped = 0;
        self.event_closed = false;
        self.event_error = None;
        self.dispose_shell_mount(cx);
        self.dispose_terminal_mount(cx);
        self.selected_page = page_id;
        self.route = route;
        self.paging = serde_json::json!({"page": 1, "limit": 50, "cursor": null});
        self.query_result = None;
        self.json_views.clear();
        self.collection_table = None;
        self.pending_confirm = None;
        self.row_action_running = None;
        self.row_action_error = None;
        self.terminal_error = None;
        self.load_current_page(cx);
    }

    fn current_page(&self) -> Option<&ResourceWorkbenchPage> {
        self.descriptor
            .pages
            .iter()
            .find(|page| page.id == self.selected_page)
    }

    fn dispose_shell_mount(&mut self, cx: &mut App) {
        if let Some(active) = self.shell_mount.take() {
            active.host.dispose(active.mount, cx);
        }
        self.renderer_error = None;
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
        let Some(declaration) = page.terminal.as_ref() else {
            self.terminal_error = Some("page declares no terminal block".into());
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
            command: interpolate_with_session(&declaration.command, &self.route, &session_metadata),
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

    /// 拉取底部状态栏数据(statusBar.operation)。
    fn load_status_bar(&mut self, cx: &mut Context<Self>) {
        let Some(status_bar) = self.descriptor.status_bar.clone() else {
            return;
        };
        if !self
            .descriptor
            .operations
            .contains_key(&status_bar.operation)
        {
            self.status_bar = Some(Err(format!(
                "unknown status operation `{}`",
                status_bar.operation
            )));
            cx.notify();
            return;
        }
        self.status_bar_loading = true;
        cx.notify();

        let mount_id = NEXT_MOUNT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let scope = self.session.scope("__status_bar__".to_string(), mount_id);
        let workbench = self.descriptor.clone();
        let context = BindingContext {
            input: serde_json::Value::Null,
            route: serde_json::Value::Null,
            selection: serde_json::Value::Null,
            paging: serde_json::json!({"page": 1, "limit": 50, "cursor": null}),
        };
        let operation = status_bar.operation;
        self.active_request_cancel = Some(scope.cancellation());
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
        if let Some(active) = &self.shell_mount {
            if active.page_id == page.id {
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
                "capabilities": self.session.capabilities(),
            }),
            resource_type: self.descriptor.resource_type.clone(),
            session: Some(self.session.clone()),
            workbench: self.descriptor.clone(),
        };
        match host.mount(request, window, cx) {
            Ok(mount) => {
                let view = mount.view.clone();
                self.shell_mount = Some(ActiveShellMount {
                    page_id: page.id.clone(),
                    host,
                    mount,
                });
                Some(view)
            }
            Err(error) => {
                self.renderer_error = Some(error.to_string());
                None
            }
        }
    }

    /// 触发当前页面的 load 操作。tasks 页面读宿主任务,不发 provider 请求。
    fn load_current_page(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.current_page() else {
            self.page_state = PageState::Failed("unknown workbench page".into());
            cx.notify();
            return;
        };
        let template = page.template;
        if matches!(
            template,
            ResourceWorkbenchTemplate::Tasks | ResourceWorkbenchTemplate::Query
        ) {
            self.page_state = PageState::Idle;
            cx.notify();
            return;
        }
        let Some(action) = page.load.clone() else {
            self.page_state = PageState::Idle;
            cx.notify();
            return;
        };
        self.load_revision += 1;
        let revision = self.load_revision;
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
        };
        let operation = action.operation;
        let tokio = self.tokio.clone();
        if template == ResourceWorkbenchTemplate::Events {
            let this_scope = scope;
            let this = cx.entity().downgrade();
            let workbench_for_event = workbench.clone();
            let context_for_event = context.clone();
            cx.spawn(async move |_, cx| {
                let result = dispatch_invoke_result_scoped(
                    &this_scope,
                    &workbench_for_event,
                    &operation,
                    &context_for_event,
                    false,
                )
                .await;
                let stream_id = match result {
                    Ok(extension_protocol::resource::ResourceInvokeResult {
                        result: extension_protocol::result_ref::ResultRef::EventStream { id },
                    }) => id,
                    Ok(_) => {
                        let _ = this.update(cx, |this, cx| {
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
                            this.event_error = Some(error.to_string());
                            this.page_state = PageState::Failed(error.to_string());
                            cx.notify();
                        });
                        return;
                    }
                };
                let stream = extension_protocol::event_stream::EventOpenResult { stream_id };
                let mut subscription = this_scope.subscribe_events(
                    &stream,
                    extension_plugin_adapter::EventStreamSubscriptionConfig::default(),
                );
                while let Some(batch) = subscription.recv().await {
                    let _ = this.update(cx, |this, cx| {
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
                if this.load_revision != revision {
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
        let Some(action) = page.execute.clone() else {
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
                if !page
                    .execute
                    .as_ref()
                    .is_some_and(|action| action.operation == operation)
                {
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
        for field in &page.inputs {
            let value = values.get(&field.id).cloned().unwrap_or_default();
            if field.required && value.is_empty() {
                self.query_result = Some(Err(format!("`{}` is required", field.id)));
                cx.notify();
                return;
            }
            input.insert(field.id.clone(), serde_json::Value::String(value));
        }

        self.query_running = true;
        self.query_result = None;
        cx.notify();

        self.load_revision += 1;
        let revision = self.load_revision;
        let mount_id = NEXT_MOUNT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let scope = self.session.scope(self.selected_page.clone(), mount_id);
        self.active_request_cancel = Some(scope.cancellation());
        let workbench = self.descriptor.clone();
        let context = BindingContext {
            input: serde_json::Value::Object(input),
            route: self.route.clone(),
            selection: serde_json::Value::Null,
            paging: self.paging_context(),
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
                if this.load_revision != revision {
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
        let Some(open) = page.collection.as_ref().and_then(|c| c.open.as_ref()) else {
            return;
        };
        let route = route_binding::build_route(&open.route, &self.route, &row);
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
                .and_then(|page| page.collection.as_ref())
                .map(|collection| collection_table::row_key(&row, &collection.key_paths))
                .unwrap_or_default(),
        });
        self.row_action_error = None;
        cx.notify();

        let revision = self.load_revision;
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
                if this.load_revision != revision || this.selected_page != page_id {
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

    /// 左侧页面导航:主题化的侧栏 + 选中态。
    fn render_nav(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let selected = self.selected_page.clone();
        let theme = cx.theme().clone();
        let entries = self.descriptor.navigation.iter().filter_map(|item| {
            self.descriptor
                .pages
                .iter()
                .find(|page| page.id == item.page_id)
                .map(|page| (page.id.clone(), page.title.clone()))
        });
        let mut list = v_flex().w_full().gap_0p5();
        for (page_id, title) in entries {
            let is_selected = page_id == selected;
            let target = page_id.clone();
            list = list.child(
                h_flex()
                    .id(page_id)
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
                    .child(div().min_w_0().truncate().child(title))
                    .on_click(cx.listener(
                        move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                            this.select_page(target.clone(), cx);
                        },
                    )),
            );
        }
        v_flex()
            .w(px(208.))
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
            .child(list)
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
        if page.template == ResourceWorkbenchTemplate::Query {
            return self.render_query_page(&page, window, cx).into_any_element();
        }
        if page.template == ResourceWorkbenchTemplate::Tasks {
            return self.render_tasks_page(&page, cx).into_any_element();
        }
        if page.template == ResourceWorkbenchTemplate::Events {
            return self.render_events_page(&page, cx).into_any_element();
        }
        // terminal 模板由 render() 直接接管,不进入工作台框架。

        let theme = cx.theme().clone();
        let state = self.page_state_snapshot();
        let is_collection = page.template == ResourceWorkbenchTemplate::Collection;
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
        if let (Some(collection), PageStateSnapshot::Loaded(value)) =
            (page.collection.as_ref(), &state)
        {
            let count = collection_table::items_of(collection, value).len();
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
                None => page
                    .collection
                    .as_ref()
                    .is_some_and(|collection| collection.pagination.kind != "cursor"),
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
            let route =
                route_binding::build_route(&link.route, &self.route, &serde_json::Value::Null);
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
        // 主体:collection 走表格卡片,其余模板维持自带滚动的全幅视图。
        let body: AnyElement = match state {
            PageStateSnapshot::Loading if is_collection => {
                self.render_collection(&page, &serde_json::Value::Null, window, cx)
            }
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
                    self.render_collection(&page, &value, window, cx)
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
    /// 只有声明了 `tabs` 的页面才有;每个页面声明完整 tab 列表,
    /// 渲染时按 `pageId == 当前页 id` 判定选中项。
    fn render_tab_strip(
        &self,
        page: &ResourceWorkbenchPage,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if page.tabs.is_empty() {
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
        for tab in &page.tabs {
            let active = tab.page_id == page.id;
            let target = tab.page_id.clone();
            let route =
                route_binding::build_route(&tab.route, &self.route, &serde_json::Value::Null);
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
        if let Some(command) = page.terminal.as_ref().map(|terminal| {
            let session_metadata = self.session.metadata().unwrap_or(serde_json::Value::Null);
            let args = terminal
                .args
                .iter()
                .map(|arg| interpolate_with_session(arg, &self.route, &session_metadata))
                .collect::<Vec<_>>();
            std::iter::once(interpolate_with_session(
                &terminal.command,
                &self.route,
                &session_metadata,
            ))
            .chain(args)
            .collect::<Vec<_>>()
            .join(" ")
        }) {
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

    /// 底部状态栏:engine 状态 + 资源计数 + 磁盘/容器占用。
    /// 未声明 `statusBar` 的工作台不渲染。
    fn render_status_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.descriptor.status_bar.as_ref()?;
        let theme = cx.theme().clone();
        let value = match &self.status_bar {
            Some(Ok(value)) => Some(value),
            _ => None,
        };
        let number = |key: &str| {
            value
                .and_then(|value| value.get(key))
                .and_then(serde_json::Value::as_i64)
        };
        let engine_ok = value
            .and_then(|value| value.get("engine"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let mut bar = h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_3()
            .px_4()
            .py_1p5()
            .border_t_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted_foreground);

        // 左:引擎状态指示点 + 文案。
        let dot_color = if engine_ok {
            theme.success
        } else {
            theme.danger
        };
        bar = bar
            .child(div().size(px(8.)).rounded_full().bg(dot_color))
            .child(div().text_color(theme.foreground).child(if engine_ok {
                "Engine running"
            } else {
                "Engine unavailable"
            }));
        if let Some(version) = value
            .and_then(|value| value.get("server_version"))
            .and_then(serde_json::Value::as_str)
        {
            bar = bar.child(div().child(format!("v{version}")));
        }

        // 中:资源计数与占用。数据未就绪时只显示加载提示。
        bar = bar.child(div().flex_1());
        if let Some(error) = match &self.status_bar {
            Some(Err(error)) => Some(error.clone()),
            _ => None,
        } {
            bar = bar.child(div().text_color(theme.danger).child(error));
        } else if let Some(value) = value {
            let containers_running = number("containers_running").unwrap_or(0);
            let containers_total = number("containers_total").unwrap_or(0);
            let images = number("images").unwrap_or(0);
            let volumes = number("volumes").unwrap_or(0);
            let networks = number("networks").unwrap_or(0);
            let disk = number("disk_used_bytes").unwrap_or(0);
            let memory = number("containers_memory_bytes").unwrap_or(0);
            let cpu = value
                .get("containers_cpu_percent")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            for (label, text) in [
                (
                    "Containers",
                    format!("{containers_running}/{containers_total}"),
                ),
                ("Images", images.to_string()),
                ("Volumes", volumes.to_string()),
                ("Networks", networks.to_string()),
                ("Disk", format_bytes(disk)),
                ("RAM", format_bytes(memory)),
                ("CPU", format!("{cpu:.2}%")),
            ] {
                bar = bar.child(
                    h_flex()
                        .gap_1()
                        .child(div().child(label))
                        .child(div().text_color(theme.foreground).child(text)),
                );
            }
        } else {
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
        let input_state = self
            .query_inputs
            .entry(self.selected_page.clone())
            .or_insert_with(|| cx.new(|cx| query_page::QueryInputState::new(page, window, cx)))
            .clone();
        let theme = cx.theme().clone();
        let result_view: AnyElement = match &self.query_result {
            None => empty_state(
                IconName::Info,
                "No result yet",
                "Run the operation to see its output here.",
                &theme,
            ),
            Some(Ok(value)) => self.render_embedded_json(value.clone(), window, cx),
            Some(Err(error)) => self.render_failure(error.clone(), cx),
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
            for (index, event) in self.event_batches.iter().rev().take(500).enumerate() {
                list = list.child(
                    div()
                        .id(("event-row", index))
                        .w_full()
                        .min_w_0()
                        .px_4()
                        .py_1p5()
                        .border_b_1()
                        .border_color(theme.table_row_border)
                        .font_family(theme.mono_font_family.clone())
                        .text_xs()
                        .child(plain_text(event)),
                );
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
        page: &ResourceWorkbenchPage,
        value: &serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(collection) = page.collection.clone() else {
            return self.render_json_value(&page.id, value.clone(), window, cx);
        };
        let loading = matches!(self.page_state, PageState::Loading);
        let items = if loading {
            Vec::new()
        } else {
            collection_table::items_of(&collection, value)
        };
        // 表格实体按 (页面, 数据版本) 缓存:首次加载给骨架屏,已有数据时
        // 保留旧行(由工具栏刷新按钮表达进行中),避免每次操作都闪一次骨架。
        let stale = self.collection_table.as_ref().is_none_or(|cached| {
            cached.page_id != page.id
                || (!loading && (cached.loading || cached.revision != self.load_revision))
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
                page_id: page.id.clone(),
                revision: self.load_revision,
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
        // terminal 模板页:独立的全区域终端控制台,不带侧边栏与底部状态栏。
        if self
            .current_page()
            .map(|page| page.template == ResourceWorkbenchTemplate::Terminal)
            .unwrap_or(false)
        {
            let page = self.current_page().cloned().unwrap_or_else(|| {
                self.descriptor
                    .pages
                    .first()
                    .cloned()
                    .expect("workbench has at least one page")
            });
            return div()
                .id("resource-workbench")
                .track_focus(&self.focus_handle)
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .child(self.render_terminal_page(&page, window, cx));
        }
        let page = self.render_page(window, cx);
        let status_bar = self.render_status_bar(cx);
        let mut main_column = v_flex().flex_1().min_w_0().min_h_0().child(page);
        if let Some(status_bar) = status_bar {
            main_column = main_column.child(status_bar);
        }
        div()
            .id("resource-workbench")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .child(self.render_nav(cx))
            .child(main_column)
    }
}
