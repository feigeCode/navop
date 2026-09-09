use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, ColorExt as _, IntoElement, ListSizingBehavior, ParentElement, Styled, div,
    uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, IconSize, Sizable, StyledExt, h_flex, input::Input, v_flex,
};
use one_core::settings::{AppSettings, ConnectionSortOrder};
use rust_i18n::t;

use crate::connection_sort::{connection_name_cmp, lru_sort_key};

use super::batch_toolbar::{auto_hide_tree_toggle, batch_mode_toggle};
use super::tree_model::{
    ConnectionNodeInput, ConnectionTreeRow, WorkspaceNodeInput, build_connection_tree_rows,
    filter_connection_tree_inputs, hide_empty_workspace_inputs,
};
use super::{PersistentConnectionSidebar, SidebarPalette};

impl PersistentConnectionSidebar {
    pub(super) fn render_connection_tree(
        &mut self,
        palette: SidebarPalette,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let rows = self.tree_rows(cx);
        v_flex()
            .relative()
            .w(self.tree_width)
            .min_w(self.tree_width)
            .max_w(self.tree_width)
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .bg(palette.background)
            .text_color(palette.foreground)
            .child(self.render_tree_header(palette, cx))
            .child(self.render_tree_search(palette, cx))
            .when(self.connection_selection.is_active(), |tree| {
                tree.child(self.render_batch_toolbar(&rows, palette, cx))
            })
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        uniform_list("persistent-connection-tree", rows.len(), {
                            cx.processor(move |this, range: Range<usize>, _window, cx| {
                                range
                                    .filter_map(|idx| rows.get(idx).cloned())
                                    .map(|row| this.render_tree_row(row, palette, cx))
                                    .collect()
                            })
                        })
                        .size_full()
                        .py_1()
                        .track_scroll(&self.tree_scroll_handle)
                        .with_sizing_behavior(ListSizingBehavior::Auto),
                    ),
            )
            .child(self.render_tree_resize_handle(cx))
            .into_any_element()
    }

    pub(super) fn tree_rows(&self, cx: &gpui::App) -> Vec<ConnectionTreeRow> {
        let home = self.home_page.read(cx);
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        let collapsed_workspaces = home
            .workspaces
            .iter()
            .filter(|workspace| workspace.sidebar_collapsed)
            .filter_map(|workspace| workspace.id)
            .collect::<std::collections::HashSet<_>>();
        let mut workspaces = home
            .workspaces
            .iter()
            .filter_map(|workspace| {
                Some(WorkspaceNodeInput {
                    id: workspace.id?,
                    parent_id: workspace.parent_id,
                    name: workspace.name.clone(),
                })
            })
            .collect::<Vec<_>>();
        let mut matching_connection_ids = std::collections::HashSet::new();
        let mut connections = home
            .connections
            .iter()
            .filter(|connection| home.match_connection_type(connection))
            .filter_map(|connection| {
                let id = connection.id?;
                if home.match_connection(connection, &query) {
                    matching_connection_ids.insert(id);
                }
                Some(ConnectionNodeInput {
                    id,
                    workspace_id: connection.workspace_id,
                    name: connection.name.clone(),
                    last_used_at: connection.last_used_at,
                    updated_at: connection.updated_at,
                    created_at: connection.created_at,
                })
            })
            .collect::<Vec<_>>();
        // 分组内的连接按设置中的排序方式排列
        match AppSettings::global(cx).connection_sort_order {
            ConnectionSortOrder::Natural => {
                connections.sort_by(|left, right| connection_name_cmp(&left.name, &right.name));
            }
            ConnectionSortOrder::Lru => {
                connections.sort_by(|left, right| {
                    lru_sort_key(
                        right.last_used_at,
                        right.updated_at,
                        right.created_at,
                        Some(right.id),
                    )
                    .cmp(&lru_sort_key(
                        left.last_used_at,
                        left.updated_at,
                        left.created_at,
                        Some(left.id),
                    ))
                });
            }
        }
        filter_connection_tree_inputs(&mut workspaces, &mut connections, &query, |connection| {
            matching_connection_ids.contains(&connection.id)
        });
        if self.hide_empty_workspaces {
            hide_empty_workspace_inputs(&mut workspaces, &connections);
        }
        let searching = !query.is_empty();
        let expanded_workspaces = std::collections::HashSet::new();
        build_connection_tree_rows(
            &workspaces,
            &connections,
            if searching {
                &expanded_workspaces
            } else {
                &collapsed_workspaces
            },
        )
    }

    fn render_tree_search(&self, palette: SidebarPalette, cx: &gpui::Context<Self>) -> AnyElement {
        let has_query = !self.search_input.read(cx).value().is_empty();
        h_flex()
            .w_full()
            .h_10()
            .flex_shrink_0()
            .gap_1()
            .items_center()
            .px_2()
            .bg(palette.background)
            .border_b_1()
            // 右侧分隔统一由 resize 手柄的可见线承担，避免多段边框叠加产生拼接感。
            .border_color(palette.border.opacity(0.6))
            .child(
                Icon::new(IconName::Search)
                    .with_size(IconSize::Micro)
                    .text_color(palette.muted_foreground),
            )
            .child(
                div().min_w_0().flex_1().child(
                    Input::new(&self.search_input)
                        .xsmall()
                        .appearance(false)
                        .cleanable(has_query)
                        .text_color(palette.foreground),
                ),
            )
            .child(self.render_tree_filter_button(palette, cx))
            .into_any_element()
    }

    fn render_tree_header(&self, palette: SidebarPalette, cx: &gpui::Context<Self>) -> AnyElement {
        let connection_count = {
            let home = self.home_page.read(cx);
            home.connections
                .iter()
                .filter(|connection| home.match_connection_type(connection))
                .count()
        };
        let view_for_batch = cx.entity();
        let view_for_actions = cx.entity();
        let layout = cx.theme().geometry.layout;
        h_flex()
            .w_full()
            .h(layout.embedded_panel_header)
            .flex_shrink_0()
            .pr_2()
            // The navigation rail is gone, so on macOS the header starts at
            // the window edge and must clear the full traffic-light strip.
            .when(cfg!(target_os = "macos"), |this| {
                this.pl(layout.macos_title_bar_content_padding)
            })
            .when(!cfg!(target_os = "macos"), |this| this.pl_2())
            .items_center()
            .justify_between()
            // On macOS the header continues the traffic-light strip. On
            // Windows/Linux it belongs to the connection panel and should not
            // create a dark title band across the top.
            .bg(if cfg!(target_os = "macos") {
                palette.rail_background
            } else {
                palette.background
            })
            .text_color(palette.foreground)
            .border_b_1()
            .border_color(palette.border.opacity(0.6))
            .child(
                h_flex()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(t!("Connection.connection_list")),
                    )
                    .child(
                        div()
                            .px_1p5()
                            .rounded_full()
                            .bg(palette.muted)
                            .text_xs()
                            .text_color(palette.muted_foreground)
                            .child(connection_count.to_string()),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(auto_hide_tree_toggle(
                        view_for_batch.clone(),
                        self.auto_hide_tree,
                        palette,
                    ))
                    .child(batch_mode_toggle(
                        view_for_batch,
                        self.connection_selection.is_active(),
                        palette,
                    ))
                    .child(self.header_actions_menu(view_for_actions, palette)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn macos_connection_header_clears_the_traffic_lights() {
        let source = include_str!("tree.rs");
        assert!(source.contains("cfg!(target_os = \"macos\")"));
        assert!(source.contains("layout.macos_title_bar_content_padding"));
    }

    #[test]
    fn connection_header_routes_secondary_actions_through_overflow_menu() {
        let source = include_str!("tree.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap();
        assert!(implementation.contains("header_actions_menu("));
        assert!(!implementation.contains("persistent-collapse-all-groups"));
        assert!(!implementation.contains("persistent-hide-empty-workspaces"));
        assert!(!implementation.contains("persistent-new-root-group"));
        assert!(!implementation.contains("persistent-refresh-connections"));
    }

    #[test]
    fn connection_header_exposes_auto_hide_toggle_left_of_batch_operations() {
        let source = include_str!("tree.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap();
        let toggle = implementation
            .find("auto_hide_tree_toggle(")
            .expect("连接树头部应渲染自动隐藏开关");
        let batch = implementation
            .find("batch_mode_toggle(")
            .expect("连接树头部应渲染批量操作开关");
        assert!(toggle < batch, "自动隐藏开关应位于批量操作开关的左侧");
    }
}
