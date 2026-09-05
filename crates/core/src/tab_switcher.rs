use crate::tab_container::TabContainer;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext as _, Context, Entity, InteractiveElement, IntoElement, MouseButton,
    ParentElement, RenderOnce, SharedString, Styled as _, Task, Window, div, px,
};
use gpui_component::list::{List, ListDelegate, ListState};
use gpui_component::{
    ActiveTheme, Icon, IconName, IndexPath, Selectable, Sizable, Size, WindowExt as _, h_flex,
};
use rust_i18n::t;
use std::sync::Arc;

pub type QuickOpenAction = Arc<dyn Fn(&mut Window, &mut App) + Send + Sync>;
pub type QuickOpenResolver = Arc<dyn Fn(&str) -> Option<(SharedString, QuickOpenAction)> + Send + Sync>;

const SWITCHER_WIDTH: f32 = 640.0;
const SWITCHER_MAX_HEIGHT: f32 = 420.0;

#[derive(Clone)]
pub struct TabSwitcherEntry {
    pub index: usize,
    pub pinned: bool,
    pub title: SharedString,
    pub icon: Option<Icon>,
    pub active: bool,
}

pub fn filter_tab_switcher_entries(
    entries: &[TabSwitcherEntry],
    query: &str,
) -> Vec<TabSwitcherEntry> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return entries.to_vec();
    }
    entries
        .iter()
        .filter(|entry| entry.title.to_lowercase().contains(&query))
        .cloned()
        .collect()
}

pub fn open_tab_switcher_dialog(
    container: Entity<TabContainer>,
    entries: Vec<TabSwitcherEntry>,
    supports_quick_open: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let search_placeholder = if supports_quick_open {
        t!("TabSwitcher.search_or_connect").to_string()
    } else {
        t!("TabSwitcher.search").to_string()
    };
    let active_row = entries
        .iter()
        .position(|entry| entry.active)
        .unwrap_or_default();
    let list = cx.new(|cx| {
        let mut list = ListState::new(TabSwitcherDelegate::new(container, entries), window, cx)
            .searchable(true);
        list.set_selected_index(Some(IndexPath::new(active_row)), window, cx);
        list
    });
    let dialog_list = list.clone();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .w(px(SWITCHER_WIDTH))
            .margin_top(px(72.0))
            .close_button(false)
            .title(t!("TabSwitcher.title").to_string())
            .content({
                let list = dialog_list.clone();
                let search_placeholder = search_placeholder.clone();
                move |content, _window, _cx| {
                    content.p_0().child(
                        div().id("tab-switcher-dialog").child(
                            List::new(&list)
                                .search_placeholder(search_placeholder.clone())
                                .with_size(Size::Large)
                                .max_h(px(SWITCHER_MAX_HEIGHT)),
                        ),
                    )
                }
            })
    });
    list.update(cx, |list, cx| list.focus(window, cx));
}

pub struct TabSwitcherDelegate {
    container: Entity<TabContainer>,
    entries: Vec<TabSwitcherEntry>,
    filtered_entries: Vec<TabSwitcherEntry>,
    selected_index: Option<IndexPath>,
    quick_open: Option<(SharedString, QuickOpenAction)>,
}

impl TabSwitcherDelegate {
    fn new(container: Entity<TabContainer>, entries: Vec<TabSwitcherEntry>) -> Self {
        Self {
            container,
            filtered_entries: entries.clone(),
            entries,
            selected_index: None,
            quick_open: None,
        }
    }
}

impl ListDelegate for TabSwitcherDelegate {
    type Item = TabSwitcherItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.filtered_entries = filter_tab_switcher_entries(&self.entries, query);
        self.quick_open = self.container.read(cx).tab_quick_open(query);
        if self.quick_open.is_some() {
            self.filtered_entries.clear();
            self.selected_index = Some(IndexPath::new(0));
        }
        cx.notify();
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.filtered_entries.len() + usize::from(self.quick_open.is_some())
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        if let Some((title, action)) = &self.quick_open {
            return (ix.row == 0).then(|| {
                let mut item = TabSwitcherItem::new(TabSwitcherEntry {
                    index: 0, pinned: false, title: title.clone(), icon: None, active: false,
                }, self.container.clone(), self.selected_index == Some(ix));
                item.quick_open = Some(action.clone());
                item
            });
        }
        let entry = self.filtered_entries.get(ix.row)?.clone();
        Some(TabSwitcherItem::new(
            entry,
            self.container.clone(),
            self.selected_index == Some(ix),
        ))
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
        if let Some((_, action)) = &self.quick_open {
            window.close_dialog(cx);
            action(window, cx);
            return;
        }
        let Some(ix) = self.selected_index else {
            return;
        };
        let Some(entry) = self.filtered_entries.get(ix.row) else {
            return;
        };
        activate_entry(&self.container, entry, window, cx);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        window.close_dialog(cx);
    }
}

#[derive(IntoElement)]
pub struct TabSwitcherItem {
    entry: TabSwitcherEntry,
    container: Entity<TabContainer>,
    selected: bool,
    quick_open: Option<QuickOpenAction>,
}

impl TabSwitcherItem {
    fn new(entry: TabSwitcherEntry, container: Entity<TabContainer>, selected: bool) -> Self {
        Self {
            entry,
            container,
            selected,
            quick_open: None,
        }
    }
}

impl Selectable for TabSwitcherItem {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for TabSwitcherItem {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let container = self.container.clone();
        let entry = self.entry.clone();
        let selected = self.selected || self.entry.active;
        h_flex()
            .id(SharedString::from(format!(
                "tab-switcher-item-{}-{}",
                entry.pinned, entry.index
            )))
            .h(px(44.0))
            .mx_2()
            .px_3()
            .rounded(px(6.0))
            .items_center()
            .gap_3()
            .cursor_pointer()
            .text_color(cx.theme().foreground)
            .when(selected, |el| el.bg(cx.theme().list_active))
            .when(!selected, |el| {
                el.text_color(cx.theme().muted_foreground)
                    .hover(|style| style.bg(cx.theme().list_hover))
            })
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                if let Some(action) = &self.quick_open {
                    window.close_dialog(cx);
                    action(window, cx);
                } else {
                    activate_entry(&container, &entry, window, cx);
                }
            })
            .child(render_entry_icon(self.entry.icon, selected, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_sm()
                    .child(self.entry.title),
            )
    }
}

fn render_entry_icon(icon: Option<Icon>, selected: bool, cx: &App) -> impl IntoElement {
    let color = if selected {
        cx.theme().foreground
    } else {
        cx.theme().muted_foreground
    };
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .child(match icon {
            Some(icon) => Icon::new(icon).with_size(Size::Small).text_color(color),
            None => Icon::new(IconName::Plus)
                .with_size(Size::Small)
                .text_color(color),
        })
}

fn activate_entry(
    container: &Entity<TabContainer>,
    entry: &TabSwitcherEntry,
    window: &mut Window,
    cx: &mut App,
) {
    container.update(cx, |container, cx| {
        if entry.pinned {
            container.activate_pinned_tab_at(entry.index, window, cx);
        } else {
            container.set_active_index(entry.index, window, cx);
        }
    });
    window.close_dialog(cx);
}

#[cfg(test)]
mod tests {
    use super::{TabSwitcherEntry, filter_tab_switcher_entries};
    use gpui::SharedString;

    fn entry(index: usize, title: &str) -> TabSwitcherEntry {
        TabSwitcherEntry {
            index,
            pinned: false,
            title: SharedString::from(title.to_string()),
            icon: None,
            active: false,
        }
    }

    #[test]
    fn tab_switcher_filter_matches_case_insensitively_and_handles_blank_query() {
        let entries = vec![
            entry(0, "Vaults"),
            entry(1, "SFTP"),
            entry(2, "V8生产CoMi"),
            entry(3, "New Tab"),
        ];

        let filtered = filter_tab_switcher_entries(&entries, "tab");

        assert_eq!(
            vec![3],
            filtered.iter().map(|entry| entry.index).collect::<Vec<_>>()
        );
        let filtered = filter_tab_switcher_entries(&entries, "   ");

        assert_eq!(
            vec![0, 1, 2, 3],
            filtered.iter().map(|entry| entry.index).collect::<Vec<_>>()
        );
    }
}
