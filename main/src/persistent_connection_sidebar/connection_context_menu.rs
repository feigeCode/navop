use std::collections::{HashMap, HashSet};

use gpui::{Entity, Window};
use gpui_component::{
    WindowExt as _,
    menu::{PopupMenu, PopupMenuItem},
    notification::Notification,
};
use one_assets::IconName;
use one_core::{
    storage::{
        ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection, Workspace,
    },
    tab_container::TabOpenMode,
};
use rust_i18n::t;
use tool_runtime::AiAccessLevel;

use super::{
    connection_copy_menu::append_copy_connection_submenu,
    context_menu::{ConnectionMenuAction, connection_menu_actions},
};
use crate::home_tab::HomePage;

/// 连接右键菜单，常驻侧栏树与主页卡片/列表/树共用，保证入口一致。
pub(crate) fn build_connection_context_menu(
    mut menu: PopupMenu,
    home: &Entity<HomePage>,
    connection_id: i64,
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    let home = home.clone();
    let Some(connection) = home
        .read(cx)
        .connections
        .iter()
        .find(|connection| connection.id == Some(connection_id))
        .cloned()
    else {
        return menu;
    };
    let can_edit = home.read(cx).can_move_connection(connection_id);
    let can_export_credentials = home
        .read(cx)
        .can_export_connection_credentials(connection_id);
    let resolved_ssh = {
        let home = home.read(cx);
        connection
            .to_port_forwarding_params()
            .ok()
            .and_then(|params| {
                home.connections
                    .iter()
                    .find(|candidate| {
                        candidate.id == Some(params.ssh_connection_id)
                            && candidate.connection_type == ConnectionType::SshSftp
                    })
                    .cloned()
            })
    };

    let action_context = ConnectionActionContext {
        connection: &connection,
        can_export_credentials,
        resolved_ssh: resolved_ssh.as_ref(),
        home: &home,
    };
    let actions = connection_menu_actions(
        connection.connection_type,
        can_edit,
        connection.preferred_open_mode,
    );
    // 「AI 访问」是一个设置项，固定放在「打开」类动作之后、「管理」类动作之前。
    let action_count = actions.len();
    let ai_access_section = actions
        .iter()
        .position(|action| starts_menu_section(*action))
        .unwrap_or(action_count);
    for (index, action) in actions.into_iter().enumerate() {
        if index == ai_access_section {
            menu = append_ai_access_submenu(menu, connection_id, &home, window, cx);
        }
        menu = add_connection_action(menu, action, &action_context, window, cx);
    }
    if ai_access_section == action_count {
        menu = append_ai_access_submenu(menu, connection_id, &home, window, cx);
    }
    menu
}

fn starts_menu_section(action: ConnectionMenuAction) -> bool {
    matches!(
        action,
        ConnectionMenuAction::CopyConnection | ConnectionMenuAction::MoveToGroup
    )
}

struct ConnectionActionContext<'a> {
    connection: &'a StoredConnection,
    can_export_credentials: bool,
    resolved_ssh: Option<&'a StoredConnection>,
    home: &'a Entity<crate::home_tab::HomePage>,
}

fn add_connection_action(
    menu: PopupMenu,
    action: ConnectionMenuAction,
    action_context: &ConnectionActionContext<'_>,
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    let menu = if starts_menu_section(action) {
        menu.separator()
    } else {
        menu
    };
    match action {
        ConnectionMenuAction::MoveToGroup => append_move_to_group_submenu(
            menu,
            action_context.connection,
            action_context.home,
            window,
            cx,
        ),
        ConnectionMenuAction::CopyConnection => append_copy_connection_submenu(
            menu,
            action_context.connection,
            action_context.can_export_credentials,
            action_context.resolved_ssh,
            action_context.home,
            window,
            cx,
        ),
        _ => menu.item(connection_menu_item(
            action,
            action_context.connection,
            action_context.home,
        )),
    }
}

/// 「AI 访问」子菜单：快捷切换三档策略（issue #437）。
fn append_ai_access_submenu(
    menu: PopupMenu,
    connection_id: i64,
    home: &Entity<crate::home_tab::HomePage>,
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    let current = super::load_ai_access_level(connection_id, cx);
    let home = home.clone();
    menu.submenu_with_icon(
        Some(IconName::Ai.into()),
        t!("Connection.ai_access").to_string(),
        window,
        cx,
        move |mut submenu, _window, _cx| {
            for level in connection_form::ai_access::all_levels() {
                let home = home.clone();
                submenu = submenu.item(
                    PopupMenuItem::new(ai_access_menu_label(level))
                        .checked(level == current)
                        .on_click(move |_, window, cx| {
                            apply_ai_access_level(&home, connection_id, level, window, cx);
                        }),
                );
            }
            submenu
        },
    )
}

/// 菜单项文案：与侧栏徽标同在 `Connection.ai_access_*` 命名空间下。
fn ai_access_menu_label(level: AiAccessLevel) -> String {
    match level {
        AiAccessLevel::Allowed => t!("Connection.ai_access_allowed").to_string(),
        AiAccessLevel::ReadOnly => t!("Connection.ai_access_read_only").to_string(),
        AiAccessLevel::Denied => t!("Connection.ai_access_denied").to_string(),
    }
}

/// 写策略并给出可见反馈：失败必须弹通知，不能让「限制没生效」被静默吞掉。
fn apply_ai_access_level(
    home: &Entity<crate::home_tab::HomePage>,
    connection_id: i64,
    level: AiAccessLevel,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    match write_ai_access_level(connection_id, level, cx) {
        // 侧栏/列表/卡片都在观察 HomePage，notify 会顺带让它们重读策略。
        Ok(()) => home.update(cx, |_, cx| cx.notify()),
        Err(error) => {
            let message = t!("Connection.ai_access_save_failed", error = error).to_string();
            window.push_notification(Notification::error(message).autohide(true), cx);
        }
    }
}

fn write_ai_access_level(
    connection_id: i64,
    level: AiAccessLevel,
    cx: &gpui::App,
) -> Result<(), String> {
    let storage = cx
        .try_global::<GlobalStorageState>()
        .map(|state| state.storage.clone())
        .ok_or_else(|| t!("Connection.ai_access_storage_missing").to_string())?;
    let repo = storage
        .get::<ConnectionRepository>()
        .ok_or_else(|| t!("Connection.ai_access_storage_missing").to_string())?;
    connection_form::ai_access::write_level(&repo, connection_id, level)
}

fn connection_menu_item(
    action: ConnectionMenuAction,
    connection: &StoredConnection,
    home: &Entity<crate::home_tab::HomePage>,
) -> PopupMenuItem {
    match action {
        ConnectionMenuAction::OpenInBackground => background_connection_item(connection, home),
        ConnectionMenuAction::OpenFullscreenWindow => {
            #[cfg(feature = "shell")]
            {
                fullscreen_window_connection_item(connection, home)
            }
            #[cfg(not(feature = "shell"))]
            {
                unsupported_menu_item("Connection.open_in_fullscreen_window")
            }
        }
        ConnectionMenuAction::OpenSftp => {
            #[cfg(feature = "shell")]
            {
                open_sftp_item(connection, home)
            }
            #[cfg(not(feature = "shell"))]
            {
                unsupported_menu_item("Home.open_sftp")
            }
        }
        ConnectionMenuAction::OpenTerminal => {
            #[cfg(feature = "shell")]
            {
                open_terminal_item(connection, home)
            }
            #[cfg(not(feature = "shell"))]
            {
                unsupported_menu_item("Home.open_terminal")
            }
        }
        ConnectionMenuAction::CopyConnection => {
            unreachable!("copy connection action renders a submenu")
        }
        ConnectionMenuAction::MoveToGroup => unreachable!("move action renders a submenu"),
        ConnectionMenuAction::Edit => edit_connection_item(connection, home),
        ConnectionMenuAction::Duplicate => duplicate_connection_item(connection, home),
        ConnectionMenuAction::Delete => delete_connection_item(connection, home),
    }
}

#[cfg(feature = "shell")]
fn fullscreen_window_connection_item(
    connection: &StoredConnection,
    home: &Entity<crate::home_tab::HomePage>,
) -> PopupMenuItem {
    let connection = connection.clone();
    let home = home.clone();
    PopupMenuItem::new(t!("Connection.open_in_fullscreen_window").to_string())
        .icon(IconName::Maximize)
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                home.open_remote_desktop_fullscreen_window(&connection, window, cx);
            });
        })
}

/// 「移动到分组」菜单最多展开的层级数；`parent_id` 脏数据成环时用于兜底。
pub(super) const MOVE_TARGET_MAX_DEPTH: usize = 32;

/// 分组数量超过该值时，在子菜单顶部提示层级关系，避免长列表难以扫读。
pub(super) const MOVE_TARGET_HINT_THRESHOLD: usize = 20;

/// 构造菜单条目时需要本地化的固定文案，注入后纯逻辑不依赖 i18n 运行时。
#[derive(Clone, Copy)]
pub(super) struct MoveTargetLabels<'a> {
    /// 第一项（`workspace_id` 为 `None`）的文案：连接是「未分组」，分组是「顶层」。
    pub(super) unassigned: &'a str,
    pub(super) root_group: &'a str,
}

/// 一个移动目标；`workspace_id` 为 `None` 表示「不放进任何分组」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MoveTargetNode {
    pub(super) label: String,
    pub(super) workspace_id: Option<i64>,
    pub(super) children: Vec<MoveTargetNode>,
}

/// 纯逻辑产出的菜单数据：`entries` 的第一项固定是「未分组 / 顶层」。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct MoveTargetMenu {
    pub(super) entries: Vec<MoveTargetNode>,
    pub(super) group_count: usize,
}

impl MoveTargetMenu {
    pub(super) fn needs_hint(&self) -> bool {
        self.group_count > MOVE_TARGET_HINT_THRESHOLD
    }
}

fn append_move_to_group_submenu(
    menu: PopupMenu,
    connection: &StoredConnection,
    home: &Entity<crate::home_tab::HomePage>,
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    let connection_id = connection.id.unwrap_or_default();
    let workspaces = home.read(cx).workspaces.clone();
    let current_workspace_id = connection.workspace_id;
    let unassigned = t!("Home.unassigned_workspace").to_string();
    let root_group = t!("Connection.move_target_root_group").to_string();
    let targets = move_target_menu(
        &workspaces,
        MoveTargetLabels {
            unassigned: &unassigned,
            root_group: &root_group,
        },
    );
    let home_for_submenu = home.clone();
    menu.submenu_with_icon(
        Some(IconName::Folder.into()),
        t!("Connection.move_to_group").to_string(),
        window,
        cx,
        move |submenu, window, cx| {
            let submenu = if targets.needs_hint() {
                submenu.label(
                    t!("Connection.move_target_hint", count = targets.group_count).to_string(),
                )
            } else {
                submenu
            };
            append_move_target_nodes(
                submenu,
                targets.entries.clone(),
                &home_for_submenu,
                connection_id,
                current_workspace_id,
                window,
                cx,
            )
        },
    )
}

/// 按层级把纯逻辑条目渲染进菜单：叶子分组是普通可点项，含子分组的项展开成子菜单。
fn append_move_target_nodes(
    mut menu: PopupMenu,
    nodes: Vec<MoveTargetNode>,
    home: &Entity<crate::home_tab::HomePage>,
    connection_id: i64,
    current_workspace_id: Option<i64>,
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    for node in nodes {
        let MoveTargetNode {
            label,
            workspace_id,
            children,
        } = node;
        let Some(workspace_id) = workspace_id else {
            // 「未分组」始终排在第一个可点条目。
            menu = move_target_item(
                menu,
                home,
                connection_id,
                None,
                current_workspace_id,
                label,
                window,
            );
            continue;
        };
        if children.is_empty() {
            menu = move_target_item(
                menu,
                home,
                connection_id,
                Some(workspace_id),
                current_workspace_id,
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
                // 父分组本身也要能作为目标，所以子菜单第一项是「移动到此分组」。
                let submenu = move_target_item(
                    submenu,
                    &home_for_submenu,
                    connection_id,
                    Some(workspace_id),
                    current_workspace_id,
                    t!("Connection.move_to_this_group").to_string(),
                    window,
                );
                let submenu = submenu.separator();
                append_move_target_nodes(
                    submenu,
                    children.clone(),
                    &home_for_submenu,
                    connection_id,
                    current_workspace_id,
                    window,
                    cx,
                )
            },
        );
    }
    menu
}

/// 纯逻辑：把扁平分组列表按 `parent_id` 组成层级条目，供菜单层直接消费。
fn move_target_menu(workspaces: &[Workspace], labels: MoveTargetLabels<'_>) -> MoveTargetMenu {
    move_target_menu_excluding(workspaces, labels, &HashSet::new())
}

/// 同上，但把 `excluded` 里的分组（以及它们的子树）整体剔掉。
///
/// 分组自身的「移动到」菜单要排除自己那颗子树，否则菜单里会出现「移到自己下面」
/// 这种必然成环的目标。
pub(super) fn move_target_menu_excluding(
    workspaces: &[Workspace],
    labels: MoveTargetLabels<'_>,
    excluded: &HashSet<i64>,
) -> MoveTargetMenu {
    let mut by_id: HashMap<i64, &Workspace> = HashMap::new();
    let mut name_counts: HashMap<&str, usize> = HashMap::new();
    let mut children: HashMap<Option<i64>, Vec<&Workspace>> = HashMap::new();

    for workspace in workspaces {
        let Some(id) = workspace.id else {
            continue;
        };
        if excluded.contains(&id) {
            continue;
        }
        by_id.insert(id, workspace);
        *name_counts.entry(workspace.name.as_str()).or_default() += 1;
    }
    for workspace in workspaces {
        let Some(id) = workspace.id else {
            continue;
        };
        if excluded.contains(&id) {
            continue;
        }
        // 父级指向自己或已不存在的分组时按根分组处理，脏数据也能出现在菜单里。
        let parent_id = workspace
            .parent_id
            .filter(|parent_id| *parent_id != id && by_id.contains_key(parent_id));
        children.entry(parent_id).or_default().push(workspace);
    }

    // 同级顺序只在这里排一次，根级与各层子级共用同一份排好序的兄弟列表。
    for siblings in children.values_mut() {
        sort_move_targets(siblings);
    }

    let mut visited: HashSet<i64> = HashSet::new();
    let mut entries = vec![MoveTargetNode {
        label: labels.unassigned.to_string(),
        workspace_id: None,
        children: Vec::new(),
    }];
    if let Some(roots) = children.remove(&None) {
        entries.extend(build_move_target_nodes(
            &roots,
            &children,
            &by_id,
            &name_counts,
            labels,
            0,
            &mut visited,
        ));
    }

    // 只靠 parent_id 环互连的分组从根级不可达，兜底挂到根级，避免分组从菜单里消失。
    let mut unreachable = by_id
        .values()
        .copied()
        .filter(|workspace| workspace.id.is_some_and(|id| !visited.contains(&id)))
        .collect::<Vec<_>>();
    sort_move_targets(&mut unreachable);
    entries.extend(build_move_target_nodes(
        &unreachable,
        &children,
        &by_id,
        &name_counts,
        labels,
        0,
        &mut visited,
    ));

    let group_count = count_move_targets(&entries[1..]);
    MoveTargetMenu {
        entries,
        group_count,
    }
}

fn build_move_target_nodes(
    workspaces: &[&Workspace],
    children: &HashMap<Option<i64>, Vec<&Workspace>>,
    by_id: &HashMap<i64, &Workspace>,
    name_counts: &HashMap<&str, usize>,
    labels: MoveTargetLabels<'_>,
    depth: usize,
    visited: &mut HashSet<i64>,
) -> Vec<MoveTargetNode> {
    if depth > MOVE_TARGET_MAX_DEPTH {
        return Vec::new();
    }
    let mut nodes = Vec::new();
    for workspace in workspaces {
        let Some(id) = workspace.id else {
            continue;
        };
        if !visited.insert(id) {
            continue;
        }
        let child_workspaces: &[&Workspace] =
            children.get(&Some(id)).map(Vec::as_slice).unwrap_or(&[]);
        let child_nodes = if depth == MOVE_TARGET_MAX_DEPTH {
            // 超过深度上限的分组不再渲染子层级，但要标记为已访问，避免被当成环兜底成根级。
            mark_move_targets_visited(child_workspaces, children, visited);
            Vec::new()
        } else {
            build_move_target_nodes(
                child_workspaces,
                children,
                by_id,
                name_counts,
                labels,
                depth + 1,
                visited,
            )
        };
        nodes.push(MoveTargetNode {
            label: move_target_label(workspace, by_id, name_counts, labels),
            workspace_id: Some(id),
            children: child_nodes,
        });
    }
    nodes
}

fn mark_move_targets_visited(
    workspaces: &[&Workspace],
    children: &HashMap<Option<i64>, Vec<&Workspace>>,
    visited: &mut HashSet<i64>,
) {
    for workspace in workspaces {
        let Some(id) = workspace.id else {
            continue;
        };
        if !visited.insert(id) {
            continue;
        }
        if let Some(child_workspaces) = children.get(&Some(id)) {
            mark_move_targets_visited(child_workspaces, children, visited);
        }
    }
}

fn count_move_targets(nodes: &[MoveTargetNode]) -> usize {
    nodes
        .iter()
        .map(|node| 1 + count_move_targets(&node.children))
        .sum()
}

/// 同级顺序与树保持一致：先 `sort_order`，再按名称自然序（数字段按数值比较）。
fn sort_move_targets(workspaces: &mut [&Workspace]) {
    workspaces.sort_by(|left, right| {
        left.sort_order
            .cmp(&right.sort_order)
            .then_with(|| crate::connection_sort::connection_name_cmp(&left.name, &right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
}

/// 全局重名的分组在 label 后补父级路径，例如 `主库（01-Prod / DB）`；名称唯一时保持原名。
fn move_target_label(
    workspace: &Workspace,
    by_id: &HashMap<i64, &Workspace>,
    name_counts: &HashMap<&str, usize>,
    labels: MoveTargetLabels<'_>,
) -> String {
    if name_counts
        .get(workspace.name.as_str())
        .copied()
        .unwrap_or(0)
        <= 1
    {
        return workspace.name.clone();
    }
    let path = workspace_ancestor_path(workspace, by_id);
    if path.is_empty() {
        format!("{}（{}）", workspace.name, labels.root_group)
    } else {
        format!("{}（{}）", workspace.name, path)
    }
}

/// 从根到直接父级的名称路径；遇到环或超过深度上限立即停止。
fn workspace_ancestor_path(workspace: &Workspace, by_id: &HashMap<i64, &Workspace>) -> String {
    let mut names: Vec<&str> = Vec::new();
    let mut visited: HashSet<i64> = HashSet::new();
    if let Some(id) = workspace.id {
        visited.insert(id);
    }
    let mut parent_id = workspace.parent_id;
    let mut depth = 0;
    while let Some(id) = parent_id {
        if depth >= MOVE_TARGET_MAX_DEPTH || !visited.insert(id) {
            break;
        }
        let Some(parent) = by_id.get(&id).copied() else {
            break;
        };
        names.push(parent.name.as_str());
        parent_id = parent.parent_id;
        depth += 1;
    }
    names.reverse();
    names.join(" / ")
}

fn move_target_item(
    menu: PopupMenu,
    home: &Entity<crate::home_tab::HomePage>,
    connection_id: i64,
    workspace_id: Option<i64>,
    current_workspace_id: Option<i64>,
    label: String,
    window: &mut Window,
) -> PopupMenu {
    let home = home.clone();
    menu.item(
        PopupMenuItem::new(label)
            .checked(workspace_id == current_workspace_id)
            .disabled(workspace_id == current_workspace_id)
            .on_click(window.listener_for(&home, move |home, _, _, cx| {
                home.move_connection_to_workspace(connection_id, workspace_id, cx);
            })),
    )
}

#[cfg(not(feature = "shell"))]
fn unsupported_menu_item(label_key: &'static str) -> PopupMenuItem {
    PopupMenuItem::new(t!(label_key).to_string()).disabled(true)
}

fn background_connection_item(
    connection: &StoredConnection,
    home: &Entity<crate::home_tab::HomePage>,
) -> PopupMenuItem {
    let connection = connection.clone();
    let home = home.clone();
    PopupMenuItem::new(t!("Connection.open_in_background").to_string())
        .icon(IconName::ExternalLink)
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                home.open_connection_from_quick_with_mode(
                    &connection,
                    TabOpenMode::Background,
                    window,
                    cx,
                );
            });
        })
}

#[cfg(feature = "shell")]
fn open_sftp_item(
    connection: &StoredConnection,
    home: &Entity<crate::home_tab::HomePage>,
) -> PopupMenuItem {
    let connection = connection.clone();
    let home = home.clone();
    PopupMenuItem::new(t!("Home.open_sftp").to_string())
        .icon(IconName::FolderOpen)
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                home.open_sftp_view(connection.clone(), window, cx);
            });
        })
}

/// SSH 条目偏好为双栏文件视图时的右键切换入口：临时按终端打开。
#[cfg(feature = "shell")]
fn open_terminal_item(
    connection: &StoredConnection,
    home: &Entity<crate::home_tab::HomePage>,
) -> PopupMenuItem {
    let connection = connection.clone();
    let home = home.clone();
    PopupMenuItem::new(t!("Home.open_terminal").to_string())
        .icon(IconName::Terminal)
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                if !app_kit::registry().open_connection(
                    &connection,
                    TabOpenMode::Activate,
                    home,
                    window,
                    cx,
                ) {
                    window.push_notification(
                        gpui_component::notification::Notification::error(format!(
                            "当前产品不支持打开该连接：{}",
                            connection.name
                        )),
                        cx,
                    );
                }
            });
        })
}

fn edit_connection_item(
    connection: &StoredConnection,
    home: &Entity<crate::home_tab::HomePage>,
) -> PopupMenuItem {
    let connection = connection.clone();
    let home = home.clone();
    PopupMenuItem::new(t!("Home.edit_connection").to_string())
        .icon(IconName::Edit)
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                home.edit_connection(connection.clone(), window, cx);
            });
        })
}

fn duplicate_connection_item(
    connection: &StoredConnection,
    home: &Entity<crate::home_tab::HomePage>,
) -> PopupMenuItem {
    let connection = connection.clone();
    let home = home.clone();
    PopupMenuItem::new(t!("Home.duplicate_connection").to_string())
        .icon(IconName::Copy)
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                home.duplicate_connection(connection.clone(), window, cx);
            });
        })
}

fn delete_connection_item(
    connection: &StoredConnection,
    home: &Entity<crate::home_tab::HomePage>,
) -> PopupMenuItem {
    let connection_id = connection.id.unwrap_or_default();
    let connection_name = connection.name.clone();
    let home = home.clone();
    PopupMenuItem::new(t!("Home.delete_connection").to_string())
        .icon(IconName::Remove)
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| {
                home.confirm_delete_connection(connection_id, connection_name.clone(), window, cx);
            });
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(
        id: i64,
        name: &str,
        parent_id: Option<i64>,
        sort_order: Option<i32>,
    ) -> Workspace {
        let mut workspace = Workspace::new(name.to_string());
        workspace.id = Some(id);
        workspace.parent_id = parent_id;
        workspace.sort_order = sort_order;
        workspace
    }

    fn labels() -> MoveTargetLabels<'static> {
        MoveTargetLabels {
            unassigned: "未分组",
            root_group: "根分组",
        }
    }

    /// 展开成 (label, workspace_id, depth)，便于断言层级与顺序。
    fn flatten(nodes: &[MoveTargetNode]) -> Vec<(String, Option<i64>, usize)> {
        fn walk(
            nodes: &[MoveTargetNode],
            depth: usize,
            out: &mut Vec<(String, Option<i64>, usize)>,
        ) {
            for node in nodes {
                out.push((node.label.clone(), node.workspace_id, depth));
                walk(&node.children, depth + 1, out);
            }
        }
        let mut out = Vec::new();
        walk(nodes, 0, &mut out);
        out
    }

    fn labels_of(entries: &[MoveTargetNode]) -> Vec<(String, Option<i64>)> {
        flatten(entries)
            .into_iter()
            .map(|(label, workspace_id, _)| (label, workspace_id))
            .collect()
    }

    #[test]
    fn move_targets_render_real_hierarchy_in_sibling_order() {
        let workspaces = vec![
            workspace(1, "生产", None, Some(1)),
            workspace(2, "数据库", Some(1), Some(2)),
            workspace(3, "缓存", Some(1), Some(1)),
            workspace(4, "测试", None, Some(0)),
            workspace(5, "MySQL", Some(2), None),
            workspace(6, "Redis", Some(3), None),
            workspace(7, "分组10", None, Some(2)),
            workspace(8, "分组2", None, Some(2)),
        ];

        let menu = move_target_menu(&workspaces, labels());

        assert_eq!(
            vec![
                ("未分组".to_string(), None, 0),
                ("测试".to_string(), Some(4), 0),
                ("生产".to_string(), Some(1), 0),
                ("缓存".to_string(), Some(3), 1),
                ("Redis".to_string(), Some(6), 2),
                ("数据库".to_string(), Some(2), 1),
                ("MySQL".to_string(), Some(5), 2),
                // 同级同 sort_order 时按名称自然序，分组2 排在 分组10 前。
                ("分组2".to_string(), Some(8), 0),
                ("分组10".to_string(), Some(7), 0),
            ],
            flatten(&menu.entries)
        );
        assert_eq!(8, menu.group_count);
        assert!(!menu.needs_hint());

        // 有子分组的分组节点必须保留 children，菜单层才会展开成子菜单。
        assert_eq!(2, menu.entries[2].children.len());
    }

    #[test]
    fn duplicate_group_names_append_ancestor_path() {
        let workspaces = vec![
            workspace(1, "01-Prod", None, Some(0)),
            workspace(2, "DB", Some(1), Some(0)),
            workspace(3, "主库", Some(2), Some(0)),
            workspace(4, "主库", None, Some(1)),
            workspace(5, "main", Some(1), Some(1)),
            workspace(6, "main", None, Some(2)),
            workspace(7, "唯一分组", None, Some(3)),
        ];

        let menu = move_target_menu(&workspaces, labels());
        let labels = labels_of(&menu.entries);

        assert!(labels.contains(&("主库（01-Prod / DB）".to_string(), Some(3))));
        assert!(labels.contains(&("主库（根分组）".to_string(), Some(4))));
        assert!(labels.contains(&("main（01-Prod）".to_string(), Some(5))));
        assert!(labels.contains(&("main（根分组）".to_string(), Some(6))));
        // 名称唯一的分组不加路径，保持原名。
        assert!(labels.contains(&("唯一分组".to_string(), Some(7))));
        assert!(labels.contains(&("01-Prod".to_string(), Some(1))));
        assert!(labels.contains(&("DB".to_string(), Some(2))));
    }

    #[test]
    fn cyclic_parent_links_terminate_and_keep_every_group_once() {
        let workspaces = vec![
            workspace(1, "环", Some(2), Some(0)),
            workspace(2, "环", Some(1), Some(1)),
            workspace(3, "缓存", Some(2), Some(2)),
            // 自己是自己的父级
            workspace(4, "自环", Some(4), Some(3)),
            // 父级 ID 已不存在
            workspace(5, "孤儿", Some(999), Some(4)),
        ];

        let menu = move_target_menu(&workspaces, labels());
        let flat = flatten(&menu.entries);

        // 未分组 + 5 个分组，每个分组恰好出现一次且没有无限递归。
        assert_eq!(6, flat.len());
        assert_eq!(5, menu.group_count);
        for id in [1, 2, 3, 4, 5] {
            assert_eq!(
                1,
                flat.iter()
                    .filter(|(_, workspace_id, _)| *workspace_id == Some(id))
                    .count(),
                "分组 {id} 应该恰好出现一次"
            );
        }
        // 环内分组的父路径也要能安全终止，label 不能为空。
        assert!(flat.iter().all(|(label, _, _)| !label.is_empty()));

        // 自环与父级失效的分组按根分组处理，仍然可点。
        assert_eq!(Some(4), flat[1].1);
        assert_eq!(Some(5), flat[2].1);
    }

    #[test]
    fn unassigned_entry_stays_first_and_large_menus_request_hint() {
        for group_count in [1usize, 20] {
            let workspaces = (1..=group_count as i64)
                .map(|id| workspace(id, &format!("分组{id}"), None, Some(id as i32)))
                .collect::<Vec<_>>();
            let menu = move_target_menu(&workspaces, labels());
            assert_eq!(
                Some(("未分组".to_string(), None)),
                menu.entries
                    .first()
                    .map(|node| (node.label.clone(), node.workspace_id))
            );
            assert_eq!(group_count, menu.group_count);
            assert!(!menu.needs_hint());
        }

        let workspaces = (1..=21)
            .map(|id| workspace(id, &format!("分组{id}"), None, Some(id as i32)))
            .collect::<Vec<_>>();
        let menu = move_target_menu(&workspaces, labels());
        assert_eq!(21, menu.group_count);
        assert!(menu.needs_hint());
    }

    #[test]
    fn groups_without_ids_are_not_offered_as_targets() {
        let mut drafts = vec![
            workspace(1, "已保存", None, Some(0)),
            Workspace::new("未保存草稿".to_string()),
        ];
        drafts[1].parent_id = Some(1);

        let menu = move_target_menu(&drafts, labels());

        assert_eq!(1, menu.group_count);
        assert_eq!(
            vec![
                ("未分组".to_string(), None),
                ("已保存".to_string(), Some(1))
            ],
            labels_of(&menu.entries)
        );
    }

    #[test]
    fn move_menu_builds_parent_aware_submenus() {
        let production = include_str!("connection_context_menu.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source exists");

        assert!(production.contains("submenu_with_icon("));
        assert!(
            !production.contains("PopupMenuItem::submenu("),
            "manually inserting a submenu leaves its parent menu unset"
        );
    }
}
