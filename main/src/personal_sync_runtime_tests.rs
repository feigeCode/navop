use gpui::TestAppContext;
use one_core::cloud_sync::personal::PersonalSyncEvent;
use one_core::connection_notifier::ConnectionDataEvent;
use one_core::settings::{AppSettings, PersonalSyncBackendKind, SyncProvider};
use one_core::storage::{DatabaseType, DbConnectionConfig, StoredConnection};

use crate::personal_sync_runtime::{
    actions_enabled, personal_sync_event_from_connection_event, runtime_status,
};
use crate::personal_sync_status::PersonalSyncRuntimeStatus;

#[gpui::test]
fn personal_sync_actions_disabled_without_path(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(AppSettings::default());
    });

    cx.update(|cx| {
        assert!(!actions_enabled(cx));
        assert_eq!(PersonalSyncRuntimeStatus::Disabled, runtime_status(cx));
    });
}

#[gpui::test]
fn personal_sync_actions_enabled_with_configured_path(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().expect("tempdir");
    cx.update(|cx| {
        let mut settings = AppSettings::default();
        settings.sync_provider = SyncProvider::Personal;
        settings.personal_sync.backend = PersonalSyncBackendKind::Folder;
        settings.personal_sync.path = temp.path().to_string_lossy().to_string();
        cx.set_global(settings);
    });

    cx.update(|cx| {
        assert!(actions_enabled(cx));
    });
}

#[gpui::test]
fn personal_sync_actions_disabled_when_onet_cloud_provider_selected(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().expect("tempdir");
    cx.update(|cx| {
        let mut settings = AppSettings::default();
        settings.sync_provider = SyncProvider::OnetCloud;
        settings.personal_sync.backend = PersonalSyncBackendKind::Folder;
        settings.personal_sync.path = temp.path().to_string_lossy().to_string();
        cx.set_global(settings);
    });

    cx.update(|cx| {
        assert!(!actions_enabled(cx));
    });
}

#[test]
fn personal_sync_maps_connection_update_to_local_change() {
    let event = ConnectionDataEvent::ConnectionUpdated {
        connection: test_connection(82),
    };

    assert_eq!(
        Some(PersonalSyncEvent::LocalChanged {
            data_type: one_core::cloud_sync::data_type::CONNECTION.to_string(),
            local_id: "82".to_string(),
        }),
        personal_sync_event_from_connection_event(&event)
    );
}

#[test]
fn personal_sync_maps_connection_delete_with_cloud_id_to_local_delete() {
    assert_eq!(
        Some(PersonalSyncEvent::LocalDeleted {
            data_type: one_core::cloud_sync::data_type::CONNECTION.to_string(),
            cloud_id: "cloud-82".to_string(),
        }),
        personal_sync_event_from_connection_event(&ConnectionDataEvent::ConnectionDeleted {
            connection_id: 82,
            cloud_id: Some("cloud-82".to_string()),
        })
    );
}

#[test]
fn personal_sync_maps_workspace_delete_with_cloud_id_to_local_delete() {
    assert_eq!(
        Some(PersonalSyncEvent::LocalDeleted {
            data_type: one_core::cloud_sync::data_type::WORKSPACE.to_string(),
            cloud_id: "workspace-cloud-3".to_string(),
        }),
        personal_sync_event_from_connection_event(&ConnectionDataEvent::WorkspaceDeleted {
            workspace_id: 3,
            cloud_id: Some("workspace-cloud-3".to_string()),
        })
    );
}

#[test]
fn personal_sync_maps_deletes_without_cloud_id_and_workspace_changes_to_full_scan() {
    assert_eq!(
        Some(PersonalSyncEvent::FullScan),
        personal_sync_event_from_connection_event(&ConnectionDataEvent::ConnectionDeleted {
            connection_id: 82,
            cloud_id: None,
        })
    );
    assert_eq!(
        Some(PersonalSyncEvent::FullScan),
        personal_sync_event_from_connection_event(&ConnectionDataEvent::WorkspaceCreated {
            workspace_id: 3,
        })
    );
    assert_eq!(
        Some(PersonalSyncEvent::FullScan),
        personal_sync_event_from_connection_event(&ConnectionDataEvent::WorkspaceUpdated {
            workspace_id: 3,
        })
    );
    assert_eq!(
        Some(PersonalSyncEvent::FullScan),
        personal_sync_event_from_connection_event(&ConnectionDataEvent::WorkspaceDeleted {
            workspace_id: 3,
            cloud_id: None,
        })
    );
}

#[test]
fn personal_sync_ignores_non_data_change_events() {
    assert_eq!(
        None,
        personal_sync_event_from_connection_event(&ConnectionDataEvent::SchemaChanged {
            connection_id: "82".to_string(),
            database: "demo".to_string(),
            schema: None,
        })
    );
    assert_eq!(
        None,
        personal_sync_event_from_connection_event(&ConnectionDataEvent::CloudSyncRequested)
    );
}

fn test_connection(id: i64) -> StoredConnection {
    let mut connection = StoredConnection::new_database(
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
            extra_params: std::collections::HashMap::new(),
        },
        None,
    );
    connection.id = Some(id);
    connection
}
