use std::collections::HashSet;

use gpui::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, Styled, Task, Window, div,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, IndexPath, Sizable, StyledExt,
    button::Button,
    checkbox::Checkbox,
    h_flex,
    input::InputState,
    list::{List, ListDelegate, ListItem, ListState},
    v_flex,
};
use rust_i18n::t;

pub(super) type TableSelectionListState = Entity<ListState<TableSelectionListDelegate>>;

const TABLE_ROW_HEIGHT: f32 = 34.0;

pub(super) fn table_selection_list_state<T: 'static>(
    selected_tables: Entity<HashSet<String>>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> TableSelectionListState {
    cx.new(|cx| {
        ListState::new(TableSelectionListDelegate::new(selected_tables), window, cx)
            .selectable(false)
    })
}

pub(super) fn refresh_table_selection_list_app(
    list_state: &TableSelectionListState,
    selected_tables: &Entity<HashSet<String>>,
    tables: Vec<String>,
    preferred: String,
    cx: &mut App,
) {
    let current = selected_tables.read(cx).clone();
    let selected = selected_after_refresh(&current, &tables, &preferred);
    selected_tables.update(cx, |slot, cx| {
        *slot = selected;
        cx.notify();
    });
    list_state.update(cx, |list, cx| {
        list.delegate_mut().set_tables(tables);
        cx.notify();
    });
}

pub(super) fn clear_table_selection_list<T: 'static>(
    list_state: &TableSelectionListState,
    selected_tables: &Entity<HashSet<String>>,
    cx: &mut Context<T>,
) {
    selected_tables.update(cx, |selected, cx| {
        selected.clear();
        cx.notify();
    });
    list_state.update(cx, |list, cx| {
        list.delegate_mut().set_tables(Vec::new());
        cx.notify();
    });
}

pub(super) fn ordered_selected_table_names<T>(
    list_state: &TableSelectionListState,
    selected_tables: &Entity<HashSet<String>>,
    fallback: &Entity<InputState>,
    cx: &Context<T>,
) -> Vec<String> {
    let selected = selected_tables.read(cx);
    let ordered = list_state
        .read(cx)
        .delegate()
        .tables()
        .iter()
        .filter(|table| selected.contains(*table))
        .cloned()
        .collect::<Vec<_>>();
    if ordered.is_empty() {
        split_table_names(&fallback.read(cx).text().to_string())
    } else {
        ordered
    }
}

pub(super) fn table_selection_panel(
    title: String,
    list_state: TableSelectionListState,
    selected_tables: Entity<HashSet<String>>,
    cx: &App,
) -> impl IntoElement {
    let all_tables = list_state.read(cx).delegate().tables().to_vec();
    let button_scope = title.to_lowercase().replace(' ', "-");

    v_flex()
        .flex_1()
        .min_h_0()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .child(div().text_sm().font_semibold().child(title))
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new(format!("table-select-all-{button_scope}"))
                                .small()
                                .child(t!("Common.select_all").to_string())
                                .on_click({
                                    let selected_tables = selected_tables.clone();
                                    let all_tables = all_tables.clone();
                                    move |_, _, cx| {
                                        selected_tables.update(cx, |selected, cx| {
                                            *selected = all_tables.iter().cloned().collect();
                                            cx.notify();
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new(format!("table-select-none-{button_scope}"))
                                .small()
                                .child(t!("Common.deselect_all").to_string())
                                .on_click({
                                    let selected_tables = selected_tables.clone();
                                    move |_, _, cx| {
                                        selected_tables.update(cx, |selected, cx| {
                                            selected.clear();
                                            cx.notify();
                                        });
                                    }
                                }),
                        ),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .overflow_hidden()
                .child(List::new(&list_state).size_full()),
        )
}

pub(super) struct TableSelectionListDelegate {
    tables: Vec<String>,
    selected_tables: Entity<HashSet<String>>,
    selected_index: Option<IndexPath>,
}

impl TableSelectionListDelegate {
    fn new(selected_tables: Entity<HashSet<String>>) -> Self {
        Self {
            tables: Vec::new(),
            selected_tables,
            selected_index: None,
        }
    }

    fn set_tables(&mut self, tables: Vec<String>) {
        self.tables = tables;
        self.selected_index = None;
    }

    fn tables(&self) -> &[String] {
        &self.tables
    }
}

impl ListDelegate for TableSelectionListDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.tables.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let table = self.tables.get(ix.row)?;
        let checked = self.selected_tables.read(cx).contains(table);
        Some(table_row(table, checked, self.selected_tables.clone(), cx))
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .p_3()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(t!("Compare.no_tables").to_string())
    }

    fn perform_search(
        &mut self,
        _query: &str,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        Task::ready(())
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
    }
}

fn table_row(
    table: &str,
    checked: bool,
    selected_tables: Entity<HashSet<String>>,
    cx: &App,
) -> ListItem {
    let table_name = table.to_string();
    let row = h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .child(Checkbox::new(format!("table-{table_name}")).checked(checked))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .child(table_name.clone()),
        );

    ListItem::new(format!("table-row-{table_name}"))
        .h(px(TABLE_ROW_HEIGHT))
        .child(row)
        .when(checked, |this| this.bg(cx.theme().list_active))
        .on_click(move |_, _, cx| {
            selected_tables.update(cx, |selected, cx| {
                if selected.contains(&table_name) {
                    selected.remove(&table_name);
                } else {
                    selected.insert(table_name.clone());
                }
                cx.notify();
            });
        })
}

fn selected_after_refresh(
    current: &HashSet<String>,
    tables: &[String],
    preferred: &str,
) -> HashSet<String> {
    let mut selected = tables
        .iter()
        .filter(|table| current.contains(*table))
        .cloned()
        .collect::<HashSet<_>>();
    if selected.is_empty() {
        if let Some(preferred) = find_table_case_insensitive(tables, preferred) {
            selected.insert(preferred);
        }
    }
    selected
}

fn find_table_case_insensitive(tables: &[String], preferred: &str) -> Option<String> {
    let preferred = preferred.trim().to_lowercase();
    if preferred.is_empty() {
        return None;
    }
    tables
        .iter()
        .find(|table| table.to_lowercase() == preferred)
        .cloned()
}

fn split_table_names(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|table| !table.is_empty())
        .map(ToString::to_string)
        .collect()
}
