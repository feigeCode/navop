use connection_importer::{
    ImportOptions, ImportSourceKind, PasswordImportStatus, parse_heidisql_settings_ini,
    preview_connections_from_path,
};
use one_core::storage::DatabaseType;
use std::fs;

const HEIDISQL_SAMPLE: &str = r#"
[Servers\Prod MySQL]
Host=db.example.com
User=app_user
Port=3307
Database=app
NetType=0

[Servers\Local PG]
Host=localhost
User=postgres
Database=postgres
Driver=PostgreSQL
"#;

#[test]
fn heidisql_parser_reads_server_sections() {
    let imported = parse_heidisql_settings_ini(
        HEIDISQL_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("HeidiSQL settings should parse");

    assert_eq!(2, imported.len());
    let mysql = &imported[0];
    assert_eq!(ImportSourceKind::HeidiSQL, mysql.source);
    assert_eq!("Prod MySQL", mysql.source_id);
    assert_eq!("Prod MySQL", mysql.name);
    assert_eq!(DatabaseType::MySQL, mysql.database_type);
    assert_eq!("db.example.com", mysql.host);
    assert_eq!(Some(3307), mysql.port);
    assert_eq!("app_user", mysql.username);
    assert_eq!(Some("app".to_string()), mysql.database);
    assert_eq!(None, mysql.password);
    assert_eq!(PasswordImportStatus::Unsupported, mysql.password_status);
}

#[test]
fn heidisql_parser_detects_postgresql_and_default_port() {
    let imported = parse_heidisql_settings_ini(
        HEIDISQL_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("HeidiSQL settings should parse");

    let pg = &imported[1];
    assert_eq!(DatabaseType::PostgreSQL, pg.database_type);
    assert_eq!("localhost", pg.host);
    assert_eq!(Some(5432), pg.port);
    assert_eq!("postgres", pg.username);
    assert_eq!(Some("postgres".to_string()), pg.database);
}

#[test]
fn heidisql_preview_reads_settings_file_from_path() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let settings = temp_dir.path().join("portable_settings.txt");
    fs::write(&settings, HEIDISQL_SAMPLE).expect("write HeidiSQL settings");

    let imported = preview_connections_from_path(
        ImportSourceKind::HeidiSQL,
        &settings,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("HeidiSQL file should preview");

    assert_eq!(2, imported.len());
    assert_eq!("Prod MySQL", imported[0].name);
}
