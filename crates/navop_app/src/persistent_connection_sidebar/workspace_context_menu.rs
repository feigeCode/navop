use gpui::{Entity, Window};
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use one_assets::IconName;
use one_core::storage::Workspace;
use rust_i18n::t;

use super::PersistentConnectionSidebar;
use super::connection_context_menu::{
    MoveTargetLabels, MoveTargetNode, move_target_menu_excluding,
};
use crate::home::home_workspace_filter::{WorkspaceDialogConfig, show_workspace_dialog};

impl PersistentConnectionSidebar {
    pub(super) fn build_workspace_context_menu(
        menu: PopupMenu,
        view: &Entity<Self>,
        workspace_id: i64,
        expanded: bool,
        window: &mut Window,
        cx: &mut gpui::Context<PopupMenu>,
    ) -> PopupMenu {
        let home = view.read(cx).home_page.clone();
        let Some(workspace) = home
            .read(cx)
            .workspaces
            .iter()
            .find(|workspace| workspace.id == Some(workspace_id))
            .cloned()
        else {
            return menu;
        };
        let sort_order = home.read(cx).workspaces.len() as i32;
        let toggle_view = view.clone();
        let child_home = home.clone();
        let new_home = home.clone();
        let rename_home = home.clone();
        let delete_home = home.clone();
        // 重命名条目会消费掉 `workspace`，移动子菜单要用克隆的那份。
        let move_workspace = workspace.clone();
        let move_home = home.clone();

        let menu = menu
            .item(new_child_item(child_home, workspace_id, sort_order))
            .item(new_connection_item(new_home, workspace_id))
            .separator()
            .item(
                PopupMenuItem::new(expansion_label(expanded))
                    .icon(expansion_icon(expanded))
                    .on_click(window.listener_for(&toggle_view, move |this, _, _, cx| {
                        this.toggle_workspace_collapsed(workspace_id, cx);
                    })),
            )
            .item(
                PopupMenuItem::new(t!("Workspace.rename").to_string())
                    .icon(IconName::Edit)
                    .on_click(move |_, window, cx| {
                        show_workspace_dialog(
                            rename_home.clone(),
                            WorkspaceDialogConfig {
                                workspace_id: Some(workspace_id),
                                parent_id: workspace.parent_id,
                                initial_name: workspace.name.clone(),
                                initial_sort_order: workspace.sort_order,
                            },
                            window,
                            cx,
                        );
                    }),
            );
        let menu = append_move_to_parent_submenu(menu, &move_home, &move_workspace, window, cx);
        menu.item(
            PopupMenuItem::new(t!("Workspace.delete").to_string())
                .icon(IconName::Remove)
                .on_click(move |_, window, cx| {
                    delete_home.update(cx, |home, cx| {
                        home.delete_workspace(workspace_id, window, cx);
                    });
                }),
        )
        .separator()
        .item(refresh_item(home))
    }
}

/// 分组右键菜单里的「移动到」：把分组（连同它的子树）挂到另一个分组下面，
/// 或者移回顶层。
///
/// 没有这个入口时，子分组一旦建出来就再也回不到根级——编辑对话框把
/// `parent_id: None` 当作「不修改父级」（`parent_id.or(ws.parent_id)`），
/// 所以「移回顶层」必须有独立的一次写入。
fn append_move_to_parent_submenu(
    menu: PopupMenu,
    home: &Entity<crate::home_tab::HomePage>,
    workspace: &Workspace,
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    let workspace_id = workspace.id.unwrap_or_default();
    let workspaces = home.read(cx).workspaces.clone();
    // 目标里不能出现自己和自己的子孙，否则一定成环。
    let excluded = home.read(cx).workspace_subtree_ids(workspace_id);
    let root_label = t!("Workspace.move_target_root").to_string();
    let root_group_label = t!("Connection.move_target_root_group").to_string();
    let targets = move_target_menu_excluding(
        &workspaces,
        MoveTargetLabels {
            unassigned: &root_label,
            root_group: &root_group_label,
        },
        &excluded,
    );
    let current_parent = workspace.parent_id;
    let home_for_submenu = home.clone();
    menu.submenu_with_icon(
        Some(IconName::Folder.into()),
        t!("Workspace.move_to_parent").to_string(),
        window,
        cx,
        move |submenu, window, cx| {
            let submenu = if targets.needs_hint() {
                submenu.label(
                    t!("Workspace.move_target_hint", count = targets.group_count).to_string(),
                )
            } else {
                submenu
            };
            append_workspace_move_targets(
                submenu,
                targets.entries.clone(),
                &home_for_submenu,
                workspace_id,
                current_parent,
                window,
                cx,
            )
        },
    )
}

/// 按层级把纯逻辑条目渲染进菜单；含子分组的条目展开成子菜单。
fn append_workspace_move_targets(
    mut menu: PopupMenu,
    nodes: Vec<MoveTargetNode>,
    home: &Entity<crate::home_tab::HomePage>,
    workspace_id: i64,
    current_parent: Option<i64>,
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    for node in nodes {
        let MoveTargetNode {
            label,
            workspace_id: target_parent,
            children,
        } = node;
        if children.is_empty() {
            menu = workspace_move_target_item(
                menu,
                home,
                workspace_id,
                target_parent,
                current_parent,
                label,
                window,
            );
            continue;
        }
        let home_for_submenu = home.clone();
        menu = menu.submenu_with_icon(
            Some(IconName::Folder.into()),
            label,
            window,
            cx,
            move |submenu, window, cx| {
                // 父分组本身也要能作为目标。
                let submenu = workspace_move_target_item(
                    submenu,
                    &home_for_submenu,
                    workspace_id,
                    target_parent,
                    current_parent,
                    t!("Workspace.move_to_this_group").to_string(),
                    window,
                );
                let submenu = submenu.separator();
                append_workspace_move_targets(
                    submenu,
                    children.clone(),
                    &home_for_submenu,
                    workspace_id,
                    current_parent,
                    window,
                    cx,
                )
            },
        );
    }
    menu
}

fn workspace_move_target_item(
    menu: PopupMenu,
    home: &Entity<crate::home_tab::HomePage>,
    workspace_id: i64,
    parent_id: Option<i64>,
    current_parent: Option<i64>,
    label: String,
    window: &mut Window,
) -> PopupMenu {
    let home = home.clone();
    menu.item(
        PopupMenuItem::new(label)
            .checked(parent_id == current_parent)
            .disabled(parent_id == current_parent)
            .on_click(window.listener_for(&home, move |home, _, _, cx| {
                home.move_workspace_to_parent(workspace_id, parent_id, cx);
            })),
    )
}

fn new_child_item(
    home: Entity<crate::home_tab::HomePage>,
    workspace_id: i64,
    sort_order: i32,
) -> PopupMenuItem {
    PopupMenuItem::new(t!("Workspace.new_child").to_string())
        .icon(IconName::Plus)
        .on_click(move |_, window, cx| {
            show_workspace_dialog(
                home.clone(),
                WorkspaceDialogConfig {
                    parent_id: Some(workspace_id),
                    initial_sort_order: Some(sort_order),
                    ..Default::default()
                },
                window,
                cx,
            );
        })
}

/// 分组右键菜单里的「新建连接」：把该分组作为目标分组传进向导，
/// 保存后连接直接落在该分组（用户仍可在表单里改）。
fn new_connection_item(
    home: Entity<crate::home_tab::HomePage>,
    workspace_id: i64,
) -> PopupMenuItem {
    PopupMenuItem::new(t!("Home.new_connection").to_string())
        .icon(IconName::Plus)
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                home.show_new_connection_dialog(Some(workspace_id), window, cx);
            });
        })
}

fn refresh_item(home: Entity<crate::home_tab::HomePage>) -> PopupMenuItem {
    PopupMenuItem::new(t!("Home.refresh").to_string())
        .icon(IconName::Refresh)
        .on_click(move |_, _, cx| {
            home.update(cx, |home, cx| home.refresh_local_home_data(cx));
        })
}

fn expansion_label(expanded: bool) -> String {
    if expanded {
        t!("Workspace.collapse")
    } else {
        t!("Workspace.expand")
    }
    .to_string()
}

fn expansion_icon(expanded: bool) -> IconName {
    if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    }
}

#[cfg(test)]
mod new_connection_target_workspace_tests {
    /// 分组右键「新建连接」必须把该分组透传进向导。
    ///
    /// 用户反馈的「新建连接之后就不在之前那个文件夹里了」，起点就在这里：入口不带分组，
    /// 表单只能预选「未分组」。
    #[test]
    fn group_context_menu_forwards_its_workspace_to_the_wizard() {
        let source = include_str!("workspace_context_menu.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source");
        assert!(
            production.contains("show_new_connection_dialog(Some(workspace_id), window, cx)"),
            "分组右键新建连接必须带上目标分组"
        );
    }

    /// 工具栏 / 空列表 / 快捷键等入口没有分组上下文，只能显式传 None。
    #[test]
    fn other_entries_pass_no_target_workspace() {
        for (name, source) in [
            (
                "home_tab/toolbar.rs",
                include_str!("../home_tab/toolbar.rs"),
            ),
            (
                "home_tab/content.rs",
                include_str!("../home_tab/content.rs"),
            ),
            ("navop_app.rs", include_str!("../navop_app.rs")),
        ] {
            assert!(
                source.contains("show_new_connection_dialog(None"),
                "{name} 里的新建连接入口不应凭空指定分组"
            );
        }
    }
}
