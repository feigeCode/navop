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
