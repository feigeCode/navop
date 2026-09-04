use gpui::{ClipboardItem, Entity, Window};
use gpui_component::{
    IconName,
    menu::{PopupMenu, PopupMenuItem},
};
#[cfg(not(test))]
use gpui_component::{WindowExt, notification::Notification};
use one_core::storage::StoredConnection;
use rust_i18n::t;

use crate::home_tab::HomePage;

use super::connection_copy::{ConnectionCopyAction, connection_copy_actions, connection_copy_text};

pub(super) fn append_copy_connection_submenu(
    menu: PopupMenu,
    connection: &StoredConnection,
    can_export_credentials: bool,
    resolved_ssh: Option<&StoredConnection>,
    home: &Entity<HomePage>,
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    let actions = connection_copy_actions(connection, can_export_credentials, resolved_ssh);
    let connection = connection.clone();
    let resolved_ssh = resolved_ssh.cloned();
    let home = home.clone();
    menu.submenu_with_icon(
        Some(IconName::Copy.into()),
        t!("Connection.copy_connection").to_string(),
        window,
        cx,
        move |mut submenu, _, _| {
            for action in actions.iter().copied() {
                if action == ConnectionCopyAction::Name {
                    submenu = submenu.separator();
                }
                let item = if action == ConnectionCopyAction::FullInfo {
                    copy_full_info_item(connection.id, &home)
                } else {
                    copy_action_item(action, &connection, resolved_ssh.as_ref())
                };
                submenu = submenu.item(item);
            }
            submenu
        },
    )
}

fn copy_action_item(
    action: ConnectionCopyAction,
    connection: &StoredConnection,
    resolved_ssh: Option<&StoredConnection>,
) -> PopupMenuItem {
    let (label, icon) = copy_action_presentation(action);
    let text = connection_copy_text(action, connection, resolved_ssh);
    PopupMenuItem::new(label)
        .icon(icon)
        .disabled(text.is_none())
        .on_click(move |_, window, cx| {
            if let Some(text) = text.clone() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                notify_copy_success(window, cx);
            }
        })
}

fn notify_copy_success(window: &mut Window, cx: &mut gpui::App) {
    #[cfg(not(test))]
    window.push_notification(
        Notification::success(t!("Connection.copy_success").to_string()).autohide(true),
        cx,
    );
    #[cfg(test)]
    let _ = (window, cx);
}

fn copy_full_info_item(connection_id: Option<i64>, home: &Entity<HomePage>) -> PopupMenuItem {
    let home = home.clone();
    PopupMenuItem::new(t!("Connection.copy_full_info").to_string())
        .icon(IconName::Copy)
        .disabled(connection_id.is_none())
        .on_click(move |_, window, cx| {
            let Some(connection_id) = connection_id else {
                return;
            };
            home.update(cx, |home, cx| {
                home.confirm_copy_full_connection_info(connection_id, window, cx);
            });
        })
}

fn copy_action_presentation(action: ConnectionCopyAction) -> (String, IconName) {
    match action {
        ConnectionCopyAction::BasicInfo => label("copy_basic_info", IconName::Copy),
        ConnectionCopyAction::FullInfo => label("copy_full_info", IconName::Copy),
        ConnectionCopyAction::Name => label("copy_connection_name", IconName::Copy),
        ConnectionCopyAction::DatabaseAddress => label("copy_database_target", IconName::Network),
        ConnectionCopyAction::SshTarget => label("copy_ssh_target", IconName::Network),
        ConnectionCopyAction::RedisAddress => label("copy_redis_target", IconName::Network),
        ConnectionCopyAction::MongoDbAddress => label("copy_mongodb_target", IconName::Network),
        ConnectionCopyAction::MqttAddress => label("copy_mqtt_target", IconName::Network),
        ConnectionCopyAction::RemoteDesktopAddress => {
            label("copy_remote_desktop_target", IconName::Network)
        }
        ConnectionCopyAction::TelnetAddress => label("copy_telnet_target", IconName::Network),
        ConnectionCopyAction::Username => label("copy_username", IconName::User),
        ConnectionCopyAction::SerialPort => label("copy_serial_port", IconName::Network),
        ConnectionCopyAction::ForwardingRule => label("copy_forwarding_rule", IconName::Network),
        ConnectionCopyAction::SshCommand => label("copy_ssh_command", IconName::SquareTerminal),
        ConnectionCopyAction::SftpCommand => label("copy_sftp_command", IconName::SquareTerminal),
        ConnectionCopyAction::JdbcUrl => label("copy_jdbc_url", IconName::Network),
        ConnectionCopyAction::CliCommand => {
            label("copy_connection_command", IconName::SquareTerminal)
        }
        ConnectionCopyAction::ConnectionUri => label("copy_connection_uri", IconName::Network),
        ConnectionCopyAction::SerialConfig => label("copy_serial_config", IconName::Copy),
        ConnectionCopyAction::ForwardingCommand => {
            label("copy_forwarding_command", IconName::SquareTerminal)
        }
        ConnectionCopyAction::SentinelConfig => label("copy_sentinel_config", IconName::Copy),
        ConnectionCopyAction::ClusterNodes => label("copy_cluster_nodes", IconName::Copy),
    }
}

fn label(key: &str, icon: IconName) -> (String, IconName) {
    let key = format!("Connection.{key}");
    (t!(&key).to_string(), icon)
}

#[cfg(test)]
mod tests {
    use one_core::storage::{SshAuthMethod, SshParams};

    use super::*;

    fn ssh_connection() -> StoredConnection {
        StoredConnection::new_ssh(
            "SSH".to_string(),
            SshParams {
                sftp_default_directory: None,
                disabled_jump_server: None,
                sftp_account: None,
                host: "ssh.example.test".to_string(),
                port: 22,
                username: "alice".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "secret".to_string(),
                },
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        )
    }

    #[test]
    fn copy_menu_uses_parent_aware_submenu_builder() {
        let production = include_str!("connection_copy_menu.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source exists");

        assert!(
            production.contains("menu.submenu_with_icon("),
            "copy submenu must use PopupMenu's parent-aware builder"
        );
        assert!(
            !production.contains("PopupMenuItem::submenu("),
            "manually inserting a submenu leaves its parent menu unset"
        );
        assert!(
            production.contains("Connection.copy_success"),
            "ordinary copy actions must provide visible success feedback"
        );
    }

    #[test]
    fn ordinary_copy_text_is_available() {
        let connection = ssh_connection();
        assert_eq!(
            connection_copy_text(ConnectionCopyAction::Name, &connection, None),
            Some("SSH".into())
        );
    }
}
