//! Native resource workbench primitives.
//!
//! Renderer state only: 页面只保存导航/加载/结果状态和会话 handle,
//! 所有 provider I/O 经 `extension_plugin_adapter::workbench_dispatch`
//! 的命名操作入口,连接主会话由宿主 tab 唯一持有。

pub mod custom_page_host;
pub mod query_page;
pub mod route_binding;

pub use custom_page_host::{
    CustomPageHost, DirtyGuardDecision, GlobalCustomPageHost, MountHandle, PageRenderer,
    ShellMountError, ShellPageMount, ShellPageMountRequest, custom_page_host, resolve_renderer,
};

use extension_plugin_adapter::{
    BindingContext, ResourceSessionHandle, WorkbenchDispatchError,
    dispatch_invoke_scoped as dispatch_invoke, dispatch_job_scoped as dispatch_job,
};
use extension_runtime::RegisteredResourceWorkbenchContribution;
use extension_runtime::extension::manifest::{ResourceWorkbenchPage, ResourceWorkbenchTemplate};
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window,
    div, px,
};

struct ActiveShellMount {
    page_id: String,
    host: std::rc::Rc<dyn CustomPageHost>,
    mount: ShellPageMount,
}

/// 单个页面的加载状态。
pub enum PageState {
    Idle,
    Loading,
    Loaded(serde_json::Value),
    Failed(String),
}

/// 挂载计数器,用于丢弃迟到结果(旧页面/旧请求的返回不得覆盖新状态)。
static NEXT_MOUNT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub struct NativeResourceWorkbench {
    descriptor: RegisteredResourceWorkbenchContribution,
    session: ResourceSessionHandle,
    selected_page: String,
    route: serde_json::Value,
    page_state: PageState,
    load_revision: u64,
    focus_handle: FocusHandle,
    tokio: tokio::runtime::Handle,
    /// query 页面输入状态(按页面 id 保存,切换页面不丢失草稿)。
    query_inputs: std::collections::BTreeMap<String, Entity<query_page::QueryInputState>>,
    /// query 页面执行状态。
    query_result: Option<Result<serde_json::Value, String>>,
    query_running: bool,
    /// 待用户确认的危险操作 id(native 页面确认条)。
    pending_confirm: Option<String>,
    shell_mount: Option<ActiveShellMount>,
    renderer_error: Option<String>,
    task_error: Option<String>,
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
            page_state: PageState::Idle,
            load_revision: 0,
            focus_handle: cx.focus_handle(),
            tokio: one_core::gpui_tokio::Tokio::handle(cx),
            query_inputs: Default::default(),
            query_result: None,
            query_running: false,
            pending_confirm: None,
            shell_mount: None,
            renderer_error: None,
            task_error: None,
            _subscriptions: Vec::new(),
        };
        this._subscriptions
            .push(cx.on_release(|this, cx| this.dispose_shell_mount(cx)));
        this.load_current_page(cx);
        this
    }

    pub fn selection(&self) -> PageSelection {
        PageSelection {
            page_id: self.selected_page.clone(),
            route: self.route.clone(),
        }
    }

    pub fn select_page(&mut self, page_id: impl Into<String>, cx: &mut Context<Self>) {
        let page_id = page_id.into();
        if !self.descriptor.pages.iter().any(|page| page.id == page_id) {
            return;
        }
        self.dispose_shell_mount(cx);
        self.selected_page = page_id;
        self.route = serde_json::Value::Null;
        self.query_result = None;
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
        self.dispose_shell_mount(cx);
        self.selected_page = page_id;
        self.route = route;
        self.query_result = None;
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
        if matches!(
            page.template,
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
        let workbench = self.descriptor.clone();
        let context = BindingContext {
            input: serde_json::Value::Null,
            route: self.route.clone(),
            selection: serde_json::Value::Null,
        };
        let operation = action.operation;
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = tokio
                .spawn(
                    async move { dispatch_invoke(&scope, &workbench, &operation, &context, false).await },
                )
                .await
                .unwrap_or_else(|join_error| {
                    Err(WorkbenchDispatchError::Provider(join_error.to_string()))
                });
            let _ = this.update(cx, |this, cx| {
                if this.load_revision != revision {
                    return;
                }
                this.page_state = match result {
                    Ok(value) => PageState::Loaded(value),
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
        let effect = extension_plugin_adapter::operation_effect(&self.descriptor, &action.operation);
        if effect.is_some_and(extension_plugin_adapter::requires_confirmation) {
            self.pending_confirm = Some(action.operation.clone());
            cx.notify();
            return;
        }
        self.run_query(action.operation, &page, false, cx);
    }

    /// 用户确认危险操作后执行。
    fn confirm_pending(&mut self, cx: &mut Context<Self>) {
        let Some(operation) = self.pending_confirm.take() else {
            return;
        };
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
        let is_job = self.descriptor.operations.get(&operation).is_some_and(|op| {
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
        let workbench = self.descriptor.clone();
        let context = BindingContext {
            input: serde_json::Value::Object(input),
            route: self.route.clone(),
            selection: serde_json::Value::Null,
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

    fn render_nav(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let selected = self.selected_page.clone();
        div()
            .w(px(200.))
            .h_full()
            .flex_shrink_0()
            .p_3()
            .child(div().mb_2().child("Pages"))
            .children(self.descriptor.navigation.iter().filter_map(|item| {
                self.descriptor
                    .pages
                    .iter()
                    .find(|page| page.id == item.page_id)
                    .map(|page| {
                        let page_id = page.id.clone();
                        let title = page.title.clone();
                        let is_selected = page.id == selected;
                        let mut entry = div()
                            .id(page.id.clone())
                            .py_1()
                            .px_2()
                            .mb_1()
                            .cursor_pointer()
                            .child(title);
                        if is_selected {
                            entry = entry.bg(gpui::rgb(0x333333));
                        }
                        let handler = cx.listener(
                            move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                                this.select_page(page_id.clone(), cx);
                            },
                        );
                        entry.on_click(handler)
                    })
            }))
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
                return div().size_full().child(view).into_any_element();
            }
            if page.renderer.fallback.as_deref() != Some("native") {
                let error = self
                    .renderer_error
                    .clone()
                    .unwrap_or_else(|| "Shell renderer is unavailable".into());
                return div().p_4().child(error).into_any_element();
            }
        }
        if page.template == ResourceWorkbenchTemplate::Query {
            return self.render_query_page(&page, window, cx).into_any_element();
        }
        if page.template == ResourceWorkbenchTemplate::Tasks {
            return self.render_tasks_page(&page, cx).into_any_element();
        }

        let mut header = div()
            .px_4()
            .py_3()
            .child(div().text_xl().child(page.title.clone()));
        if let Some(error) = &self.renderer_error {
            header = header.child(
                div()
                    .mt_1()
                    .text_color(gpui::rgb(0xd7a23a))
                    .child(format!("Shell fallback: {error}")),
            );
        }
        // detail 页 links(如 Index → Mapping)。
        for (link_index, link) in page.links.iter().enumerate() {
            let target = link.page_id.clone();
            let route =
                route_binding::build_route(&link.route, &self.route, &serde_json::Value::Null);
            let title = link.title.clone();
            header = header.child(
                div()
                    .id(("link", link_index))
                    .mt_1()
                    .cursor_pointer()
                    .text_color(gpui::rgb(0x6cb6ff))
                    .child(title)
                    .on_click(cx.listener(
                        move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                            this.navigate(target.clone(), route.clone(), cx);
                        },
                    )),
            );
        }
        let state = self.page_state_snapshot();
        let body = match state {
            PageStateSnapshot::Idle => div().p_4().child("Ready").into_any_element(),
            PageStateSnapshot::Loading => div().p_4().child("Loading...").into_any_element(),
            PageStateSnapshot::Failed(error) => div()
                .p_4()
                .child(format!("Error: {error}"))
                .into_any_element(),
            PageStateSnapshot::Loaded(value) => {
                if page.template == ResourceWorkbenchTemplate::Collection {
                    self.render_collection(&page, &value, cx).into_any_element()
                } else {
                    render_json(&value).into_any_element()
                }
            }
        };
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("page-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .into_any_element()
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
        let execute_label = if self.query_running {
            "Running..."
        } else {
            "Execute"
        };
        let result_view = match &self.query_result {
            None => div().p_4().child("Enter parameters and run"),
            Some(Ok(value)) => render_json(value),
            Some(Err(error)) => div().p_4().child(format!("Error: {error}")),
        };
        let mut view = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_4()
                    .py_3()
                    .child(div().text_xl().child(page.title.clone())),
            )
            .child(
                div()
                    .px_4()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(input_state)
                    .child(
                        div()
                            .id("query-execute")
                            .px_3()
                            .py_1()
                            .cursor_pointer()
                            .bg(gpui::rgb(0x2d5a2d))
                            .child(execute_label)
                            .on_click(cx.listener(
                                move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                                    this.execute_query(cx);
                                },
                            )),
                    ),
            );
        // 危险操作确认条:确认/取消后继续或放弃本次写操作。
        if self.pending_confirm.is_some() {
            view = view.child(
                div()
                    .id("query-confirm")
                    .px_4()
                    .py_2()
                    .bg(gpui::rgb(0x3a2d1a))
                    .flex()
                    .gap_3()
                    .child("This operation may change data. Confirm?")
                    .child(
                        div()
                            .id("query-confirm-ok")
                            .px_3()
                            .py_1()
                            .cursor_pointer()
                            .bg(gpui::rgb(0x8a3a2a))
                            .child("Confirm")
                            .on_click(cx.listener(
                                move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                                    this.confirm_pending(cx);
                                },
                            )),
                    )
                    .child(
                        div()
                            .id("query-confirm-cancel")
                            .px_3()
                            .py_1()
                            .cursor_pointer()
                            .child("Cancel")
                            .on_click(cx.listener(
                                move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                                    this.cancel_pending(cx);
                                },
                            )),
                    ),
            );
        }
        view.child(
            div()
                .id("query-result")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(result_view),
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
        let tasks = self.session.task_snapshots();
        let body = if tasks.is_empty() {
            div().p_4().child("No tasks").into_any_element()
        } else {
            let mut list = div().flex().flex_col();
            for (index, task) in tasks.into_iter().enumerate() {
                let cancellable = matches!(
                    task.state,
                    extension_protocol::job::JobState::Queued
                        | extension_protocol::job::JobState::Running
                );
                let job_id = task.job_id.clone();
                let mut row = div()
                    .py_2()
                    .px_4()
                    .child(format!("{}  ({:?})", task.job_id, task.state));
                if cancellable {
                    row = row.child(
                        div()
                            .id(("cancel-task", index))
                            .ml_2()
                            .cursor_pointer()
                            .text_color(gpui::rgb(0xe06c75))
                            .child("Cancel")
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
        let mut view = div().flex_1().min_w_0().min_h_0().flex().flex_col().child(
            div()
                .px_4()
                .py_3()
                .child(div().text_xl().child(page.title.clone()))
                .child(
                    div()
                        .id("refresh-tasks")
                        .mt_1()
                        .cursor_pointer()
                        .text_color(gpui::rgb(0x6cb6ff))
                        .child("Refresh")
                        .on_click(cx.listener(
                            |_this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                                cx.notify();
                            },
                        )),
                ),
        );
        if let Some(error) = self.task_error.clone() {
            view = view.child(div().px_4().text_color(gpui::rgb(0xe06c75)).child(error));
        }
        view.child(div().flex_1().min_h_0().overflow_hidden().child(body))
    }
}

/// JSON pretty 渲染(通用,任何模板的加载结果都可用)。
fn render_json(value: &serde_json::Value) -> gpui::Div {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
    div().p_4().child(text)
}

/// 把 manifest 声明的 `/name` 路径归一为 serde_json pointer 形式。
fn pointer_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

impl NativeResourceWorkbench {
    /// collection 渲染:声明列 + 行点击导航(按 open 声明)。
    fn render_collection(
        &mut self,
        page: &ResourceWorkbenchPage,
        value: &serde_json::Value,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let Some(collection) = &page.collection else {
            return render_json(value);
        };
        let items = value
            .pointer(&pointer_path(&collection.items_path))
            .cloned()
            .and_then(|items| items.as_array().cloned())
            .unwrap_or_default();
        let column_count = collection.columns.len().max(1);
        let mut table = div().flex().flex_col();
        let mut header_row = div().flex().py_1();
        for column in &collection.columns {
            header_row = header_row.child(
                div()
                    .w(px(720.0 / column_count as f32))
                    .px_2()
                    .child(column.title.clone()),
            );
        }
        table = table.child(header_row);
        for (row_index, item) in items.into_iter().enumerate() {
            let mut row = div().flex().py_1();
            for column in &collection.columns {
                let cell = item
                    .pointer(&pointer_path(&column.path))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let text = match cell {
                    serde_json::Value::String(text) => text,
                    serde_json::Value::Null => String::new(),
                    other => other.to_string(),
                };
                row = row.child(div().w(px(720.0 / column_count as f32)).px_2().child(text));
            }
            if collection.open.is_some() {
                let clicked = item.clone();
                let clickable = div()
                    .id(("row", row_index))
                    .flex()
                    .py_1()
                    .cursor_pointer()
                    .on_click(cx.listener(
                        move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                            this.open_collection_row(clicked.clone(), cx);
                        },
                    ));
                // 重建行(带 id)保持类型一致。
                let mut stateful_row = clickable;
                for column in &collection.columns {
                    let cell = item
                        .pointer(&pointer_path(&column.path))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let text = match cell {
                        serde_json::Value::String(text) => text,
                        serde_json::Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    stateful_row = stateful_row
                        .child(div().w(px(720.0 / column_count as f32)).px_2().child(text));
                }
                table = table.child(stateful_row);
            } else {
                table = table.child(row);
            }
        }
        table
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
        div()
            .id("resource-workbench")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .child(self.render_nav(cx))
            .child(self.render_page(window, cx))
    }
}
