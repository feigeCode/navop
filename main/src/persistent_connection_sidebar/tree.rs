use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement,
    v_flex,
};
use rust_i18n::t;

use super::PersistentConnectionSidebar;
use super::tree_model::{
    ConnectionNodeInput, ConnectionTreeRow, WorkspaceNodeInput, build_connection_tree_rows,
};
use crate::home::home_workspace_filter::{WorkspaceDialogConfig, show_workspace_dialog};

const CONNECTION_TREE_WIDTH: gpui::Pixels = px(260.0);

impl PersistentConnectionSidebar {
    pub(super) fn render_connection_tree(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let rows = self.tree_rows(cx);
        v_flex()
            .w(CONNECTION_TREE_WIDTH)
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(self.render_tree_header(cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .size_full()
                            .py_1()
                            .overflow_y_scrollbar()
                            .children(rows.into_iter().map(|row| self.render_tree_row(row, cx))),
                    ),
            )
            .into_any_element()
    }

    fn tree_rows(&self, cx: &gpui::App) -> Vec<ConnectionTreeRow> {
        let home = self.home_page.read(cx);
        let workspaces = home
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
        let connections = home
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
        build_connection_tree_rows(
            &workspaces,
            &connections,
            &self.collapsed_workspaces,
            self.unassigned_collapsed,
        )
    }

    fn render_tree_header(&self, cx: &gpui::Context<Self>) -> AnyElement {
        let home_for_new = self.home_page.clone();
        let home_for_refresh = self.home_page.clone();
        h_flex()
            .w_full()
            .h(px(40.0))
            .px_2()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(t!("Connection.connection_list")),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("persistent-new-root-group")
                            .icon(IconName::FolderOpen)
                            .ghost()
                            .xsmall()
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
                            .tooltip(t!("Home.refresh"))
                            .on_click(move |_, _, cx| {
                                home_for_refresh
                                    .update(cx, |home, cx| home.refresh_local_home_data(cx));
                            }),
                    ),
            )
            .into_any_element()
    }
}
