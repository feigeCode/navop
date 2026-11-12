use std::collections::BTreeMap;

use connection_import_protocol::{
    DatabaseImportRecord, ImportDatabaseType, ImportRecord, ImportRecordKind, ImporterCapabilities,
    ImporterDescriptor, PasswordImportStatus, Platform, SshImportAuthMethod, SshImportRecord,
};
use extension_runtime::connection_import_provider::ImportPreviewError;

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

#[test]
fn import_window_model_exposes_preview_error_on_matching_source() {
    let mut model = ConnectionImportWindowModel::new_for_tests(vec![
        descriptor("broken"),
        descriptor("healthy"),
    ]);

    model.apply_preview_errors(
        &["broken".to_string(), "healthy".to_string()],
        vec![ImportPreviewError {
            importer_id: "broken".to_string(),
            message: "invalid preview payload".to_string(),
        }],
    );

    assert_eq!(
        Some("invalid preview payload"),
        model.sources()[0].preview_error.as_deref()
    );
    assert!(model.sources()[1].preview_error.is_none());
}

#[test]
fn toggling_migration_defaults_reconverts_parsed_rows() {
    let mut model = ConnectionImportWindowModel::empty_for_tests();
    model.apply_preview_records(vec![ssh_record("legacy")]);

    assert!(model.defaults().allow_legacy_algorithms);
    assert!(!model.defaults().disable_shell_integration);

    model.toggle_allow_legacy_algorithms();
    model.toggle_disable_shell_integration();

    let params = model.rows()[0]
        .draft
        .to_stored_connection()
        .expect("convert ssh import")
        .to_ssh_params()
        .expect("ssh params");
    assert_eq!(None, params.allow_legacy_algorithms);
    assert_eq!(Some(true), params.disable_shell_integration);
}

fn ssh_record(name: &str) -> ImportRecord {
    ImportRecord {
        id: name.to_string(),
        importer_id: "securecrt".to_string(),
        source_label: "SecureCRT".to_string(),
        source_id: None,
        kind: ImportRecordKind::Ssh,
        display_name: name.to_string(),
        database: None,
        ssh: Some(SshImportRecord {
            name: name.to_string(),
            host: "ssh.example.test".to_string(),
            port: Some(22),
            username: "deploy".to_string(),
            group_path: None,
            auth_method: SshImportAuthMethod::Password { password: None },
            init_script: None,
            jump_server: None,
            proxy: None,
        }),
        telnet: None,
        port_forwarding: None,
        quick_command: None,
        workspace: None,
        password_status: PasswordImportStatus::Missing,
        warnings: Vec::new(),
    }
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
            supports_manual_directory_pick: false,
            manual_file_pick_prompt: None,
            manual_directory_pick_prompt: None,
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
        telnet: None,
        port_forwarding: None,
        quick_command: None,
        workspace: None,
        password_status: PasswordImportStatus::Unsupported,
        warnings: Vec::new(),
    }
}
