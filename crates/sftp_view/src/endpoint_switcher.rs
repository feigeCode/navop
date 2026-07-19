use crate::{SftpView, endpoint::LeftEndpointValue};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Context, Entity, InteractiveElement, IntoElement, MouseButton,
    ParentElement, RenderOnce, SharedString, Styled as _, Task, Window, div, px,
};
use gpui_component::list::{List, ListDelegate, ListState};
use gpui_component::{
    ActiveTheme, Icon, IconName, IndexPath, Selectable, Sizable, Size, WindowExt as _, h_flex,
};
use rust_i18n::t;

const SWITCHER_WIDTH: f32 = 520.0;
const SWITCHER_MAX_HEIGHT: f32 = 420.0;

#[derive(Clone)]
pub(crate) struct EndpointSwitcherEntry {
    pub value: LeftEndpointValue,
    pub title: SharedString,
    pub icon: IconName,
    pub active: bool,
}

impl EndpointSwitcherEntry {
    fn id(&self) -> SharedString {
        match self.value {
            LeftEndpointValue::Local => "local".into(),
            LeftEndpointValue::Remote(id) => format!("remote-{id}").into(),
        }
    }
}

pub(crate) fn filter_endpoint_switcher_entries(
    entries: &[EndpointSwitcherEntry],
    query: &str,
) -> Vec<EndpointSwitcherEntry> {
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

pub(crate) fn open_endpoint_switcher_dialog(
    view: Entity<SftpView>,
    entries: Vec<EndpointSwitcherEntry>,
    window: &mut Window,
    cx: &mut App,
) {
    let active_row = entries
        .iter()
        .position(|entry| entry.active)
        .unwrap_or_default();
    let list = cx.new(|cx| {
        let mut list = ListState::new(EndpointSwitcherDelegate::new(view, entries), window, cx)
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
            .title(t!("Endpoint.switch_title").to_string())
            .content({
                let list = dialog_list.clone();
                move |content, _window, _cx| {
                    content.p_0().child(
                        div().id("endpoint-switcher-dialog").child(
                            List::new(&list)
                                .search_placeholder(t!("Endpoint.search").to_string())
                                .with_size(Size::Large)
                                .max_h(px(SWITCHER_MAX_HEIGHT)),
                        ),
                    )
                }
            })
    });
    list.update(cx, |list, cx| list.focus(window, cx));
}

struct EndpointSwitcherDelegate {
    view: Entity<SftpView>,
    entries: Vec<EndpointSwitcherEntry>,
    filtered_entries: Vec<EndpointSwitcherEntry>,
    selected_index: Option<IndexPath>,
}

impl EndpointSwitcherDelegate {
    fn new(view: Entity<SftpView>, entries: Vec<EndpointSwitcherEntry>) -> Self {
        Self {
            view,
            filtered_entries: entries.clone(),
            entries,
            selected_index: None,
        }
    }
}

impl ListDelegate for EndpointSwitcherDelegate {
    type Item = EndpointSwitcherItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.filtered_entries = filter_endpoint_switcher_entries(&self.entries, query);
        cx.notify();
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.filtered_entries.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let entry = self.filtered_entries.get(ix.row)?.clone();
        Some(EndpointSwitcherItem::new(
            entry,
            self.view.clone(),
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
        let Some(ix) = self.selected_index else {
            return;
        };
        let Some(entry) = self.filtered_entries.get(ix.row) else {
            return;
        };
        activate_entry(&self.view, entry, window, cx);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        window.close_dialog(cx);
    }
}

#[derive(IntoElement)]
struct EndpointSwitcherItem {
    entry: EndpointSwitcherEntry,
    view: Entity<SftpView>,
    selected: bool,
}

impl EndpointSwitcherItem {
    fn new(entry: EndpointSwitcherEntry, view: Entity<SftpView>, selected: bool) -> Self {
        Self {
            entry,
            view,
            selected,
        }
    }
}

impl Selectable for EndpointSwitcherItem {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for EndpointSwitcherItem {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = self.view.clone();
        let entry = self.entry.clone();
        let selected = self.selected || entry.active;

        h_flex()
            .id(SharedString::from(format!(
                "endpoint-switcher-item-{}",
                entry.id()
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
                activate_entry(&view, &entry, window, cx);
            })
            .child(match &self.entry.value {
                LeftEndpointValue::Local => Icon::new(self.entry.icon.clone())
                    .with_size(px(24.0))
                    .text_color(if selected {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    }),
                LeftEndpointValue::Remote(_) => Icon::new(self.entry.icon.clone())
                    .color()
                    .with_size(px(24.0))
                    .text_color(gpui::rgb(0x8b5cf6)),
            })
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

fn activate_entry(
    view: &Entity<SftpView>,
    entry: &EndpointSwitcherEntry,
    window: &mut Window,
    cx: &mut App,
) {
    view.update(cx, |view, cx| {
        view.switch_left_endpoint(entry.value.clone(), window, cx);
    });
    window.close_dialog(cx);
}

#[cfg(test)]
mod tests {
    use super::{EndpointSwitcherEntry, filter_endpoint_switcher_entries};
    use crate::endpoint::LeftEndpointValue;
    use gpui_component::IconName;

    fn entry(value: LeftEndpointValue, title: &str) -> EndpointSwitcherEntry {
        let icon = match value {
            LeftEndpointValue::Local => IconName::HardDrive,
            LeftEndpointValue::Remote(_) => IconName::TerminalColor,
        };
        EndpointSwitcherEntry {
            value,
            title: title.to_string().into(),
            icon,
            active: false,
        }
    }

    #[test]
    fn endpoint_switcher_filter_matches_case_insensitively_and_handles_blank_query() {
        let entries = vec![
            entry(LeftEndpointValue::Local, "Local"),
            entry(LeftEndpointValue::Remote(1), "Production (prod.internal)"),
            entry(LeftEndpointValue::Remote(2), "Staging"),
        ];

        let filtered = filter_endpoint_switcher_entries(&entries, "PROD");
        assert_eq!(1, filtered.len());
        assert_eq!(LeftEndpointValue::Remote(1), filtered[0].value);

        let filtered = filter_endpoint_switcher_entries(&entries, "   ");
        assert_eq!(3, filtered.len());
    }
}
