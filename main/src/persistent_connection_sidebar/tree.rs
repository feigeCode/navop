use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, IntoElement, ListSizingBehavior, ParentElement, Styled, div, px, uniform_list,
};
use gpui_component::{
    Icon, IconName, Sizable, Size, StyledExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, LocalInputStyle},
    v_flex,
};
use rust_i18n::t;

use super::tree_model::{
    ConnectionNodeInput, ConnectionTreeRow, WorkspaceNodeInput, build_connection_tree_rows,
    filter_connection_tree_inputs,
};
use super::{PersistentConnectionSidebar, SidebarPalette};
use crate::home::home_workspace_filter::{WorkspaceDialogConfig, show_workspace_dialog};

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
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .border_r_1()
                    .border_color(palette.border)
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

    fn tree_rows(&self, cx: &gpui::App) -> Vec<ConnectionTreeRow> {
        let home = self.home_page.read(cx);
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
        let mut connections = home
            .connections
            .iter()
            .filter(|connection| home.match_connection_type(connection))
            .filter_map(|connection| {
                Some(ConnectionNodeInput {
                    id: connection.id?,
                    workspace_id: connection.workspace_id,
                    name: connection.name.clone(),
                })
            })
            .collect::<Vec<_>>();
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        filter_connection_tree_inputs(&mut workspaces, &mut connections, &query);
        let searching = !query.is_empty();
        let expanded_workspaces = std::collections::HashSet::new();
        build_connection_tree_rows(
            &workspaces,
            &connections,
            if searching {
                &expanded_workspaces
            } else {
                &self.collapsed_workspaces
            },
            if searching {
                false
            } else {
                self.unassigned_collapsed
            },
        )
    }

    fn render_tree_search(&self, palette: SidebarPalette, cx: &gpui::Context<Self>) -> AnyElement {
        let has_query = !self.search_input.read(cx).value().is_empty();
        h_flex()
            .w_full()
            .h_10()
            .flex_shrink_0()
            .gap_2()
            .items_center()
            .px_2()
            .bg(palette.background)
            .border_r_1()
            .border_b_1()
            .border_color(palette.border)
            .child(
                Icon::new(IconName::Search)
                    .with_size(Size::XSmall)
                    .text_color(palette.muted_foreground),
            )
            .child(
                div().min_w_0().flex_1().child(
                    Input::new(&self.search_input)
                        .xsmall()
                        .appearance(false)
                        .cleanable(has_query)
                        .local_style(LocalInputStyle {
                            background: palette.background,
                            foreground: palette.foreground,
                            muted_foreground: palette.muted_foreground,
                            border: palette.border,
                        })
                        .text_color(palette.foreground)
                        .caret_color(palette.foreground),
                ),
            )
            .into_any_element()
    }

    fn render_tree_header(&self, palette: SidebarPalette, cx: &gpui::Context<Self>) -> AnyElement {
        let home_for_new = self.home_page.clone();
        let home_for_refresh = self.home_page.clone();
        let connection_count = {
            let home = self.home_page.read(cx);
            home.connections
                .iter()
                .filter(|connection| home.match_connection_type(connection))
                .count()
        };
        let view = cx.entity();
        h_flex()
            .w_full()
            .h(px(40.0))
            .flex_shrink_0()
            .pr_2()
            // On macOS the traffic-light cluster (~64px from the window edge)
            // extends past the narrow navigation rail; pad the header so its
            // content clears the window buttons.
            .when(cfg!(target_os = "macos"), |this| this.pl(px(28.0)))
            .when(!cfg!(target_os = "macos"), |this| this.pl_2())
            .items_center()
            .justify_between()
            // Match the navigation rail so the whole top title-bar strip
            // (traffic lights + header) reads as one uniform color.
            .bg(palette.rail_background)
            .text_color(palette.foreground)
            .border_r_1()
            .border_color(palette.border)
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
                    .child(
                        Button::new("persistent-collapse-all-groups")
                            .icon(IconName::ChevronsUpDown)
                            .ghost()
                            .xsmall()
                            .text_color(palette.foreground)
                            .tooltip(t!("Connection.collapse_all"))
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| this.collapse_all_groups(cx));
                            }),
                    )
                    .child(
                        Button::new("persistent-new-root-group")
                            .icon(IconName::FolderOpen)
                            .ghost()
                            .xsmall()
                            .text_color(palette.foreground)
                            .tooltip(t!("Workspace.new"))
                            .on_click(move |_, window, cx| {
                                let sort_order = home_for_new.read(cx).workspaces.len() as i32;
                                show_workspace_dialog(
                                    home_for_new.clone(),
                                    WorkspaceDialogConfig {
                                        initial_sort_order: Some(sort_order),
                                        ..Default::default()
                                    },
                                    window,
                                    cx,
                                );
                            }),
                    )
                    .child(
                        Button::new("persistent-refresh-connections")
                            .icon(IconName::Refresh)
                            .ghost()
                            .xsmall()
                            .text_color(palette.foreground)
                            .tooltip(t!("Home.refresh"))
                            .on_click(move |_, _, cx| {
                                home_for_refresh
                                    .update(cx, |home, cx| home.refresh_local_home_data(cx));
                            }),
                    ),
            )
            .into_any_element()
    }

    fn collapse_all_groups(&mut self, cx: &mut gpui::Context<Self>) {
        self.collapsed_workspaces = self
            .home_page
            .read(cx)
            .workspaces
            .iter()
            .filter_map(|workspace| workspace.id)
            .collect();
        self.unassigned_collapsed = true;
        cx.notify();
    }
}
