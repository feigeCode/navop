use one_core::storage::ConnectionType;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConnectionMenuAction {
    OpenInBackground,
    OpenFullscreenWindow,
    OpenSftp,
    CopyInfo,
    CopyName,
    CopyTargets,
    CopyCommand,
    MoveToGroup,
    Edit,
    Duplicate,
    Delete,
}

pub(super) fn connection_menu_actions(
    connection_type: ConnectionType,
    can_edit: bool,
) -> Vec<ConnectionMenuAction> {
    let mut actions = vec![ConnectionMenuAction::OpenInBackground];
    if matches!(connection_type, ConnectionType::Rdp | ConnectionType::Vnc) {
        actions.push(ConnectionMenuAction::OpenFullscreenWindow);
    }
    if connection_type == ConnectionType::SshSftp {
        actions.push(ConnectionMenuAction::OpenSftp);
    }
    actions.push(ConnectionMenuAction::CopyInfo);
    actions.push(ConnectionMenuAction::CopyName);
    if connection_type != ConnectionType::All {
        actions.push(ConnectionMenuAction::CopyTargets);
    }
    if matches!(
        connection_type,
        ConnectionType::Database
            | ConnectionType::SshSftp
            | ConnectionType::Redis
            | ConnectionType::MongoDB
    ) {
        actions.push(ConnectionMenuAction::CopyCommand);
    }
    if can_edit {
        actions.extend([
            ConnectionMenuAction::MoveToGroup,
            ConnectionMenuAction::Edit,
            ConnectionMenuAction::Duplicate,
            ConnectionMenuAction::Delete,
        ]);
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_menu_has_terminal_sftp_and_management_actions() {
        assert_eq!(
            connection_menu_actions(ConnectionType::SshSftp, true),
            vec![
                ConnectionMenuAction::OpenInBackground,
                ConnectionMenuAction::OpenSftp,
                ConnectionMenuAction::CopyInfo,
                ConnectionMenuAction::CopyName,
                ConnectionMenuAction::CopyTargets,
                ConnectionMenuAction::CopyCommand,
                ConnectionMenuAction::MoveToGroup,
                ConnectionMenuAction::Edit,
                ConnectionMenuAction::Duplicate,
                ConnectionMenuAction::Delete,
            ]
        );
    }

    #[test]
    fn non_ssh_menu_does_not_offer_sftp() {
        let actions = connection_menu_actions(ConnectionType::Database, true);
        assert!(actions.contains(&ConnectionMenuAction::OpenInBackground));
        assert!(actions.contains(&ConnectionMenuAction::CopyInfo));
        assert!(actions.contains(&ConnectionMenuAction::CopyTargets));
        assert!(actions.contains(&ConnectionMenuAction::CopyCommand));
        assert!(!actions.contains(&ConnectionMenuAction::OpenSftp));
    }

    #[test]
    fn read_only_connection_menu_keeps_open_actions_only() {
        assert_eq!(
            connection_menu_actions(ConnectionType::Rdp, false),
            vec![
                ConnectionMenuAction::OpenInBackground,
                ConnectionMenuAction::OpenFullscreenWindow,
                ConnectionMenuAction::CopyInfo,
                ConnectionMenuAction::CopyName,
                ConnectionMenuAction::CopyTargets,
            ]
        );
    }

    #[test]
    fn unsupported_types_do_not_offer_a_cli_command() {
        let actions = connection_menu_actions(ConnectionType::Rdp, true);
        assert!(!actions.contains(&ConnectionMenuAction::CopyCommand));
        assert!(actions.contains(&ConnectionMenuAction::OpenFullscreenWindow));
        assert!(actions.contains(&ConnectionMenuAction::MoveToGroup));
    }

    #[test]
    fn only_remote_desktop_connections_offer_fullscreen_window() {
        assert!(
            connection_menu_actions(ConnectionType::Rdp, true)
                .contains(&ConnectionMenuAction::OpenFullscreenWindow)
        );
        assert!(
            connection_menu_actions(ConnectionType::Vnc, true)
                .contains(&ConnectionMenuAction::OpenFullscreenWindow)
        );
        assert!(
            !connection_menu_actions(ConnectionType::SshSftp, true)
                .contains(&ConnectionMenuAction::OpenFullscreenWindow)
        );
    }

    #[test]
    fn connection_tree_wires_context_menus_for_every_row_kind() {
        let rows = include_str!("rows.rs");
        let module = include_str!("mod.rs");

        assert_eq!(3, rows.matches(".context_menu(").count());
        assert!(module.contains("mod connection_context_menu;"));
        assert!(module.contains("mod workspace_context_menu;"));
    }
}
