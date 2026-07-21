use super::*;

#[test]
fn open_local_terminal_shortcut_defaults_are_conflict_free() {
    assert_eq!("cmd-alt-t", OPEN_LOCAL_TERMINAL_SHORTCUT_MACOS);
    assert_eq!("alt-t", OPEN_LOCAL_TERMINAL_SHORTCUT_OTHER);
    assert_ne!("ctrl-alt-t", OPEN_LOCAL_TERMINAL_SHORTCUT_OTHER);
}

#[test]
fn local_terminal_launcher_is_visible_in_home_toolbar() {
    let toolbar = include_str!("../toolbar.rs");
    let launcher = include_str!("../local_terminal.rs");

    assert!(toolbar.contains("render_local_terminal_button(window, cx)"));
    assert!(launcher.contains("DropdownButton::new(\"local-terminal-dropdown\")"));
    assert!(launcher.contains("IconName::SquareTerminalColor.color()"));
    assert!(launcher.contains(".checked(kind == default_kind)"));
}

#[test]
fn connection_team_badge_uses_cached_team_name() {
    let teams = vec![TeamOption {
        id: "team-1".to_string(),
        name: "Platform".to_string(),
        key_status: one_core::cloud_sync::TeamKeyCacheStatus::Cached,
        key_version: 1,
        key_verification: None,
        last_verified_at: None,
        role: Some("member".to_string()),
        membership_state: one_core::storage::TeamMembershipState::Active,
    }];

    assert_eq!(
        Some("Platform".to_string()),
        connection_team_badge(Some("team-1"), &teams).map(|badge| badge.name)
    );
    assert!(connection_team_badge(Some("missing"), &teams).is_none());
    assert!(connection_team_badge(None, &teams).is_none());
}

#[test]
fn list_and_card_layouts_render_cached_team_badges() {
    let list_item = include_str!("../connection_list.rs");
    let card = include_str!("../connection_card.rs");

    assert!(list_item.contains("connection_team_badge"));
    assert!(list_item.contains("conn-list-team-"));
    assert!(card.contains("connection_team_badge"));
    assert!(card.contains("conn-team-"));
}

#[test]
fn connection_hover_actions_have_stable_ids() {
    let list_actions = include_str!("../connection_list_actions.rs");
    let card_actions = include_str!("../connection_card_actions.rs");

    assert!(list_actions.contains("conn-list-actions-{}"));
    assert!(card_actions.contains("conn-card-actions-{}"));
}

#[test]
fn home_blocking_work_is_dispatched_off_the_gpui_foreground() {
    let data = include_str!("../data.rs");
    let cloud_sync = include_str!("../cloud_sync.rs");

    assert!(data.contains("cx.background_spawn"));
    assert!(cloud_sync.contains("Tokio::spawn"));
    assert!(!cloud_sync.contains("self.log_sync_decrypt_health"));
}

#[test]
fn connection_render_uses_cached_team_permissions() {
    let list_item = include_str!("../connection_list.rs");
    let card = include_str!("../connection_card.rs");

    assert!(list_item.contains("can_edit_connection_with_cached_teams"));
    assert!(card.contains("can_edit_connection_with_cached_teams"));
    assert!(!list_item.contains("can_edit_connection(&conn, cx)"));
    assert!(!card.contains("can_edit_connection(&conn, cx)"));
}

#[test]
fn team_key_entry_uses_team_management_feature_gate() {
    let toolbar = include_str!("../toolbar.rs");

    assert!(toolbar.contains("is_feature_enabled(Feature::TeamManagement, cx)"));
}

#[test]
fn personal_and_team_keys_share_one_toolbar_menu() {
    let toolbar = include_str!("../toolbar.rs");

    assert!(toolbar.contains("Button::new(\"key-menu-button\")"));
    assert!(toolbar.contains(".dropdown_caret(true)"));
    assert!(toolbar.contains("Encryption.personal_key_unlocked"));
    assert!(toolbar.contains("Encryption.personal_key_locked"));
    assert!(toolbar.contains("Encryption.team_key"));
    assert!(!toolbar.contains("Button::new(\"team-key-button\")"));
}

#[test]
fn home_overview_is_compact_and_avoids_duplicate_search() {
    let toolbar = include_str!("../toolbar.rs");
    let content = include_str!("../content.rs");
    let card = include_str!("../connection_card.rs");

    assert!(toolbar.contains("Input::new(&self.search_input)"));
    assert!(content.contains("max_w(px(1160.0))"));
    assert!(content.contains("w(px(280.0))"));
    assert!(card.contains("h(px(76.0))"));
    assert!(!card.contains(".shadow_sm()\n            .group"));
}

#[test]
fn sidebar_search_aligns_with_home_toolbar_height() {
    let tree = include_str!("../../persistent_connection_sidebar/tree.rs");
    assert!(tree.contains("fn render_tree_search"));
    assert!(tree.contains(".h_10()"));
}

#[test]
fn team_key_settings_tab_has_feature_guard() {
    let source = include_str!("../../home/home_tabs.rs");
    let entry = source
        .split("pub(crate) fn add_team_key_settings_tab(")
        .nth(1)
        .expect("team key settings entry exists")
        .split("pub(crate) fn add_extensions_tab(")
        .next()
        .expect("team key settings entry has an end marker");

    assert!(entry.contains("is_feature_enabled(Feature::TeamManagement, cx)"));
}

#[test]
fn home_render_uses_cached_external_driver_registry() {
    let home = include_str!("../../home_tab.rs");
    let icon = include_str!("../connection_icon.rs");
    let list_item = include_str!("../connection_list.rs");
    let card = include_str!("../connection_card_content.rs");
    let quick_open = include_str!("../../home/home_connection_quick_open.rs");

    assert!(home.contains("external_driver_registry: IpcDriverRegistry"));
    assert!(icon.contains("external_driver_icon_for_config_with_registry"));
    assert!(list_item.contains("connection_icon"));
    assert!(card.contains("connection_icon"));
    assert!(quick_open.contains("external_driver_icon_for_config_with_registry"));
    assert!(quick_open.contains("external_driver_registry: IpcDriverRegistry"));
    assert!(!icon.contains("IpcDriverRegistry::load_default()"));
    assert!(!quick_open.contains("IpcDriverRegistry::load_default()"));
}
