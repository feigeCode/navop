use std::sync::Arc;

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::{ActiveTheme, Icon, IconName, IconSize, Sizable, v_flex};
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;

use crate::state::MarketplaceLoadState;
use crate::{ExtensionKind, ExtensionSummary, ExtensionViewHost, MarketplaceEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionManagerMode {
    Installed,
    Marketplace,
}

pub struct ExtensionManagerView {
    pub(crate) host: Arc<dyn ExtensionViewHost>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) mode: ExtensionManagerMode,
    pub(crate) search: Entity<InputState>,
    pub(crate) selected_kind: Option<ExtensionKind>,
    pub(crate) updates_only: bool,
    pub(crate) installed: Vec<ExtensionSummary>,
    pub(crate) marketplace_entries: Vec<MarketplaceEntry>,
    pub(crate) marketplace_load_attempted: bool,
    pub(crate) marketplace_load_state: MarketplaceLoadState,
    pub(crate) busy: Option<String>,
    pub(crate) status: SharedString,
    pub(crate) _subscriptions: Vec<Subscription>,
}

impl ExtensionManagerView {
    pub fn new(
        host: Arc<dyn ExtensionViewHost>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_mode(
            host,
            ExtensionManagerMode::Installed,
            String::new(),
            window,
            cx,
        )
    }

    pub fn new_marketplace_search(
        host: Arc<dyn ExtensionViewHost>,
        query: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_mode(
            host,
            ExtensionManagerMode::Marketplace,
            query.into(),
            window,
            cx,
        )
    }

    /// 切换到扩展市场并只显示有可用更新的扩展。
    ///
    /// 供更新通知的“查看更新”入口复用：无论页签是新建还是已打开，都保证
    /// 清空搜索与分类过滤，避免旧过滤条件遮住需要更新的扩展。
    pub fn show_updates_only(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = ExtensionManagerMode::Marketplace;
        self.updates_only = true;
        self.selected_kind = None;
        self.search.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.ensure_marketplace_loaded(cx);
        cx.notify();
    }

    pub(crate) fn set_mode(&mut self, mode: ExtensionManagerMode, cx: &mut Context<Self>) {
        self.mode = mode;
        // 已安装列表无法判断更新状态，切回已安装页签时关闭“有更新”过滤
        if mode == ExtensionManagerMode::Installed {
            self.updates_only = false;
        }
        self.ensure_marketplace_loaded(cx);
        cx.notify();
    }

    fn new_with_mode(
        host: Arc<dyn ExtensionViewHost>,
        mode: ExtensionManagerMode,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("Extension.search").to_string())
                .default_value(query)
        });
        let search_sub =
            cx.subscribe_in(
                &search,
                window,
                |view, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => {
                        view.load_marketplace_from_search_if_manifest_url(cx);
                    }
                    _ => {}
                },
            );
        let mut view = Self {
            host,
            focus_handle: cx.focus_handle(),
            mode,
            search,
            selected_kind: None,
            updates_only: false,
            installed: Vec::new(),
            marketplace_entries: Vec::new(),
            marketplace_load_attempted: false,
            marketplace_load_state: MarketplaceLoadState::NotLoaded,
            busy: None,
            status: SharedString::from(""),
            _subscriptions: vec![search_sub],
        };
        view.refresh_installed(cx);
        view
    }
}

impl Focusable for ExtensionManagerView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<()> for ExtensionManagerView {}

impl EventEmitter<TabContentEvent> for ExtensionManagerView {}

impl TabContent for ExtensionManagerView {
    fn content_key(&self) -> &'static str {
        "Extensions"
    }

    fn title(&self, _cx: &App) -> SharedString {
        SharedString::from(t!("Extension.tab_title").to_string())
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(
            Icon::new(IconName::ExtensionsColor)
                .with_size(IconSize::Medium)
                .color(),
        )
    }
}

impl Render for ExtensionManagerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus_handle).size_full().child(
            v_flex()
                .size_full()
                .bg(cx.theme().background)
                .child(self.render_toolbar(window, cx))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .p_4()
                        .child(self.render_body(window, cx)),
                ),
        )
    }
}
