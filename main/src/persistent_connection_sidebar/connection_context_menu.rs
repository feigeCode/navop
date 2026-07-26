use gpui::{Entity, Window};
use gpui_component::{
    IconName,
    menu::{PopupMenu, PopupMenuItem},
};
use one_core::{
    storage::{ConnectionType, StoredConnection},
    tab_container::TabOpenMode,
};
use rust_i18n::t;

use super::{
    PersistentConnectionSidebar,
    connection_copy_menu::append_copy_connection_submenu,
    context_menu::{ConnectionMenuAction, connection_menu_actions},
};

impl PersistentConnectionSidebar {
    pub(super) fn build_connection_context_menu(
        mut menu: PopupMenu,
        view: &Entity<Self>,
        connection_id: i64,
        window: &mut Window,
        cx: &mut gpui::Context<PopupMenu>,
    ) -> PopupMenu {
        let home = view.read(cx).home_page.clone();
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
        for action in connection_menu_actions(connection.connection_type, can_edit) {
            menu = add_connection_action(menu, action, &action_context, window, cx);
        }
        menu
    }
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

fn connection_menu_item(
    action: ConnectionMenuAction,
    connection: &StoredConnection,
    home: &Entity<crate::home_tab::HomePage>,
) -> PopupMenuItem {
    match action {
        ConnectionMenuAction::OpenInBackground => background_connection_item(connection, home),
        ConnectionMenuAction::OpenFullscreenWindow => {
            fullscreen_window_connection_item(connection, home)
        }
        ConnectionMenuAction::OpenSftp => open_sftp_item(connection, home),
        ConnectionMenuAction::CopyConnection => {
            unreachable!("copy connection action renders a submenu")
        }
        ConnectionMenuAction::MoveToGroup => unreachable!("move action renders a submenu"),
        ConnectionMenuAction::Edit => edit_connection_item(connection, home),
        ConnectionMenuAction::Duplicate => duplicate_connection_item(connection, home),
        ConnectionMenuAction::Delete => delete_connection_item(connection, home),
    }
}

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
    let home_for_submenu = home.clone();
    let submenu = PopupMenu::build(window, cx, move |submenu, window, _| {
        let submenu = move_target_item(
            submenu,
            &home_for_submenu,
            connection_id,
            None,
            current_workspace_id,
            t!("Home.unassigned_workspace").to_string(),
            window,
        );
        workspaces.into_iter().fold(submenu, |submenu, workspace| {
            let Some(workspace_id) = workspace.id else {
                return submenu;
            };
            move_target_item(
                submenu,
                &home_for_submenu,
                connection_id,
                Some(workspace_id),
                current_workspace_id,
                workspace.name,
                window,
            )
        })
    });
    menu.item(
        PopupMenuItem::submenu(t!("Connection.move_to_group").to_string(), submenu)
            .icon(IconName::Folder),
    )
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
