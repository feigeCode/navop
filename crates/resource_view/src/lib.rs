//! Native resource workbench primitives.
//!
//! Renderer state only: 页面只保存导航/加载/结果状态和会话 handle,
//! 所有 provider I/O 经 `extension_plugin_adapter::workbench_dispatch`
//! 的命名操作入口,连接主会话由宿主 tab 唯一持有。

pub mod custom_page_host;

pub use custom_page_host::{
    CustomPageHost, DirtyGuardDecision, GlobalCustomPageHost, MountHandle, PageRenderer,
    ShellMountError, ShellPageMount, ShellPageMountRequest, custom_page_host, resolve_renderer,
};

use extension_plugin_adapter::{
    BindingContext, ResourceSessionHandle, WorkbenchDispatchError,
    dispatch_invoke_scoped as dispatch_invoke,
};
use extension_runtime::RegisteredResourceWorkbenchContribution;
use extension_runtime::extension::manifest::{
    ResourceWorkbenchCollection, ResourceWorkbenchPage, ResourceWorkbenchTemplate,
};
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px,
};

/// 页面事件:通知宿主 tab 页面切换/加载状态变化。
pub struct WorkbenchEvent;

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
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageSelection {
    pub page_id: String,
    pub route: serde_json::Value,
}

impl NativeResourceWorkbench {
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
        };
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
        self.selected_page = page_id;
        self.route = serde_json::Value::Null;
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
        self.selected_page = page_id;
        self.route = route;
        self.load_current_page(cx);
    }

    fn current_page(&self) -> Option<&ResourceWorkbenchPage> {
        self.descriptor
            .pages
            .iter()
            .find(|page| page.id == self.selected_page)
    }

    /// 触发当前页面的 load 操作。tasks 页面读宿主任务,不发 provider 请求。
    fn load_current_page(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.current_page() else {
            self.page_state = PageState::Failed("unknown workbench page".into());
            cx.notify();
            return;
        };
        if page.template == ResourceWorkbenchTemplate::Tasks {
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
                    async move { dispatch_invoke(&scope, &workbench, &operation, &context).await },
                )
                .await
                .unwrap_or_else(|join_error| {
                    Err(WorkbenchDispatchError::Provider(join_error.to_string()))
                });
            let _ = this.update(cx, |this, cx| {
                // 迟到结果防护:仅当请求未变更时应用。
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

    fn render_page(&self) -> impl IntoElement + use<> {
        let Some(page) = self.current_page() else {
            return div().p_4().child("Unknown page");
        };
        let header = div()
            .px_4()
            .py_3()
            .child(div().text_xl().child(page.title.clone()));
        let body = match &self.page_state {
            PageState::Idle => div().p_4().child("Ready"),
            PageState::Loading => div().p_4().child("Loading..."),
            PageState::Failed(error) => div().p_4().child(format!("Error: {error}")),
            PageState::Loaded(value) => render_value(page, value),
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
    }
}

/// 按模板渲染已加载结果:collection 渲染声明列,json 原样输出。
fn render_value(page: &ResourceWorkbenchPage, value: &serde_json::Value) -> gpui::Div {
    if page.template == ResourceWorkbenchTemplate::Collection {
        if let Some(collection) = &page.collection {
            return render_collection(collection, value);
        }
    }
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
    div().p_4().child(text)
}

fn render_collection(
    collection: &ResourceWorkbenchCollection,
    value: &serde_json::Value,
) -> gpui::Div {
    let items = value
        .pointer(normalize_pointer(&collection.items_path))
        .cloned()
        .and_then(|items| items.as_array().cloned())
        .unwrap_or_default();
    let mut table = div().flex().flex_col();
    let mut header_row = div().flex().py_1();
    for column in &collection.columns {
        header_row = header_row.child(div().w(px(240.)).px_2().child(column.title.clone()));
    }
    table = table.child(header_row);
    for item in items {
        let mut row = div().flex().py_1();
        for column in &collection.columns {
            let cell = item
                .pointer(normalize_pointer(&column.path))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let text = match cell {
                serde_json::Value::String(text) => text,
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            };
            row = row.child(div().w(px(240.)).px_2().child(text));
        }
        table = table.child(row);
    }
    table
}

fn normalize_pointer(path: &str) -> &str {
    path.strip_prefix('/').unwrap_or(path)
}

impl EventEmitter<WorkbenchEvent> for NativeResourceWorkbench {}

impl Focusable for NativeResourceWorkbench {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NativeResourceWorkbench {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("resource-workbench")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .child(self.render_nav(cx))
            .child(self.render_page())
    }
}
