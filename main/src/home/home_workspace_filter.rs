use std::collections::HashSet;

use crate::home_tab::HomePage;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, IndexPath, Sizable, Size, WindowExt,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState, MaskPattern},
    list::{ListDelegate, ListItem, ListState},
    tooltip::Tooltip,
    v_flex,
};
use one_core::storage::{StoredConnection, Workspace};
use rust_i18n::t;

pub(crate) fn show_workspace_dialog(
    parent: Entity<HomePage>,
    workspace_id: Option<i64>,
    initial_name: String,
    initial_sort_order: Option<i32>,
    window: &mut Window,
    cx: &mut App,
) {
    let name_input = cx.new(|cx| {
        let mut state = InputState::new(window, cx)
            .placeholder(t!("Workspace.name_placeholder"))
            .clean_on_escape();
        if !initial_name.is_empty() {
            state.set_value(initial_name.clone(), window, cx);
        }
        state
    });
    let sort_order_input = cx.new(|cx| {
        let mut state = InputState::new(window, cx)
            .placeholder(t!("Workspace.sort_order"))
            .mask_pattern(MaskPattern::number(None))
            .clean_on_escape();
        if let Some(sort_order) = initial_sort_order {
            state.set_value(sort_order.to_string(), window, cx);
        }
        state
    });

    name_input.update(cx, |state, cx| {
        state.focus(window, cx);
    });

    let input_for_render = name_input.clone();
    let input_for_ok = name_input.clone();
    let sort_input_for_render = sort_order_input.clone();
    let sort_input_for_ok = sort_order_input.clone();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let parent_for_ok = parent.clone();
        let input_for_ok = input_for_ok.clone();
        let sort_input_for_ok = sort_input_for_ok.clone();
        dialog
            .title(
                if workspace_id.is_some() {
                    t!("Workspace.edit").to_string()
                } else {
                    t!("Workspace.new").to_string()
                }
                .into_any_element(),
            )
            .child(
                v_flex()
                    .gap_3()
                    .w(px(360.0))
                    .child(Input::new(&input_for_render).w_full())
                    .child(Input::new(&sort_input_for_render).w_full()),
            )
            .confirm()
            .on_ok(move |_, _, cx| {
                let name = input_for_ok.read(cx).text().to_string().trim().to_string();
                if name.is_empty() {
                    return false;
                }
                let sort_text = sort_input_for_ok
                    .read(cx)
                    .text()
                    .to_string()
                    .trim()
                    .to_string();
                let sort_order = if sort_text.is_empty() {
                    None
                } else if let Ok(sort_order) = sort_text.parse::<i32>() {
                    Some(sort_order)
                } else {
                    return false;
                };

                let _ = parent_for_ok.update(cx, |home, cx| {
                    home.handle_save_workspace(workspace_id, name, sort_order, cx);
                });
                true
            })
    });
}

#[derive(Clone)]
struct WorkspaceFilterItem {
    id: i64,
    name: String,
    count: usize,
    checked: bool,
    sort_order: Option<i32>,
}

#[derive(Clone)]
struct DragWorkspace {
    source_id: i64,
    name: String,
    count: usize,
}

impl Render for DragWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("drag-workspace")
            .cursor_grabbing()
            .w(px(240.0))
            .px_3()
            .py_2()
            .items_center()
            .gap_2()
            .rounded(px(6.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md()
            .child(IconName::AppsColor.color().with_size(px(18.0)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(self.name.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("({})", self.count)),
            )
    }
}

pub(crate) struct WorkspaceFilterDelegate {
    parent: Entity<HomePage>,
    items: Vec<WorkspaceFilterItem>,
    search_query: String,
}

impl WorkspaceFilterDelegate {
    pub(crate) fn new(parent: Entity<HomePage>) -> Self {
        Self {
            parent,
            items: Vec::new(),
            search_query: String::new(),
        }
    }

    pub(crate) fn update_items_with_data(
        &mut self,
        workspaces: &[Workspace],
        connections: &[StoredConnection],
        filtered_ids: &HashSet<i64>,
    ) {
        self.items = workspaces
            .iter()
            .filter_map(|ws| {
                let id = ws.id?;
                let count = connections
                    .iter()
                    .filter(|c| c.workspace_id == Some(id))
                    .count();
                let checked = filtered_ids.is_empty() || filtered_ids.contains(&id);

                if self.search_query.is_empty()
                    || ws
                        .name
                        .to_lowercase()
                        .contains(&self.search_query.to_lowercase())
                {
                    Some(WorkspaceFilterItem {
                        id,
                        name: ws.name.clone(),
                        count,
                        checked,
                        sort_order: ws.sort_order,
                    })
                } else {
                    None
                }
            })
            .collect();
    }

    fn filtered_items(&self) -> &[WorkspaceFilterItem] {
        &self.items
    }
}

impl ListDelegate for WorkspaceFilterDelegate {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.search_query = query.to_string();
        let parent = self.parent.read(cx);
        self.update_items_with_data(
            &parent.workspaces,
            &parent.connections,
            &parent.filtered_workspace_ids,
        );
        cx.notify();
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.items.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let item = self.filtered_items().get(ix.row)?.clone();
        let parent = self.parent.clone();
        let parent_for_edit = self.parent.clone();
        let parent_for_delete = self.parent.clone();
        let item_id = item.id;
        let item_id_for_edit = item.id;
        let item_name_for_edit = item.name.clone();
        let item_sort_order_for_edit = item.sort_order;
        let item_id_for_delete = item.id;
        let item_id_for_drop = item.id;
        let group_name: SharedString = format!("workspace-item-{}", item.id).into();
        let drag_enabled = self.search_query.is_empty();
        let drag_border_color = cx.theme().drag_border;

        let list_item =
            ListItem::new(ix)
                .px_3()
                .py_2()
                .rounded(px(4.0))
                .on_click(move |_, _, cx| {
                    parent.update(cx, |this, cx| {
                        this.toggle_workspace_filter(item_id, cx);
                    });
                });

        let row = h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .group(group_name.clone());

        let row = if drag_enabled {
            let parent_for_drop = self.parent.clone();
            row.drag_over::<DragWorkspace>(move |this, _, _, _| {
                this.border_t_2().border_color(drag_border_color)
            })
            .on_drop(window.listener_for(
                &parent_for_drop,
                move |this, drag: &DragWorkspace, _window, cx| {
                    this.reorder_workspace_by_id(drag.source_id, item_id_for_drop, cx);
                },
            ))
        } else {
            row
        };

        Some(
            list_item.child(
                row.child(
                    div()
                        .id(SharedString::from(format!("ws-sort-handle-{}", item.id)))
                        .w(px(18.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_grab()
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .when(drag_enabled, |this| {
                            this.on_drag(
                                DragWorkspace {
                                    source_id: item.id,
                                    name: item.name.clone(),
                                    count: item.count,
                                },
                                |drag, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.new(|_| drag.clone())
                                },
                            )
                        })
                        .child(
                            Icon::new(IconName::Menu)
                                .with_size(Size::Small)
                                .text_color(cx.theme().muted_foreground),
                        ),
                )
                .child(
                    Checkbox::new(SharedString::from(format!("ws-check-{}", item.id)))
                        .checked(item.checked),
                )
                .child({
                    let tooltip_text =
                        SharedString::from(format!("{} ({})", item.name, item.count));
                    h_flex()
                        .id(SharedString::from(format!("ws-name-{}", item.id)))
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(item.name.clone())
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("({})", item.count)),
                        )
                        .tooltip(move |window, cx| {
                            Tooltip::new(tooltip_text.clone()).build(window, cx)
                        })
                })
                .child(
                    h_flex()
                        .gap_0p5()
                        .invisible()
                        .group_hover(group_name, |this| this.visible())
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            Button::new(SharedString::from(format!("ws-edit-{}", item.id)))
                                .icon(IconName::Edit)
                                .primary()
                                .xsmall()
                                .on_click(move |_, window, cx| {
                                    show_workspace_dialog(
                                        parent_for_edit.clone(),
                                        Some(item_id_for_edit),
                                        item_name_for_edit.clone(),
                                        item_sort_order_for_edit,
                                        window,
                                        cx,
                                    );
                                }),
                        )
                        .child(
                            Button::new(SharedString::from(format!("ws-delete-{}", item.id)))
                                .icon(IconName::Remove)
                                .danger()
                                .xsmall()
                                .on_click(window.listener_for(
                                    &parent_for_delete,
                                    move |this, _, window, cx| {
                                        this.delete_workspace(item_id_for_delete, window, cx);
                                    },
                                )),
                        ),
                ),
            ),
        )
    }

    fn set_selected_index(
        &mut self,
        _ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
    }

    fn confirm(
        &mut self,
        _secondary: bool,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
    }

    fn cancel(&mut self, _window: &mut Window, cx: &mut Context<ListState<Self>>) {
        self.parent.update(cx, |this, cx| {
            this.workspace_filter_open = false;
            cx.notify();
        });
    }
}
