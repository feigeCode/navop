use one_core::storage::ConnectionType;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConnectionMenuAction {
    OpenInBackground,
    OpenFullscreenWindow,
    OpenSftp,
    CopyConnection,
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
    if connection_type != ConnectionType::All {
        actions.push(ConnectionMenuAction::CopyConnection);
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
                ConnectionMenuAction::CopyConnection,
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
        assert!(actions.contains(&ConnectionMenuAction::CopyConnection));
        assert!(!actions.contains(&ConnectionMenuAction::OpenSftp));
    }

    #[test]
    fn read_only_connection_menu_keeps_copy_submenu_without_management_actions() {
        assert_eq!(
            connection_menu_actions(ConnectionType::Rdp, false),
            vec![
                ConnectionMenuAction::OpenInBackground,
                ConnectionMenuAction::OpenFullscreenWindow,
                ConnectionMenuAction::CopyConnection,
            ]
        );
    }

    #[test]
    fn every_real_connection_type_uses_one_top_level_copy_submenu() {
        let actions = connection_menu_actions(ConnectionType::Rdp, true);
        assert_eq!(
            1,
            actions
                .iter()
                .filter(|action| **action == ConnectionMenuAction::CopyConnection)
                .count()
        );
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
