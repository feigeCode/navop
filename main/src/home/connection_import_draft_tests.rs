use std::collections::BTreeMap;

use connection_import_protocol::{
    DatabaseImportRecord, ImportDatabaseType, ImportRecord, ImportRecordKind, PasswordImportStatus,
    SshImportAuthMethod, SshImportRecord,
};
use one_core::storage::{ConnectionType, DatabaseType, SshAuthMethod};

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
    draft
        .apply_edit(ImportDraftEdit::Text {
            field: ImportDraftField::Username,
            value: "app_user".to_string(),
        })
        .unwrap();
    draft
        .apply_edit(ImportDraftEdit::Text {
            field: ImportDraftField::Password,
            value: "changed-secret".to_string(),
        })
        .unwrap();
    draft
        .apply_edit(ImportDraftEdit::Text {
            field: ImportDraftField::Database,
            value: "reporting".to_string(),
        })
        .unwrap();

    let stored = selected_import_drafts_to_connections(&[draft]).unwrap();
    let config = stored[0].to_db_connection().unwrap();

    assert_eq!("local mysql", stored[0].name);
    assert_eq!("127.0.0.1", config.host);
    assert_eq!(3307, config.port);
    assert_eq!("app_user", config.username);
    assert_eq!("changed-secret", config.password);
    assert_eq!(Some("reporting".to_string()), config.database);
}

#[test]
fn sqlite_database_draft_uses_database_field_as_file_path() {
    let draft = EditableImportDraft::new(sqlite_import(
        "DBeaver Sample Database (SQLite)",
        Some("/tmp/dbeaver-sample.db"),
    ));

    let stored = draft.to_stored_connection().unwrap();
    let config = stored.to_db_connection().unwrap();

    assert_eq!(DatabaseType::SQLite, config.database_type);
    assert_eq!("/tmp/dbeaver-sample.db", config.host);
    assert_eq!(None, config.database);
}

#[test]
fn sqlite_database_draft_without_file_path_can_open_editor_prefill() {
    let draft = EditableImportDraft::new(sqlite_import("DBeaver Sample Database (SQLite)", None));

    assert_eq!(
        "数据库文件路径不能为空",
        draft.to_stored_connection().unwrap_err()
    );

    let editor_connection = draft.to_editor_connection().unwrap();
    let config = editor_connection.to_db_connection().unwrap();

    assert_eq!(DatabaseType::SQLite, config.database_type);
    assert_eq!("", config.host);
}

#[test]
fn external_mongodb_import_converts_to_native_mongodb_connection() {
    let draft = EditableImportDraft::new(external_database_import("mongo-prod", "mongodb", 27017));

    let stored = draft.to_editor_connection().unwrap();
    let params = stored.to_mongodb_params().unwrap();

    assert_eq!(ConnectionType::MongoDB, stored.connection_type);
    assert_eq!("mongo-prod", stored.name);
    assert_eq!("db.example.test", params.host);
    assert_eq!(Some(27017), params.port);
    assert_eq!(Some("app".to_string()), params.database);
    assert_eq!(Some("root".to_string()), params.username);
}

#[test]
fn external_redis_import_converts_to_native_redis_connection() {
    let draft = EditableImportDraft::new(external_database_import("redis-prod", "redis", 6379));

    let stored = draft.to_editor_connection().unwrap();
    let params = stored.to_redis_params().unwrap();

    assert_eq!(ConnectionType::Redis, stored.connection_type);
    assert_eq!("redis-prod", stored.name);
    assert_eq!("db.example.test", params.host);
    assert_eq!(6379, params.port);
    assert_eq!(Some("root".to_string()), params.username);
    assert_eq!(Some("secret".to_string()), params.password);
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

#[test]
fn mongodb_import_duplicate_detection_matches_native_connection() {
    let draft = EditableImportDraft::new(external_database_import("mongo-prod", "mongodb", 27017));
    let existing = draft.to_stored_connection().unwrap();

    assert_eq!(
        Some("mongo-prod".to_string()),
        duplicate_connection_name(&draft, &[existing]).unwrap()
    );
}

#[test]
fn redis_import_duplicate_detection_matches_native_connection() {
    let draft = EditableImportDraft::new(external_database_import("redis-prod", "redis", 6379));
    let existing = draft.to_stored_connection().unwrap();

    assert_eq!(
        Some("redis-prod".to_string()),
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

fn sqlite_import(name: &str, path: Option<&str>) -> ImportRecord {
    ImportRecord {
        id: format!("dbeaver:{name}"),
        importer_id: "com.onetcli.importer.dbeaver/dbeaver".to_string(),
        source_label: "DBeaver".to_string(),
        source_id: None,
        kind: ImportRecordKind::Database,
        display_name: name.to_string(),
        database: Some(DatabaseImportRecord {
            database_type: ImportDatabaseType::Sqlite,
            name: name.to_string(),
            host: String::new(),
            port: None,
            username: String::new(),
            password: None,
            database: path.map(str::to_string),
            extra_params: BTreeMap::new(),
        }),
        ssh: None,
        port_forwarding: None,
        password_status: PasswordImportStatus::Missing,
        warnings: Vec::new(),
    }
}

fn external_database_import(name: &str, driver_id: &str, port: u16) -> ImportRecord {
    ImportRecord {
        id: format!("external:{name}"),
        importer_id: "com.onetcli.importer.external/external".to_string(),
        source_label: "External".to_string(),
        source_id: None,
        kind: ImportRecordKind::Database,
        display_name: name.to_string(),
        database: Some(DatabaseImportRecord {
            database_type: ImportDatabaseType::External {
                id: driver_id.to_string(),
            },
            name: name.to_string(),
            host: "db.example.test".to_string(),
            port: Some(port),
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
