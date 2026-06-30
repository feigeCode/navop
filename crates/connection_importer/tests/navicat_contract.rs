use connection_importer::{
    ImportOptions, ImportSourceKind, PasswordImportStatus, parse_navicat_connections_xml,
    preview_connections_from_path,
};
use one_core::storage::DatabaseType;
use std::fs;

const NAVICAT_SAMPLE: &str = r#"
<Connections>
  <Connection
    ConnectionName="Navicat MySQL"
    ConnType="MYSQL"
    Host="navicat.example.com"
    Port="3307"
    UserName="app_user"
    Database="app" />
  <Connection
    Name="Navicat PG"
    Type="POSTGRESQL"
    Host="localhost"
    User="postgres"
    Database="postgres" />
</Connections>
"#;

#[test]
fn navicat_parser_reads_exported_connection_xml() {
    let imported = parse_navicat_connections_xml(
        NAVICAT_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("Navicat XML should parse");

    assert_eq!(2, imported.len());
    let mysql = &imported[0];
    assert_eq!(ImportSourceKind::Navicat, mysql.source);
    assert_eq!("Navicat MySQL", mysql.source_id);
    assert_eq!("Navicat MySQL", mysql.name);
    assert_eq!(DatabaseType::MySQL, mysql.database_type);
    assert_eq!("navicat.example.com", mysql.host);
    assert_eq!(Some(3307), mysql.port);
    assert_eq!("app_user", mysql.username);
    assert_eq!(Some("app".to_string()), mysql.database);
    assert_eq!(None, mysql.password);
    assert_eq!(PasswordImportStatus::Unsupported, mysql.password_status);
}

#[test]
fn navicat_parser_uses_default_port_and_alternate_names() {
    let imported = parse_navicat_connections_xml(
        NAVICAT_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("Navicat XML should parse");

    let pg = &imported[1];
    assert_eq!(DatabaseType::PostgreSQL, pg.database_type);
    assert_eq!("localhost", pg.host);
    assert_eq!(Some(5432), pg.port);
    assert_eq!("postgres", pg.username);
    assert_eq!(Some("postgres".to_string()), pg.database);
}

#[test]
fn navicat_preview_reads_xml_file_from_path() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let export = temp_dir.path().join("connections.ncx");
    fs::write(&export, NAVICAT_SAMPLE).expect("write Navicat export");

    let imported = preview_connections_from_path(
        ImportSourceKind::Navicat,
        &export,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("Navicat XML should preview");

    assert_eq!(2, imported.len());
    assert_eq!("Navicat MySQL", imported[0].name);
}
