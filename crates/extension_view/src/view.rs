use std::sync::Arc;

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::{ActiveTheme, Icon, IconName, v_flex};
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;

use crate::{ExtensionSummary, ExtensionViewHost, MarketplaceEntry};

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
    pub(crate) installed: Vec<ExtensionSummary>,
    pub(crate) marketplace_entries: Vec<MarketplaceEntry>,
    pub(crate) marketplace_load_attempted: bool,
    pub(crate) loading: bool,
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
        let search = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("Extension.search").to_string()));
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
            mode: ExtensionManagerMode::Installed,
            search,
            installed: Vec::new(),
            marketplace_entries: Vec::new(),
            marketplace_load_attempted: false,
            loading: false,
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
        Some(IconName::GalleryVerticalEnd.color())
    }
}

impl Render for ExtensionManagerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus_handle).size_full().child(
            v_flex()
                .size_full()
                .gap_4()
                .p_4()
                .bg(cx.theme().background)
                .child(self.render_toolbar(window, cx))
                .child(div().flex_1().min_h_0().child(self.render_body(cx))),
        )
    }
}
