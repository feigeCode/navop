use gpui::{
    App, Context, FontWeight, IntoElement as _, ParentElement as _, SharedString, Styled as _,
    Task, Window, div, px,
};
use gpui_component::{
    Icon, IconName, IconSize, IndexPath, Sizable as _, WindowExt as _, h_flex,
    list::{ListDelegate, ListItem, ListState},
};

use super::{NavigationActivate, NavigationApplication, NavigationQuickOpenItem, NavigationTarget};

const ITEM_HEIGHT: gpui::Pixels = px(44.0);
const ITEM_RADIUS: gpui::Pixels = px(6.0);

pub(super) struct NavigationQuickOpenDelegate {
    items: Vec<NavigationQuickOpenItem>,
    filtered_items: Vec<NavigationQuickOpenItem>,
    selected_index: Option<IndexPath>,
    search_query: String,
    on_activate: NavigationActivate,
}

impl NavigationQuickOpenDelegate {
    pub(super) fn new(
        items: Vec<NavigationQuickOpenItem>,
        on_activate: NavigationActivate,
    ) -> Self {
        Self {
            filtered_items: items.clone(),
            items,
            selected_index: None,
            search_query: String::new(),
            on_activate,
        }
    }

    fn apply_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_items = self.items.clone();
            return;
        }
        let query = self.search_query.to_lowercase();
        self.filtered_items = self
            .items
            .iter()
            .filter(|item| item_matches_query(item, &query))
            .cloned()
            .collect();
    }

    fn activate(&self, target: NavigationTarget, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
        (self.on_activate)(target, window, cx);
    }
}

pub(super) fn item_matches_query(item: &NavigationQuickOpenItem, query: &str) -> bool {
    item.label.to_lowercase().contains(&query.to_lowercase())
}

impl ListDelegate for NavigationQuickOpenDelegate {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.search_query = query.to_string();
        self.apply_filter();
        cx.notify();
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.filtered_items.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let item = self.filtered_items.get(ix.row)?.clone();
        let target = item.target;
        let on_activate = self.on_activate.clone();
        Some(
            ListItem::new(ix)
                .mx_2()
                .h(ITEM_HEIGHT)
                .px_3()
                .rounded(ITEM_RADIUS)
                .check_icon(IconName::Check)
                .confirmed(item.selected)
                .on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    on_activate(target, window, cx);
                })
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_3()
                        .child(render_target_icon(target))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(SharedString::from(item.label)),
                        ),
                ),
        )
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
    }

    fn confirm(
        &mut self,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        let Some(ix) = self.selected_index else {
            return;
        };
        let Some(item) = self.filtered_items.get(ix.row) else {
            return;
        };
        self.activate(item.target, window, cx);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        window.close_dialog(cx);
    }
}

fn render_target_icon(target: NavigationTarget) -> gpui::AnyElement {
    let icon = match target {
        NavigationTarget::Connection(connection_type) => connection_type.icon(),
        NavigationTarget::Application(application) => application_icon(application),
    };
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .child(Icon::new(icon).color().with_size(IconSize::Medium))
        .into_any_element()
}

fn application_icon(application: NavigationApplication) -> IconName {
    match application {
        NavigationApplication::AiWorkbench => IconName::AILine,
        NavigationApplication::Team => IconName::TeamLine,
        NavigationApplication::Notes => IconName::NotesLine,
        #[cfg(feature = "api-testing")]
        NavigationApplication::ApiTesting => IconName::Globe,
        NavigationApplication::JsonFormatter => IconName::Json,
        NavigationApplication::SessionLogs => IconName::Terminal,
        NavigationApplication::CredentialVault => IconName::Key,
        NavigationApplication::Extensions => IconName::ExtensionsLine,
        NavigationApplication::Settings => IconName::Settings,
    }
}
