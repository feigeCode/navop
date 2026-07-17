use super::*;
use public_mcp::client_config::ClientConfigHealth;

#[test]
fn client_config_health_labels_match_config_states() {
    assert_eq!(
        "Settings.General.Mcp.client_config_status_up_to_date",
        client_config_health_label_key(ClientConfigHealth::UpToDate)
    );
    assert_eq!(
        "Settings.General.Mcp.client_config_status_not_installed",
        client_config_health_label_key(ClientConfigHealth::NotInstalled)
    );
    assert_eq!(
        "Settings.General.Mcp.client_config_status_needs_repair",
        client_config_health_label_key(ClientConfigHealth::NeedsRepair)
    );
    assert_eq!(
        "Settings.General.Mcp.client_config_status_needs_repair",
        client_config_health_label_key(ClientConfigHealth::NeedsMigration)
    );
    assert_eq!(
        "Settings.General.Mcp.client_config_status_needs_repair",
        client_config_health_label_key(ClientConfigHealth::PackageVersionOutdated)
    );
    assert_eq!(
        "Settings.General.Mcp.client_config_status_missing_helper",
        client_config_health_label_key(ClientConfigHealth::MissingHelper)
    );
    assert_eq!(
        "Settings.General.Mcp.client_config_status_unusable_helper",
        client_config_health_label_key(ClientConfigHealth::UnusableHelper)
    );
}

#[test]
fn client_config_action_button_is_disabled_when_helper_is_unavailable() {
    assert!(!client_config_action_enabled(
        ClientConfigHealth::MissingHelper
    ));
    assert!(!client_config_action_enabled(
        ClientConfigHealth::UnusableHelper
    ));
}

#[test]
fn client_config_action_button_is_enabled_for_all_config_health_states() {
    assert!(client_config_action_enabled(
        ClientConfigHealth::NotInstalled
    ));
    assert!(client_config_action_enabled(
        ClientConfigHealth::NeedsRepair
    ));
    assert!(client_config_action_enabled(ClientConfigHealth::UpToDate));
}

#[test]
fn client_config_button_shows_uninstall_when_up_to_date_and_install_otherwise() {
    assert_eq!(
        t!("Settings.General.Mcp.uninstall_client_config").to_string(),
        client_config_action_label(ClientConfigHealth::UpToDate)
    );
    assert_eq!(
        t!("Settings.General.Mcp.install_client_config").to_string(),
        client_config_action_label(ClientConfigHealth::NotInstalled)
    );
    assert_eq!(
        t!("Settings.General.Mcp.install_client_config").to_string(),
        client_config_action_label(ClientConfigHealth::NeedsRepair)
    );
}
