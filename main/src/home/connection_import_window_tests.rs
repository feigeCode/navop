use std::collections::BTreeMap;

use connection_import_protocol::{
    DatabaseImportRecord, ImportDatabaseType, ImportRecord, ImportRecordKind, ImporterCapabilities,
    ImporterDescriptor, PasswordImportStatus, Platform,
};

use super::connection_import_window::ConnectionImportWindowModel;

#[test]
fn import_window_button_state_requires_selected_source_before_scan() {
    let mut model = ConnectionImportWindowModel::new_for_tests(vec![descriptor("dbeaver")]);

    assert!(model.can_scan());

    model.toggle_source("dbeaver");

    assert!(!model.can_scan());
}

#[test]
fn batch_save_only_targets_selected_pending_or_failed_rows() {
    let mut model = ConnectionImportWindowModel::empty_for_tests();
    model.apply_preview_records(vec![database_record("a"), database_record("b")]);
    model.mark_saved("a", Some(1));

    assert_eq!(vec!["b".to_string()], model.batch_save_row_ids());
}

fn descriptor(id: &str) -> ImporterDescriptor {
    ImporterDescriptor {
        id: id.to_string(),
        display_name: id.to_string(),
        description: None,
        icon: None,
        vendor: None,
        supported_platforms: vec![Platform::Macos],
        output_kinds: vec![ImportRecordKind::Database],
        capabilities: ImporterCapabilities {
            supports_scan: true,
            supports_password_import: false,
            supports_manual_file_pick: true,
            manual_file_pick_prompt: None,
            supports_incremental_preview: false,
        },
    }
}

fn database_record(name: &str) -> ImportRecord {
    ImportRecord {
        id: name.to_string(),
        importer_id: "dbeaver".to_string(),
        source_label: "DBeaver".to_string(),
        source_id: None,
        kind: ImportRecordKind::Database,
        display_name: name.to_string(),
        database: Some(DatabaseImportRecord {
            database_type: ImportDatabaseType::MySql,
            name: name.to_string(),
            host: "mysql.example.test".to_string(),
            port: Some(3306),
            username: "root".to_string(),
            password: None,
            database: Some("app".to_string()),
            extra_params: BTreeMap::new(),
        }),
        ssh: None,
        port_forwarding: None,
        password_status: PasswordImportStatus::Unsupported,
        warnings: Vec::new(),
    }
}
