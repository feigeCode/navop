use connection_importer::{
    ImportOptions, ImportSourceKind, PasswordImportStatus, SourceAvailability,
    duplicate_fingerprint, list_sources, parse_dbeaver_data_sources_json,
    preview_connections_from_path, to_db_connection_config,
};
use one_core::storage::DatabaseType;
use std::fs;

const DBEAVER_SAMPLE: &str = r#"
{
  "connections": {
    "mysql-prod": {
      "provider": "mysql",
      "driver": "mysql8",
      "name": "Prod MySQL",
      "configuration": {
        "host": "db.example.com",
        "port": "3307",
        "database": "app"
      },
      "user": "app_user"
    },
    "pg-local": {
      "provider": "postgresql",
      "driver": "postgresql",
      "name": "Local PG",
      "configuration": {
        "host": "localhost",
        "database": "postgres"
      },
      "user": "postgres"
    }
  }
}
"#;

#[test]
fn dbeaver_parser_reads_connection_fields_and_database_type() {
    let imported = parse_dbeaver_data_sources_json(
        DBEAVER_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("sample should parse");

    assert_eq!(2, imported.len());
    let mysql = &imported[0];
    assert_eq!(ImportSourceKind::DBeaver, mysql.source);
    assert_eq!("mysql-prod", mysql.source_id);
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
fn dbeaver_parser_uses_database_default_port_when_missing() {
    let imported = parse_dbeaver_data_sources_json(
        DBEAVER_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("sample should parse");

    let pg = &imported[1];
    assert_eq!(DatabaseType::PostgreSQL, pg.database_type);
    assert_eq!(Some(5432), pg.port);
}

#[test]
fn conversion_builds_db_connection_config() {
    let imported = parse_dbeaver_data_sources_json(
        DBEAVER_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("sample should parse")
    .remove(0);

    let config = to_db_connection_config(imported).expect("connection should convert");

    assert_eq!("Prod MySQL", config.name);
    assert_eq!(DatabaseType::MySQL, config.database_type);
    assert_eq!("db.example.com", config.host);
    assert_eq!(3307, config.port);
    assert_eq!("app_user", config.username);
    assert_eq!("", config.password);
    assert_eq!(Some("app".to_string()), config.database);
}

#[test]
fn duplicate_fingerprint_uses_stable_connection_identity() {
    let mut imported = parse_dbeaver_data_sources_json(
        DBEAVER_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("sample should parse")
    .remove(0);

    let first = duplicate_fingerprint(&imported);
    imported.name = "Renamed".to_string();
    let renamed = duplicate_fingerprint(&imported);

    assert_eq!(first, renamed);
    assert!(first.contains("MySQL"));
    assert!(first.contains("db.example.com"));
}

#[test]
fn list_sources_exposes_dbeaver_and_reserved_sources() {
    let sources = list_sources();
    let source_names: Vec<_> = sources
        .iter()
        .map(|source| source.display_name.as_str())
        .collect();

    assert!(source_names.contains(&"DBeaver"));
    assert!(source_names.contains(&"TablePlus"));
    assert!(source_names.contains(&"Sequel Ace"));
    assert!(source_names.contains(&"Beekeeper Studio"));
    assert!(
        sources
            .iter()
            .any(|source| source.kind == ImportSourceKind::DBeaver
                && !matches!(source.availability, SourceAvailability::Unsupported))
    );
}

#[test]
fn preview_connections_reads_dbeaver_file_from_path() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let data_sources = temp_dir.path().join("data-sources.json");
    fs::write(&data_sources, DBEAVER_SAMPLE).expect("write data sources");

    let imported = preview_connections_from_path(
        ImportSourceKind::DBeaver,
        &data_sources,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("DBeaver file should preview");

    assert_eq!(2, imported.len());
    assert_eq!("Prod MySQL", imported[0].name);
    assert_eq!("Local PG", imported[1].name);
}

#[test]
fn preview_connections_rejects_reserved_sources() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let data_sources = temp_dir.path().join("data-sources.json");
    fs::write(&data_sources, DBEAVER_SAMPLE).expect("write data sources");

    let error = preview_connections_from_path(
        ImportSourceKind::TablePlus,
        &data_sources,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect_err("reserved source should not preview");

    assert!(error.to_string().contains("unsupported import source"));
}
