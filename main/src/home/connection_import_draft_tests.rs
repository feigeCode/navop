use connection_import_protocol::{
    DatabaseImportRecord, ImportDatabaseType, ImportRecord, ImportRecordKind, PasswordImportStatus,
    SshImportAuthMethod, SshImportRecord, SshJumpServerImportRecord, SshProxyImportKind,
    SshProxyImportRecord, WorkspaceImportRecord,
};
use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, DatabaseType, GlobalStorageState, ProxyType,
    SshAuthMethod, StorageManager, WorkspaceRepository,
};
use std::collections::BTreeMap;

use super::connection_import_actions::{ save_import_draft};
use super::connection_import_draft::{
    EditableImportDraft, ImportDraftEdit, ImportDraftField, ImportDraftKind, selected_import_count,
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
fn workspace_import_draft_preserves_nested_path() {
    let draft = EditableImportDraft::new(workspace_import("Production/Staging"));

    assert_eq!(ImportDraftKind::Workspace, draft.kind());
    assert_eq!(
        Some("Production/Staging".to_string()),
        draft.workspace_path()
    );
}

#[test]
fn workspace_import_cannot_be_converted_to_a_connection() {
    let draft = EditableImportDraft::new(workspace_import("Production/Staging"));

    assert_eq!(
        rust_i18n::t!("Home.ConnectionImport.workspace_save_unsupported"),
        draft.to_stored_connection().unwrap_err()
    );
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

#[gpui::test]
fn saving_ssh_import_creates_and_reuses_nested_workspaces(cx: &mut gpui::TestAppContext) {
    let temp = tempfile::tempdir().expect("tempdir");
    let conn = SqliteConnection::open(temp.path().join("import.db")).expect("sqlite");
    conn.with_connection(|conn| run_migrations(conn))
        .expect("migrations");
    let storage = StorageManager::new_with_connection(conn);
    storage.register(ConnectionRepository::new(storage.connection()));
    storage.register(WorkspaceRepository::new(storage.connection()));
    cx.update(|cx| cx.set_global(GlobalStorageState { storage }));

    let mut record = ssh_import("api");
    record.ssh.as_mut().unwrap().group_path = Some("Production / Staging".to_string());
    let draft = EditableImportDraft::new(record);
    let first_connection_id = match cx
        .update(|cx| save_import_draft(&draft, cx))
        .expect("save first SSH import")
    {
        super::connection_import_actions::ImportSaveResult::Saved { connection_id } => {
            connection_id
        }
        super::connection_import_actions::ImportSaveResult::SkippedDuplicate { .. } => {
            panic!("first SSH import should be saved")
        }
    };

    let (production, staging) = cx.update(|cx| {
        let workspaces = cx
            .global::<GlobalStorageState>()
            .storage
            .get::<WorkspaceRepository>()
            .unwrap()
            .list()
            .expect("list workspaces after first save");
        let production = workspaces
            .iter()
            .find(|workspace| workspace.name == "Production")
            .expect("Production workspace");
        let staging = workspaces
            .iter()
            .find(|workspace| workspace.name == "Staging")
            .expect("Staging workspace");
        (production.clone(), staging.clone())
    });
    assert_eq!(None, production.parent_id);
    assert_eq!(production.id, staging.parent_id);

    let first = cx
        .update(|cx| {
            cx.global::<GlobalStorageState>()
                .storage
                .get::<ConnectionRepository>()
                .unwrap()
                .list()
                .expect("list connections after first save")
        })
        .into_iter()
        .find(|connection| connection.id == first_connection_id)
        .expect("saved first connection");
    assert_eq!(staging.id, first.workspace_id);

    let mut duplicate_record = ssh_import("api-different-endpoint");
    duplicate_record.ssh.as_mut().unwrap().host = "another.example.test".to_string();
    duplicate_record.ssh.as_mut().unwrap().group_path = Some(" Production / Staging ".to_string());
    let duplicate = EditableImportDraft::new(duplicate_record);
    let second_connection_id = match cx
        .update(|cx| save_import_draft(&duplicate, cx))
        .expect("save second SSH import")
    {
        super::connection_import_actions::ImportSaveResult::Saved { connection_id } => {
            connection_id
        }
        super::connection_import_actions::ImportSaveResult::SkippedDuplicate { .. } => {
            panic!("second SSH import should be saved")
        }
    };
    let second = cx
        .update(|cx| {
            cx.global::<GlobalStorageState>()
                .storage
                .get::<ConnectionRepository>()
                .unwrap()
                .list()
                .expect("list connections after second save")
        })
        .into_iter()
        .find(|connection| connection.id == second_connection_id)
        .expect("saved second connection");

    let final_workspaces = cx
        .update(|cx| {
            cx.global::<GlobalStorageState>()
                .storage
                .get::<WorkspaceRepository>()
                .unwrap()
                .list()
        })
        .expect("list workspaces after second save");
    assert_eq!(2, final_workspaces.len());
    assert_eq!(staging.id, second.workspace_id);
}

#[gpui::test]
fn saving_ssh_import_normalizes_windows_workspace_group_paths(cx: &mut gpui::TestAppContext) {
    let temp = tempfile::tempdir().expect("tempdir");
    let conn = SqliteConnection::open(temp.path().join("import.db")).expect("sqlite");
    conn.with_connection(|conn| run_migrations(conn))
        .expect("migrations");
    let storage = StorageManager::new_with_connection(conn);
    storage.register(ConnectionRepository::new(storage.connection()));
    storage.register(WorkspaceRepository::new(storage.connection()));
    cx.update(|cx| cx.set_global(GlobalStorageState { storage }));

    let mut record = ssh_import("windows-group");
    record.ssh.as_mut().unwrap().group_path = Some(r"Production\Staging".to_string());
    let draft = EditableImportDraft::new(record);

    cx.update(|cx| save_import_draft(&draft, cx))
        .expect("save SSH import");

    cx.update(|cx| {
        let workspaces = cx
            .global::<GlobalStorageState>()
            .storage
            .get::<WorkspaceRepository>()
            .unwrap()
            .list()
            .expect("list workspaces");
        let production = workspaces
            .iter()
            .find(|workspace| workspace.name == "Production")
            .expect("Production workspace");
        let staging = workspaces
            .iter()
            .find(|workspace| workspace.name == "Staging")
            .expect("Staging workspace");
        assert_eq!(None, production.parent_id);
        assert_eq!(Some(production.id), Some(staging.parent_id));
    });
}

#[gpui::test]
fn saving_workspace_import_creates_reuses_and_normalizes_nested_workspaces(
    cx: &mut gpui::TestAppContext,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let conn = SqliteConnection::open(temp.path().join("import.db")).expect("sqlite");
    conn.with_connection(|conn| run_migrations(conn))
        .expect("migrations");
    let storage = StorageManager::new_with_connection(conn);
    storage.register(ConnectionRepository::new(storage.connection()));
    storage.register(WorkspaceRepository::new(storage.connection()));
    cx.update(|cx| cx.set_global(GlobalStorageState { storage }));

    for path in ["Production/Staging", r" Production\Staging "] {
        let draft = EditableImportDraft::new(workspace_import(path));
        let result = cx
            .update(|cx| save_import_draft(&draft, cx))
            .expect("save workspace import");
        assert_eq!(
            super::connection_import_actions::ImportSaveResult::Saved {
                connection_id: None,
            },
            result
        );
    }

    cx.update(|cx| {
        let storage = &cx.global::<GlobalStorageState>().storage;
        let workspaces = storage
            .get::<WorkspaceRepository>()
            .unwrap()
            .list()
            .expect("list workspaces");
        assert_eq!(2, workspaces.len());
        let production = workspaces
            .iter()
            .find(|workspace| workspace.name == "Production")
            .expect("Production workspace");
        let staging = workspaces
            .iter()
            .find(|workspace| workspace.name == "Staging")
            .expect("Staging workspace");
        assert_eq!(None, production.parent_id);
        assert_eq!(production.id, staging.parent_id);

        let connections = storage
            .get::<ConnectionRepository>()
            .unwrap()
            .list()
            .expect("list connections");
        assert!(connections.is_empty());
    });
}

#[gpui::test]
fn duplicate_ssh_import_assigns_group_workspace_to_existing_ungrouped_connection(
    cx: &mut gpui::TestAppContext,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let conn = SqliteConnection::open(temp.path().join("import.db")).expect("sqlite");
    conn.with_connection(|conn| run_migrations(conn))
        .expect("migrations");
    let storage = StorageManager::new_with_connection(conn);
    storage.register(ConnectionRepository::new(storage.connection()));
    storage.register(WorkspaceRepository::new(storage.connection()));
    cx.update(|cx| cx.set_global(GlobalStorageState { storage }));

    let mut existing = EditableImportDraft::new(ssh_import("existing"))
        .to_stored_connection()
        .expect("existing connection");
    let existing_id = cx.update(|cx| {
        let repo = cx
            .global::<GlobalStorageState>()
            .storage
            .get::<ConnectionRepository>()
            .expect("connection repository");
        repo.insert(&mut existing).expect("insert existing");
        existing.id.expect("existing connection id")
    });

    let mut duplicate_record = ssh_import("duplicate-with-group");
    duplicate_record.ssh.as_mut().unwrap().group_path = Some("Production/Staging".to_string());
    let duplicate = EditableImportDraft::new(duplicate_record);

    let result = cx
        .update(|cx| save_import_draft(&duplicate, cx))
        .expect("apply imported workspace");
    assert_eq!(
        super::connection_import_actions::ImportSaveResult::Saved {
            connection_id: Some(existing_id),
        },
        result
    );

    cx.update(|cx| {
        let storage = &cx.global::<GlobalStorageState>().storage;
        let connections = storage
            .get::<ConnectionRepository>()
            .unwrap()
            .list()
            .expect("list connections");
        assert_eq!(1, connections.len());

        let workspaces = storage
            .get::<WorkspaceRepository>()
            .unwrap()
            .list()
            .expect("list workspaces");
        let production = workspaces
            .iter()
            .find(|workspace| workspace.name == "Production")
            .expect("Production workspace");
        let staging = workspaces
            .iter()
            .find(|workspace| workspace.name == "Staging")
            .expect("Staging workspace");
        assert_eq!(None, production.parent_id);
        assert_eq!(production.id, staging.parent_id);
        assert_eq!(staging.id, connections[0].workspace_id);
    });
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
        rust_i18n::t!("Home.ConnectionImport.database_file_required"),
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
fn imported_ssh_private_key_material_is_converted_to_stored_connection() {
    let mut record = ssh_import("inline-key");
    let ssh = record.ssh.as_mut().expect("ssh record should exist");
    ssh.auth_method = SshImportAuthMethod::PrivateKeyMaterial {
        private_key: Some("-----BEGIN OPENSSH PRIVATE KEY-----\nfixture\n".to_string()),
        passphrase: Some("secret".to_string()),
        file_name_hint: Some("id_ed25519".to_string()),
    };
    let draft = EditableImportDraft::new(record);

    let stored = selected_import_drafts_to_connections(&[draft]).unwrap();
    let params = stored[0].to_ssh_params().unwrap();

    assert!(matches!(
        params.auth_method,
        SshAuthMethod::PrivateKeyContent {
            ref private_key,
            passphrase: Some(ref passphrase),
        } if private_key.contains("OPENSSH PRIVATE KEY") && passphrase == "secret"
    ));
}

#[test]
fn imported_ssh_extended_options_are_converted_to_stored_connection() {
    let mut record = ssh_import("extended");
    let ssh = record.ssh.as_mut().expect("ssh record should exist");
    ssh.init_script = Some("echo ready".to_string());
    ssh.jump_server = Some(SshJumpServerImportRecord {
        host: "bastion.example.test".to_string(),
        port: 2200,
        username: "jump-user".to_string(),
        auth_method: SshImportAuthMethod::Password {
            password: Some("jump-secret".to_string()),
        },
    });
    ssh.proxy = Some(SshProxyImportRecord {
        kind: SshProxyImportKind::Socks5,
        host: "proxy.example.test".to_string(),
        port: 1080,
        username: Some("proxy-user".to_string()),
        password: Some("proxy-secret".to_string()),
    });

    let stored =
        selected_import_drafts_to_connections(&[EditableImportDraft::new(record)]).unwrap();
    let params = stored[0].to_ssh_params().unwrap();

    assert_eq!(Some("echo ready".to_string()), params.init_script);
    let jump = params.jump_server.expect("jump server should be preserved");
    assert_eq!("bastion.example.test", jump.host);
    assert_eq!(2200, jump.port);
    assert_eq!("jump-user", jump.username);
    assert!(matches!(
        jump.auth_method,
        SshAuthMethod::Password { ref password } if password == "jump-secret"
    ));
    assert!(jump.credential_reference.is_none());

    let proxy = params.proxy.expect("proxy should be preserved");
    assert_eq!(ProxyType::Socks5, proxy.proxy_type);
    assert_eq!("proxy.example.test", proxy.host);
    assert_eq!(1080, proxy.port);
    assert_eq!(Some("proxy-user".to_string()), proxy.username);
    assert_eq!(Some("proxy-secret".to_string()), proxy.password);
    assert!(proxy.credential_reference.is_none());
}

#[test]
fn imported_jump_server_private_key_uses_its_own_key_path() {
    let mut record = ssh_import("jump-key");
    let ssh = record.ssh.as_mut().expect("ssh record should exist");
    ssh.jump_server = Some(SshJumpServerImportRecord {
        host: "bastion.example.test".to_string(),
        port: 22,
        username: "jump-user".to_string(),
        auth_method: SshImportAuthMethod::PrivateKey {
            key_path: "/jump/.ssh/id_ed25519".to_string(),
            passphrase: Some("passphrase".to_string()),
        },
    });
    let mut draft = EditableImportDraft::new(record);
    draft
        .apply_edit(ImportDraftEdit::Text {
            field: ImportDraftField::PrivateKeyPath,
            value: "/target/.ssh/id_rsa".to_string(),
        })
        .unwrap();

    let stored = selected_import_drafts_to_connections(&[draft]).unwrap();
    let params = stored[0].to_ssh_params().unwrap();
    let jump = params.jump_server.expect("jump server should be preserved");

    assert!(matches!(
        jump.auth_method,
        SshAuthMethod::PrivateKey {
            ref key_path,
            passphrase: Some(ref passphrase),
        } if key_path == "/jump/.ssh/id_ed25519" && passphrase == "passphrase"
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
        quick_command: None,
        workspace: None,
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
        quick_command: None,
        workspace: None,
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
        quick_command: None,
        workspace: None,
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
            group_path: None,
            auth_method: SshImportAuthMethod::PrivateKey {
                key_path: "~/.ssh/id_rsa".to_string(),
                passphrase: None,
            },
            init_script: None,
            jump_server: None,
            proxy: None,
        }),
        port_forwarding: None,
        quick_command: None,
        workspace: None,
        password_status: PasswordImportStatus::Unsupported,
        warnings: Vec::new(),
    }
}

fn workspace_import(path: &str) -> ImportRecord {
    ImportRecord {
        id: format!("securecrt:workspace:{path}"),
        importer_id: "com.onetcli.importer.securecrt/securecrt".to_string(),
        source_label: "SecureCRT".to_string(),
        source_id: Some(format!("Sessions/{path}")),
        kind: ImportRecordKind::Workspace,
        display_name: path.rsplit(['/', '\\']).next().unwrap_or(path).to_string(),
        database: None,
        ssh: None,
        port_forwarding: None,
        quick_command: None,
        workspace: Some(WorkspaceImportRecord {
            path: path.to_string(),
        }),
        password_status: PasswordImportStatus::Missing,
        warnings: Vec::new(),
    }
}
