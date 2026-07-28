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
    use one_core::cloud_sync::{TeamKeyCacheStatus, TeamOption};
    use one_core::storage::{StoredConnection, TeamMembershipState};

    use super::*;
    use crate::home_tab::{TeamPermissionSnapshot, can_manage_connection_with_permissions};
    use crate::persistent_connection_sidebar::tree_model::{
        ConnectionNodeInput, WorkspaceNodeInput, filter_connection_tree_inputs,
    };

    fn searched_team_ssh() -> StoredConnection {
        StoredConnection {
            id: Some(50),
            name: "186华为云服务器".to_string(),
            connection_type: ConnectionType::SshSftp,
            params: "{}".to_string(),
            workspace_id: Some(1),
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: Some("cloud-1".to_string()),
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: Some("team-1".to_string()),
            owner_id: Some("user-1".to_string()),
        }
    }

    fn owner_team() -> TeamOption {
        TeamOption {
            id: "team-1".to_string(),
            name: "CoMi团队".to_string(),
            key_status: TeamKeyCacheStatus::Missing,
            key_version: 0,
            key_verification: None,
            last_verified_at: None,
            role: Some("owner".to_string()),
            membership_state: TeamMembershipState::Active,
        }
    }

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
    fn searched_team_ssh_uses_stable_id_and_cached_permission_snapshot() {
        let connections = vec![searched_team_ssh()];
        let mut permissions =
            TeamPermissionSnapshot::from_persisted_user_id(Some("user-1".to_string()));
        assert!(permissions.replace_teams_for("user-1", vec![owner_team()]));
        let expected = vec![
            ConnectionMenuAction::OpenInBackground,
            ConnectionMenuAction::OpenSftp,
            ConnectionMenuAction::CopyConnection,
            ConnectionMenuAction::MoveToGroup,
            ConnectionMenuAction::Edit,
            ConnectionMenuAction::Duplicate,
            ConnectionMenuAction::Delete,
        ];

        for query in ["", "华为"] {
            let mut workspaces = vec![WorkspaceNodeInput {
                id: 1,
                parent_id: None,
                name: "生产环境".to_string(),
            }];
            let mut tree_connections = vec![ConnectionNodeInput {
                id: 50,
                workspace_id: Some(1),
                name: "186华为云服务器".to_string(),
            }];
            filter_connection_tree_inputs(
                &mut workspaces,
                &mut tree_connections,
                query,
                |connection| connection.name.contains(query),
            );
            let visible_ids = tree_connections
                .iter()
                .map(|connection| connection.id)
                .collect::<Vec<_>>();
            assert_eq!(vec![50], visible_ids);

            let can_edit =
                can_manage_connection_with_permissions(&connections, visible_ids[0], &permissions);
            assert_eq!(
                expected,
                connection_menu_actions(ConnectionType::SshSftp, can_edit)
            );
        }
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
