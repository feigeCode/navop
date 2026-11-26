use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, InteractiveElement, IntoElement, ListSizingBehavior, MouseButton, ParentElement,
    Styled, div, px, uniform_list,
};
use gpui_component::{ActiveTheme, Icon, Sizable, StyledExt, h_flex, input::Input, v_flex};
use one_assets::IconName;
use one_core::settings::{AppSettings, ConnectionSortOrder};
use one_ui::IconSize;
use rust_i18n::t;

use crate::connection_sort::{connection_name_cmp, lru_sort_key, manual_order_name_cmp};

use super::batch_toolbar::{auto_hide_tree_toggle, batch_mode_toggle};
use super::tree_model::{
    ConnectionNodeInput, ConnectionTreeRow, TreeSectionOptions, WorkspaceNodeInput,
    build_connection_tree_rows, filter_connection_tree_inputs, hide_empty_workspace_inputs,
};
use super::{PersistentConnectionSidebar, SidebarPalette};

impl PersistentConnectionSidebar {
    /// 主页 Tree 布局嵌入的树视图：满宽、无 resize 手柄，交互与常驻侧栏一致。
    pub(crate) fn render_home_tree(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        self.home_embedded = true;
        self.home_navigation_layout = false;
        self.render_tree_impl(None, cx)
    }

    pub(crate) fn render_home_navigation_tree(
        &mut self,
        width: gpui::Pixels,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        self.home_embedded = true;
        self.home_navigation_layout = true;
        self.render_tree_impl(Some(width), cx)
    }

    pub(super) fn render_connection_tree(
        &mut self,
        _palette: SidebarPalette,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        self.home_embedded = false;
        self.render_tree_impl(Some(self.tree_width), cx)
    }

    /// width 为 Some 时按固定宽度停靠渲染（含 resize 手柄），None 时满宽嵌入主页。
    pub(super) fn render_tree_impl(
        &mut self,
        width: Option<gpui::Pixels>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let palette = self.palette(cx);
        let rows = self.tree_rows(cx);
        v_flex()
            .relative()
            .debug_selector(|| "persistent-connection-resource".to_owned())
            .when_some(width, |tree, width| tree.w(width).min_w(width).max_w(width))
            .w_full()
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .bg(palette.background)
            .text_color(palette.foreground)
            // 外壳托管时左侧就是应用菜单，两者同属一个壳，再画分隔线会在壳中间留一道
            // 接缝；只有嵌进内容画布的树才用左边线与画布分区。
            .when(self.home_embedded && !self.home_navigation_layout, |tree| {
                tree.border_l_1().border_color(palette.border)
            })
            // 嵌入主页时隐藏树头部：页面标题行已提供计数与分组菜单，避免重复。
            .when(!self.home_embedded || self.home_navigation_layout, |tree| {
                tree.child(self.render_tree_header(palette, cx))
            })
            // 嵌入主页时不渲染树内搜索框：主页工具栏的搜索与类型筛选直接驱动树，
            // 避免上下两个搜索框重复。
            .when(!self.home_embedded || self.home_navigation_layout, |tree| {
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
                    .px_2()
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
            .when(width.is_some() && !self.home_navigation_layout, |tree| {
                // 应用外壳统一渲染右侧拖动条，避免内外两套拖动事件抢占。
                tree.child(self.render_tree_resize_handle(cx))
            })
            .into_any_element()
    }

    pub(super) fn tree_rows(&self, cx: &gpui::App) -> Vec<ConnectionTreeRow> {
        let home = self.home_page.read(cx);
        // 嵌入主页时使用主页工具栏的搜索词与类型筛选；停靠/浮动时用树自身的。
        let (query, type_filter) = if self.home_embedded && !self.home_navigation_layout {
            (
                home.search_query.read(cx).trim().to_lowercase(),
                home.selected_filter.clone(),
            )
        } else {
            (
                self.search_input.read(cx).value().trim().to_lowercase(),
                self.selected_filter.clone(),
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
            .enumerate()
            .filter_map(|(position, workspace)| {
                Some(WorkspaceNodeInput {
                    id: workspace.id?,
                    parent_id: workspace.parent_id,
                    name: if cfg!(feature = "screenshot-safe") {
                        crate::screenshot_safe::workspace_name(position)
                    } else {
                        workspace.name.clone()
                    },
                    // 同级顺序由树模型按 (sort_order, name) 显式排序，与主页分组排序一致。
                    sort_order: workspace.sort_order,
                })
            })
            .collect::<Vec<_>>();
        let mut matching_connection_ids = std::collections::HashSet::new();
        let mut connections = home
            .connections
            .iter()
            .filter(|connection| {
                crate::home_tab::connection_filter::match_connection_type(&type_filter, connection)
            })
            .filter_map(|connection| {
                let id = connection.id?;
                if home.match_connection(connection, &query) {
                    matching_connection_ids.insert(id);
                }
                Some(ConnectionNodeInput {
                    id,
                    workspace_id: connection.workspace_id,
                    name: if cfg!(feature = "screenshot-safe") {
                        crate::screenshot_safe::connection_name(
                            connection.connection_type,
                            Some(id),
                        )
                    } else {
                        connection.name.clone()
                    },
                    sort_order: connection.sort_order,
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
            ConnectionSortOrder::Manual => {
                connections.sort_by(|left, right| {
                    manual_order_name_cmp(
                        left.sort_order,
                        &left.name,
                        right.sort_order,
                        &right.name,
                    )
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
        let mut rows = build_connection_tree_rows(
            &workspaces,
            &connections,
            if searching {
                &expanded_workspaces
            } else {
                &collapsed_workspaces
            },
            TreeSectionOptions {
                // 搜索只展示命中项，不重复渲染分区标题。
                show_groups_header: !searching,
                show_unassigned_header: !searching,
                // 与卡片/列表布局的“未分组”分组共用折叠状态。
                unassigned_expanded: !home.unassigned_connections_collapsed(),
            },
        );
        if self.home_embedded && !self.home_navigation_layout && query.is_empty() {
            let recent =
                crate::home_tab::recent_connections(&home.connections, &type_filter, "", 4);
            if !recent.is_empty() {
                let expanded = !home.recent_connections_collapsed();
                let mut recent_rows = vec![ConnectionTreeRow::RecentHeader {
                    count: recent.len(),
                    expanded,
                }];
                if expanded {
                    recent_rows.extend(recent.into_iter().filter_map(|connection| {
                        Some(ConnectionTreeRow::RecentConnection {
                            id: connection.id?,
                            name: if cfg!(feature = "screenshot-safe") {
                                crate::screenshot_safe::connection_name(
                                    connection.connection_type,
                                    connection.id,
                                )
                            } else {
                                connection.name
                            },
                        })
                    }));
                }
                recent_rows.append(&mut rows);
                return recent_rows;
            }
        }
        rows
    }

    fn render_tree_search(&self, palette: SidebarPalette, cx: &gpui::Context<Self>) -> AnyElement {
        let has_query = !self.search_input.read(cx).value().is_empty();
        let search_height = if self.home_navigation_layout {
            // 导航主页右侧使用完整 Home toolbar；左侧搜索区与其底边对齐。
            px(44.0)
        } else {
            px(40.0)
        };
        h_flex()
            .w_full()
            .h(search_height)
            .flex_shrink_0()
            .gap_1()
            .items_center()
            .px_2()
            .bg(palette.background)
            .child(
                h_flex()
                    .w_full()
                    .h(px(30.0))
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(palette.border)
                    .bg(cx.theme().background)
                    .child(
                        Icon::new(IconName::Search)
                            .with_size(IconSize::Micro)
                            .text_color(palette.muted_foreground),
                    )
                    .child(
                        div().min_w_0().flex_1().child(
                            Input::new(&self.search_input)
                                .appearance(false)
                                .cleanable(has_query)
                                .text_color(palette.foreground),
                        ),
                    )
                    .child(self.render_tree_filter_button(palette, cx)),
            )
            .into_any_element()
    }

    fn render_tree_header(&self, palette: SidebarPalette, cx: &gpui::Context<Self>) -> AnyElement {
        let connection_count = {
            let home = self.home_page.read(cx);
            home.connections
                .iter()
                .filter(|connection| {
                    crate::home_tab::connection_filter::match_connection_type(
                        &self.selected_filter,
                        connection,
                    )
                })
                .count()
        };
        let view_for_batch = cx.entity();
        let home_for_batch = self.home_page.clone();
        let batch_active = self.home_page.read(cx).batch_mode_active();
        let view_for_actions = cx.entity();
        // macOS 红绿灯避让由最左应用导航负责，连接树不再叠加避让。
        h_flex()
            .id("persistent-connection-header")
            .w_full()
            .h(one_ui::theme_geometry().layout.panel_header)
            .flex_shrink_0()
            .pr_2()
            .pl_2()
            .items_center()
            .justify_between()
            // 连接列表位于正常 Tab 栏下方，header 属于资源面板，不再延续系统标题栏。
            .bg(palette.background)
            .text_color(palette.foreground)
            .child(
                h_flex()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_base()
                            .font_semibold()
                            .child(t!("Connection.connection_list")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted_foreground)
                            .child(connection_count.to_string()),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
    fn connection_header_does_not_duplicate_the_outer_navigation_traffic_light_inset() {
        let source = include_str!("tree.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap();
        assert!(!implementation.contains("layout.macos_title_bar_content_padding"));
        assert!(!implementation.contains("titlebar_inset"));
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
    fn connection_header_below_tabs_is_not_a_window_drag_region() {
        let source = include_str!("tree.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap();
        assert!(!implementation.contains("window_control_area(WindowControlArea::Drag)"));
        assert!(!implementation.contains("window.start_window_move()"));
        assert!(
            implementation
                .contains("on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())")
        );
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

    #[test]
    fn embedded_home_tree_prepends_recent_connections_only_without_search() {
        let source = include_str!("tree.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap();

        // 仅普通嵌入树预置最近区；全局导航布局的最近区由内容区自己渲染。
        assert!(
            implementation
                .contains("self.home_embedded && !self.home_navigation_layout && query.is_empty()")
        );
        assert!(implementation.contains("home_tab::recent_connections"));
        assert!(implementation.contains("ConnectionTreeRow::RecentHeader"));
        assert!(implementation.contains("ConnectionTreeRow::RecentConnection"));
    }

    #[test]
    fn tree_rows_feed_sort_order_and_the_unassigned_section() {
        let source = include_str!("tree.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap();

        // 同级顺序由树模型按 (sort_order, name) 显式排序。
        assert!(implementation.contains("sort_order: workspace.sort_order"));
        // 未分组分区：搜索时不重复渲染标题，折叠状态与卡片/列表布局共用。
        assert!(implementation.contains("TreeSectionOptions {"));
        assert!(implementation.contains("show_groups_header: !searching"));
        assert!(implementation.contains("show_unassigned_header: !searching"));
        assert!(
            implementation
                .contains("unassigned_expanded: !home.unassigned_connections_collapsed()")
        );
        assert!(implementation.contains("ConnectionSortOrder::Manual"));
    }

    #[test]
    fn each_tree_render_entry_restores_its_own_embedding_mode() {
        let source = include_str!("tree.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap();

        let home = implementation
            .split("fn render_home_tree")
            .nth(1)
            .expect("home tree entry exists")
            .split("fn render_home_navigation_tree")
            .next()
            .unwrap();
        let navigation = implementation
            .split("fn render_home_navigation_tree")
            .nth(1)
            .expect("home navigation tree entry exists")
            .split("fn render_connection_tree")
            .next()
            .unwrap();
        let docked = implementation
            .split("fn render_connection_tree")
            .nth(1)
            .expect("docked tree entry exists")
            .split("fn render_tree_impl")
            .next()
            .unwrap();

        assert!(home.contains("self.home_navigation_layout = false"));
        assert!(navigation.contains("self.home_navigation_layout = true"));
        assert!(docked.contains("self.home_embedded = false"));
    }
}
