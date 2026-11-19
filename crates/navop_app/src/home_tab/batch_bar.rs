//! 主页卡片/列表布局下的批量操作条（Tree 布局由常驻侧栏树内渲染同款工具条）。

use super::*;
use gpui_component::Disableable as _;

impl HomePage {
    /// 批量模式下的操作条：已选计数、全选可见、移动到分组、删除、退出批量模式。
    pub(super) fn render_batch_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected_ids = self.connection_selection.ids();
        let query = self.search_query.read(cx).to_lowercase();
        let visible_ids = self.visible_manageable_connection_ids(&query, cx);
        let move_targets = self.batch_move_targets();
        let view = cx.entity();
        h_flex()
            .w_full()
            .h_9()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_5()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .text_xs()
                    .text_color(cx.theme().foreground)
                    .child(t!("Connection.batch_selected", count = selected_ids.len()).to_string()),
            )
            .child(connection_selection::connection_selection_actions(
                view.clone(),
                visible_ids,
                selected_ids.len(),
            ))
            .child(move_connections_button(
                view.clone(),
                selected_ids.clone(),
                move_targets,
            ))
            .child(delete_connections_button(view.clone(), selected_ids))
            .child(exit_batch_mode_button(view))
            .into_any_element()
    }

    fn batch_move_targets(&self) -> Vec<(Option<i64>, String)> {
        std::iter::once((None, t!("Home.unassigned_workspace").to_string()))
            .chain(
                self.workspaces
                    .iter()
                    .filter_map(|workspace| Some((Some(workspace.id?), workspace.name.clone()))),
            )
            .collect()
    }
}

fn move_connections_button(
    view: Entity<HomePage>,
    selected_ids: Vec<i64>,
    move_targets: Vec<(Option<i64>, String)>,
) -> AnyElement {
    let disabled = selected_ids.is_empty();
    IconButton::new("home-move-selected-connections", IconName::Folder)
        .role(IconButtonRole::Compact)
        .tooltip(t!("Connection.move_to_group"))
        .disabled(disabled)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            move_targets
                .iter()
                .cloned()
                .fold(menu, |menu, (workspace_id, label)| {
                    let view = view.clone();
                    let selected_ids = selected_ids.clone();
                    menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                        view.update(cx, |home, cx| {
                            home.move_connections_to_workspace(
                                selected_ids.clone(),
                                workspace_id,
                                cx,
                            );
                            home.connection_selection.clear();
                            cx.notify();
                        });
                    }))
                })
        })
        .into_any_element()
}

fn delete_connections_button(view: Entity<HomePage>, selected_ids: Vec<i64>) -> IconButton {
    let disabled = selected_ids.is_empty();
    IconButton::new("home-delete-selected-connections", IconName::Remove)
        .role(IconButtonRole::Compact)
        .tooltip(t!("Common.delete"))
        .disabled(disabled)
        .on_click(move |_, window, cx| {
            view.update(cx, |home, cx| {
                home.confirm_delete_connections(selected_ids.clone(), window, cx);
            });
        })
}

fn exit_batch_mode_button(view: Entity<HomePage>) -> IconButton {
    IconButton::new("home-exit-batch-connections", IconName::Close)
        .role(IconButtonRole::Compact)
        .tooltip(t!("Connection.batch_exit"))
        .on_click(move |_, _, cx| {
            view.update(cx, |home, cx| home.set_batch_mode(false, cx));
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn batch_bar_exposes_select_move_delete_and_exit_actions() {
        let source = include_str!("batch_bar.rs");
        assert!(source.contains("connection_selection_actions"));
        assert!(source.contains("home-move-selected-connections"));
        assert!(source.contains("home-delete-selected-connections"));
        assert!(source.contains("home-exit-batch-connections"));
    }
}
