use std::collections::BTreeMap;

use connection_import_protocol::{
    DatabaseImportRecord, ImportDatabaseType, ImportRecord, ImportRecordKind, PasswordImportStatus,
    SshImportAuthMethod, SshImportRecord,
};
use one_core::storage::{ConnectionType, SshAuthMethod};

use super::connection_import_actions::duplicate_connection_name;
use super::connection_import_draft::{
    EditableImportDraft, ImportDraftEdit, ImportDraftField, selected_import_count,
    selected_import_drafts_to_connections,
};

#[test]
fn imported_drafts_are_selected_by_default_and_can_be_unselected() {
    let mut drafts = vec![
        EditableImportDraft::new(database_import("db")),
        EditableImportDraft::new(ssh_import("ssh")),
    ];

    assert_eq!(2, selected_import_count(&drafts));

    drafts[1]
        .apply_edit(ImportDraftEdit::Selected(false))
        .unwrap();

    assert_eq!(1, selected_import_count(&drafts));
}

#[test]
fn edited_database_draft_is_converted_to_stored_connection() {
    let mut draft = EditableImportDraft::new(database_import("prod"));
    draft
        .apply_edit(ImportDraftEdit::Text {
            field: ImportDraftField::Name,
            value: "local mysql".to_string(),
        })
        .unwrap();
    draft
        .apply_edit(ImportDraftEdit::Text {
            field: ImportDraftField::Host,
            value: "127.0.0.1".to_string(),
        })
        .unwrap();
    draft
        .apply_edit(ImportDraftEdit::Text {
            field: ImportDraftField::Port,
            value: "3307".to_string(),
        })
        .unwrap();

    let stored = selected_import_drafts_to_connections(&[draft]).unwrap();
    let config = stored[0].to_db_connection().unwrap();

    assert_eq!("local mysql", stored[0].name);
    assert_eq!("127.0.0.1", config.host);
    assert_eq!(3307, config.port);
}

#[test]
fn only_selected_drafts_are_converted() {
    let selected = EditableImportDraft::new(database_import("selected"));
    let mut skipped = EditableImportDraft::new(database_import("skipped"));
    skipped
        .apply_edit(ImportDraftEdit::Selected(false))
        .unwrap();

    let stored = selected_import_drafts_to_connections(&[selected, skipped]).unwrap();

    assert_eq!(1, stored.len());
    assert_eq!("selected", stored[0].name);
}

#[test]
fn edited_ssh_private_key_path_is_converted_to_stored_connection() {
    let mut draft = EditableImportDraft::new(ssh_import("jump"));
    draft
        .apply_edit(ImportDraftEdit::Text {
            field: ImportDraftField::PrivateKeyPath,
            value: "/tmp/id_ed25519".to_string(),
        })
        .unwrap();

    let stored = selected_import_drafts_to_connections(&[draft]).unwrap();
    let params = stored[0].to_ssh_params().unwrap();

    assert_eq!(ConnectionType::SshSftp, stored[0].connection_type);
    assert_eq!("jump", stored[0].name);
    assert_eq!("ssh.example.test", params.host);
    assert_eq!(2222, params.port);
    assert!(matches!(
        params.auth_method,
        SshAuthMethod::PrivateKey { ref key_path, .. } if key_path == "/tmp/id_ed25519"
    ));
}

#[test]
fn database_duplicate_identity_uses_type_host_port_username_and_database() {
    let draft = EditableImportDraft::new(database_import("prod"));

    assert_eq!(
        "db:mysql:mysql.example.test:3306:root:app",
        draft.duplicate_identity().unwrap()
    );
}

#[test]
fn ssh_duplicate_identity_uses_host_port_and_username() {
    let draft = EditableImportDraft::new(ssh_import("jump"));

    assert_eq!(
        "ssh:ssh.example.test:2222:deploy",
        draft.duplicate_identity().unwrap()
    );
}

#[test]
fn duplicate_detection_matches_existing_connection_identity() {
    let draft = EditableImportDraft::new(database_import("prod"));
    let existing = draft.to_stored_connection().unwrap();

    assert_eq!(
        Some("prod".to_string()),
        duplicate_connection_name(&draft, &[existing]).unwrap()
    );
}

fn database_import(name: &str) -> ImportRecord {
    ImportRecord {
        id: format!("datagrip:{name}"),
        importer_id: "com.onetcli.importer.datagrip/datagrip".to_string(),
        source_label: "DataGrip".to_string(),
        source_id: None,
        kind: ImportRecordKind::Database,
        display_name: name.to_string(),
        database: Some(DatabaseImportRecord {
            database_type: ImportDatabaseType::MySql,
            name: name.to_string(),
            host: "mysql.example.test".to_string(),
            port: Some(3306),
            username: "root".to_string(),
            password: Some("secret".to_string()),
            database: Some("app".to_string()),
            extra_params: BTreeMap::new(),
        }),
        ssh: None,
        port_forwarding: None,
        password_status: PasswordImportStatus::Included,
        warnings: Vec::new(),
    }
}

fn ssh_import(name: &str) -> ImportRecord {
    ImportRecord {
        id: format!("xshell:{name}"),
        importer_id: "com.onetcli.importer.xshell/xshell".to_string(),
        source_label: "Xshell".to_string(),
        source_id: None,
        kind: ImportRecordKind::Ssh,
        display_name: name.to_string(),
        database: None,
        ssh: Some(SshImportRecord {
            name: name.to_string(),
            host: "ssh.example.test".to_string(),
            port: Some(2222),
            username: "deploy".to_string(),
            auth_method: SshImportAuthMethod::PrivateKey {
                key_path: "~/.ssh/id_rsa".to_string(),
                passphrase: None,
            },
            init_script: None,
            jump_server: None,
            proxy: None,
        }),
        port_forwarding: None,
        password_status: PasswordImportStatus::Unsupported,
        warnings: Vec::new(),
    }
}
