use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, IntoElement, ListSizingBehavior, ParentElement, Styled, div, uniform_list};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, IconSize, Sizable, StyledExt, h_flex,
    input::{Input, LocalInputStyle},
    v_flex,
};
use rust_i18n::t;

use super::batch_toolbar::batch_mode_toggle;
use super::header_actions::header_actions_menu;
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
                })
            })
            .collect::<Vec<_>>();
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
            .gap_2()
            .items_center()
            .px_2()
            .bg(palette.background)
            .border_r_1()
            .border_b_1()
            .border_color(palette.border)
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
            // Leave a full control-sized gap after the macOS traffic lights;
            // the narrower padding made the title look attached to the green
            // window button even though the bounds did not overlap.
            .when(cfg!(target_os = "macos"), |this| {
                this.pl(layout.macos_compact_title_bar_content_padding)
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
                    .child(batch_mode_toggle(
                        view_for_batch,
                        self.connection_selection.is_active(),
                        palette,
                    ))
                    .child(header_actions_menu(view_for_actions, palette, cx)),
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
        assert!(source.contains("layout.macos_compact_title_bar_content_padding"));
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
}
