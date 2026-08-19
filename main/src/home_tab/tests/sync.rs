use super::*;
use one_core::cloud_sync::{CloudSyncData, ConflictType};

#[test]
fn refreshed_pending_conflicts_clears_stale_conflicts_after_clean_sync() {
    let previous = vec![sync_conflict("cloud-1")];

    let refreshed = refreshed_pending_conflicts(previous, Vec::new(), &[]);

    assert!(refreshed.is_empty());
}

#[test]
fn refreshed_pending_conflicts_keeps_previous_when_sync_errors_without_fresh_conflicts() {
    let previous = vec![sync_conflict("cloud-1")];
    let errors = vec!["network failed".to_string()];

    let refreshed = refreshed_pending_conflicts(previous.clone(), Vec::new(), &errors);

    assert_eq!(1, refreshed.len());
    assert_eq!(previous[0].cloud.id, refreshed[0].cloud.id);
}

#[test]
fn sync_route_uses_selected_provider() {
    assert_eq!(
        HomeSyncRoute::OnetCloud,
        sync_route_for_provider(SyncProvider::OnetCloud)
    );
    assert_eq!(
        HomeSyncRoute::Personal,
        sync_route_for_provider(SyncProvider::Personal)
    );
}

#[test]
fn disabled_global_sync_keeps_home_sync_actionable_for_settings_prompt() {
    let state = home_sync_button_state(HomeSyncButtonContext {
        route: HomeSyncRoute::OnetCloud,
        sync_enabled: false,
        is_logged_in: true,
        has_sync_license: true,
        onet_syncing: false,
        personal_sync_ready: false,
        personal_syncing: false,
    });

    assert_eq!(HomeSyncButtonState::NeedsSettings, state);
    assert!(!state.is_disabled());
}

#[test]
fn home_sync_remains_disabled_for_busy_and_unavailable_states() {
    assert!(
        home_sync_button_state(HomeSyncButtonContext {
            route: HomeSyncRoute::OnetCloud,
            sync_enabled: false,
            is_logged_in: true,
            has_sync_license: true,
            onet_syncing: true,
            personal_sync_ready: false,
            personal_syncing: false,
        })
        .is_disabled()
    );
    assert!(
        home_sync_button_state(HomeSyncButtonContext {
            route: HomeSyncRoute::OnetCloud,
            sync_enabled: true,
            is_logged_in: false,
            has_sync_license: true,
            onet_syncing: false,
            personal_sync_ready: false,
            personal_syncing: false,
        })
        .is_disabled()
    );
    assert!(
        home_sync_button_state(HomeSyncButtonContext {
            route: HomeSyncRoute::Personal,
            sync_enabled: true,
            is_logged_in: true,
            has_sync_license: true,
            onet_syncing: false,
            personal_sync_ready: false,
            personal_syncing: false,
        })
        .is_disabled()
    );
    assert!(
        home_sync_button_state(HomeSyncButtonContext {
            route: HomeSyncRoute::Personal,
            sync_enabled: false,
            is_logged_in: true,
            has_sync_license: true,
            onet_syncing: false,
            personal_sync_ready: true,
            personal_syncing: true,
        })
        .is_disabled()
    );
}

#[test]
fn configured_home_sync_action_is_ready() {
    assert_eq!(
        HomeSyncButtonState::Ready,
        home_sync_button_state(HomeSyncButtonContext {
            route: HomeSyncRoute::OnetCloud,
            sync_enabled: true,
            is_logged_in: true,
            has_sync_license: true,
            onet_syncing: false,
            personal_sync_ready: false,
            personal_syncing: false,
        })
    );
    assert_eq!(
        HomeSyncButtonState::Ready,
        home_sync_button_state(HomeSyncButtonContext {
            route: HomeSyncRoute::Personal,
            sync_enabled: true,
            is_logged_in: true,
            has_sync_license: true,
            onet_syncing: false,
            personal_sync_ready: true,
            personal_syncing: false,
        })
    );
}

#[test]
fn every_home_sync_entry_uses_the_guarded_click_handler() {
    let toolbar = include_str!("../toolbar.rs");
    let modern_home = include_str!("../modern_home.rs");

    assert!(toolbar.contains("this.handle_sync_click(window, cx);"));
    assert!(modern_home.contains("home.handle_sync_click(window, cx);"));
}

#[test]
fn disabled_sync_dialog_opens_the_sync_settings_page() {
    let cloud_sync = include_str!("../cloud_sync.rs");
    let home_tabs = include_str!("../../home/home_tabs.rs");

    assert!(cloud_sync.contains("Home.sync_disabled_title"));
    assert!(cloud_sync.contains("home.add_sync_settings_tab(window, cx);"));
    assert!(home_tabs.contains("SettingsPanel::new_sync(win, cx)"));
}

#[test]
fn team_key_menu_item_is_visible_only_for_onetcloud_with_cached_teams() {
    assert!(should_show_team_key_menu_item(HomeSyncRoute::OnetCloud, 1));
    assert!(should_show_team_key_menu_item(HomeSyncRoute::OnetCloud, 3));
    assert!(!should_show_team_key_menu_item(HomeSyncRoute::OnetCloud, 0));
    assert!(!should_show_team_key_menu_item(HomeSyncRoute::Personal, 1));
}

#[test]
fn team_management_entry_follows_feature_gate() {
    assert!(should_show_team_management_entry(true));
    assert!(!should_show_team_management_entry(false));
}

#[test]
fn successful_cloud_sync_announces_team_cache_update() {
    let cloud_sync = include_str!("../cloud_sync.rs");
    let success = cloud_sync
        .split("Ok(stats) =>")
        .nth(1)
        .expect("sync success branch exists")
        .split("Err(e) =>")
        .next()
        .expect("sync success branch has an end marker");

    assert!(success.contains("ConnectionDataEvent::TeamCacheUpdated"));
}

fn sync_conflict(cloud_id: &str) -> SyncConflict {
    SyncConflict {
        local: StoredConnection::new_database(
            "demo".to_string(),
            DbConnectionConfig {
                id: String::new(),
                database_type: DatabaseType::MySQL,
                name: "demo".to_string(),
                host: "localhost".to_string(),
                port: 3306,
                username: String::new(),
                password: String::new(),
                credential_reference: None,
                database: None,
                service_name: None,
                sid: None,
                workspace_id: None,
                proxy: None,
                extra_params: std::collections::HashMap::new(),
            },
            None,
        ),
        cloud: CloudSyncData {
            id: cloud_id.to_string(),
            owner_id: "owner".to_string(),
            team_id: None,
            data_type: one_core::cloud_sync::data_type::CONNECTION.to_string(),
            encrypted_data: String::new(),
            key_version: 1,
            checksum: String::new(),
            version: 1,
            updated_at: 1,
            deleted_at: None,
        },
        cloud_name: "demo".to_string(),
        conflict_type: ConflictType::LocalModifiedCloudDeleted,
    }
}
