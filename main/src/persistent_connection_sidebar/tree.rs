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
    /// 主页 Tree 布局嵌入的树视图：满宽、无 resize 手柄，交互与常驻侧栏一致。
    pub(crate) fn render_home_tree(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        self.render_tree_impl(None, false, cx)
    }

    /// 停靠渲染（docked=true 时树从窗口顶部开始，macOS 头部需避让红绿灯）。
    pub(crate) fn render_docked_tree(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        self.render_tree_impl(Some(self.tree_width), true, cx)
    }

    pub(super) fn render_connection_tree(
        &mut self,
        _palette: SidebarPalette,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        self.render_tree_impl(Some(self.tree_width), false, cx)
    }

    /// width 为 Some 时按固定宽度停靠渲染（含 resize 手柄），None 时满宽嵌入主页。
    fn render_tree_impl(
        &mut self,
        width: Option<gpui::Pixels>,
        macos_titlebar_inset: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let palette = self.palette(cx);
        let rows = self.tree_rows(cx);
        v_flex()
            .relative()
            .when_some(width, |tree, width| tree.w(width).min_w(width).max_w(width))
            .w_full()
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .bg(palette.background)
            .text_color(palette.foreground)
            // 嵌入主页时隐藏树头部：页面标题行已提供计数与分组菜单，避免重复。
            .when(!self.home_embedded, |tree| {
                tree.child(self.render_tree_header(palette, macos_titlebar_inset, cx))
            })
            // 嵌入主页时不渲染树内搜索框：主页工具栏的搜索与类型筛选直接驱动树，
            // 避免上下两个搜索框重复。
            .when(!self.home_embedded, |tree| {
                tree.child(self.render_tree_search(palette, cx))
            })
            .when(self.home_page.read(cx).batch_mode_active(), |tree| {
                tree.child(self.render_batch_toolbar(&rows, palette, cx))
            })
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .when(rows.is_empty(), |body| {
                        // 过滤空态：树内就地改筛选，不提供清除按钮（DESIGN §10）。
                        body.child(
                            div()
                                .w_full()
                                .px_3()
                                .py_7()
                                .flex()
                                .justify_center()
                                .text_xs()
                                .text_color(palette.muted_foreground)
                                .child(t!("Home.no_filter_results")),
                        )
                    })
                    .when(!rows.is_empty(), |body| {
                        body.child(
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
                        )
                    }),
            )
            .when(width.is_some(), |tree| {
                tree.child(self.render_tree_resize_handle(cx))
            })
            .into_any_element()
    }

    pub(super) fn tree_rows(&self, cx: &gpui::App) -> Vec<ConnectionTreeRow> {
        let home = self.home_page.read(cx);
        // 嵌入主页时使用主页工具栏的搜索词与类型筛选；停靠/浮动时用树自身的。
        let (query, type_filter) = if self.home_embedded {
            (
                home.search_query.read(cx).trim().to_lowercase(),
                home.selected_filter,
            )
        } else {
            (
                self.search_input.read(cx).value().trim().to_lowercase(),
                self.selected_filter,
            )
        };
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
            .filter(|connection| {
                crate::home_tab::connection_filter::match_connection_type(
                    type_filter,
                    connection,
                )
            })
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

    fn render_tree_header(
        &self,
        palette: SidebarPalette,
        macos_titlebar_inset: bool,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let connection_count = {
            let home = self.home_page.read(cx);
            home.connections
                .iter()
                .filter(|connection| {
                    crate::home_tab::connection_filter::match_connection_type(
                        self.selected_filter,
                        connection,
                    )
                })
                .count()
        };
        let view_for_batch = cx.entity();
        let home_for_batch = self.home_page.clone();
        let batch_active = self.home_page.read(cx).batch_mode_active();
        let view_for_actions = cx.entity();
        let layout = cx.theme().geometry.layout;
        // 停靠树的 header 从窗口左上角开始；macOS 红绿灯覆盖该区域，需左侧避让。
        let titlebar_inset = cfg!(target_os = "macos") && macos_titlebar_inset;
        h_flex()
            .w_full()
            .h(layout.embedded_panel_header)
            .flex_shrink_0()
            .pr_2()
            .when(titlebar_inset, |header| {
                header.pl(layout.macos_title_bar_content_padding)
            })
            .when(!titlebar_inset, |header| header.pl_2())
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
                    .child(batch_mode_toggle(home_for_batch, batch_active, palette))
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
