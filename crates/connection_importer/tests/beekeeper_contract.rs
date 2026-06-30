use connection_importer::{
    ImportOptions, ImportSourceKind, PasswordImportStatus, preview_connections_from_path,
};
use one_core::storage::DatabaseType;
use rusqlite::{Connection, params};
use std::fs;
use std::path::Path;

const ENCRYPTED_KEY_FILE: &str = "dbbae2c0721679d0a4830122a784912039ef84cd09aaf7ff9834a8060ce0fb0f757afa80805cb338d7e5957eced3bbc7+RSw16suTeBgjq5xTkW2gWwjkf0fiqOEp3Mgbnjgk/bOvNiopNdcXhHj+kvh6j7XMiEE8uLZ+jeq9C99QVTug5GY1W1Rkxgl3i07YLc5MJ7LwsAGk3E3/iehQ65D/oer";
const ENCRYPTED_PASSWORD: &str = "1e2ce9febe48a9fd8f12d47f8bcbfe92e255470cb9ae907e0c141cca0647ce5488a60624a6d2ea7bedcfb98830625c92U6l3ilSfRXcUmuV0CGjkfuOdBVaLXEaO93ebuUcGVrY=";

#[test]
fn beekeeper_import_reads_saved_connections_from_app_db() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let app_db = temp_dir.path().join("app.db");
    let db = create_app_db(&app_db);
    insert_connection(
        &db,
        Row {
            id: 1,
            name: "Beekeeper MySQL",
            connection_type: "mysql",
            host: Some("mysql.example.com"),
            port: None,
            username: Some("app"),
            default_database: Some("shop"),
            password: None,
            remember_password: true,
        },
    );
    insert_connection(
        &db,
        Row {
            id: 2,
            name: "Beekeeper SQLite",
            connection_type: "sqlite",
            host: None,
            port: None,
            username: None,
            default_database: Some("/tmp/local.sqlite"),
            password: None,
            remember_password: false,
        },
    );

    let imported = preview_connections_from_path(
        ImportSourceKind::BeekeeperStudio,
        &app_db,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("Beekeeper app db should preview");

    assert_eq!(2, imported.len());
    assert_eq!(ImportSourceKind::BeekeeperStudio, imported[0].source);
    assert_eq!("1", imported[0].source_id);
    assert_eq!("Beekeeper MySQL", imported[0].name);
    assert_eq!(DatabaseType::MySQL, imported[0].database_type);
    assert_eq!("mysql.example.com", imported[0].host);
    assert_eq!(Some(3306), imported[0].port);
    assert_eq!("app", imported[0].username);
    assert_eq!(Some("shop".to_string()), imported[0].database);
    assert_eq!(
        PasswordImportStatus::Unsupported,
        imported[0].password_status
    );

    assert_eq!(DatabaseType::SQLite, imported[1].database_type);
    assert_eq!("/tmp/local.sqlite", imported[1].host);
    assert_eq!(None, imported[1].port);
}

#[test]
fn beekeeper_import_decrypts_password_with_local_key_file_when_requested() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let app_db = temp_dir.path().join("app.db");
    fs::write(temp_dir.path().join(".key"), ENCRYPTED_KEY_FILE).expect("write key file");
    let db = create_app_db(&app_db);
    insert_connection(
        &db,
        Row {
            id: 10,
            name: "Encrypted MySQL",
            connection_type: "mysql",
            host: Some("127.0.0.1"),
            port: Some(3307),
            username: Some("root"),
            default_database: Some("mysql"),
            password: Some(ENCRYPTED_PASSWORD),
            remember_password: true,
        },
    );

    let imported = preview_connections_from_path(
        ImportSourceKind::BeekeeperStudio,
        &app_db,
        ImportOptions {
            include_passwords: true,
        },
    )
    .expect("encrypted Beekeeper app db should preview");

    assert_eq!(Some("secret-password".to_string()), imported[0].password);
    assert_eq!(PasswordImportStatus::Included, imported[0].password_status);
}

#[test]
fn beekeeper_import_marks_requested_password_missing_without_key_file() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let app_db = temp_dir.path().join("app.db");
    let db = create_app_db(&app_db);
    insert_connection(
        &db,
        Row {
            id: 11,
            name: "Missing Key",
            connection_type: "postgres",
            host: Some("localhost"),
            port: None,
            username: Some("postgres"),
            default_database: Some("postgres"),
            password: Some(ENCRYPTED_PASSWORD),
            remember_password: true,
        },
    );

    let imported = preview_connections_from_path(
        ImportSourceKind::BeekeeperStudio,
        &app_db,
        ImportOptions {
            include_passwords: true,
        },
    )
    .expect("Beekeeper app db should preview without key file");

    assert_eq!(None, imported[0].password);
    assert_eq!(PasswordImportStatus::Missing, imported[0].password_status);
}

struct Row<'a> {
    id: i64,
    name: &'a str,
    connection_type: &'a str,
    host: Option<&'a str>,
    port: Option<u16>,
    username: Option<&'a str>,
    default_database: Option<&'a str>,
    password: Option<&'a str>,
    remember_password: bool,
}

fn create_app_db(path: &Path) -> Connection {
    let db = Connection::open(path).expect("open sqlite db");
    db.execute_batch(
        r#"
        CREATE TABLE saved_connection (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            connectionType TEXT,
            host TEXT,
            port INTEGER,
            username TEXT,
            defaultDatabase TEXT,
            password TEXT,
            rememberPassword BOOLEAN NOT NULL
        );
        "#,
    )
    .expect("create saved_connection table");
    db
}

fn insert_connection(db: &Connection, row: Row<'_>) {
    db.execute(
        r#"
        INSERT INTO saved_connection (
            id, name, connectionType, host, port, username, defaultDatabase,
            password, rememberPassword
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        params![
            row.id,
            row.name,
            row.connection_type,
            row.host,
            row.port,
            row.username,
            row.default_database,
            row.password,
            row.remember_password,
        ],
    )
    .expect("insert saved connection");
}
