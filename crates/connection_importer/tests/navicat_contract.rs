use connection_importer::{
    ImportOptions, ImportSourceKind, PasswordImportStatus, parse_navicat_connections_plist,
    parse_navicat_connections_xml, preview_connections_from_path,
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

const NAVICAT_CONN_PLIST_SAMPLE: &[u8] = br#"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>0</key>
  <dict>
    <key>0</key>
    <dict>
      <key>MySQL</key>
      <dict>
        <key>10.2.4.55</key>
        <dict>
          <key>host</key>
          <string>10.2.4.55</string>
          <key>port</key>
          <string>3306</string>
          <key>username</key>
          <string>root</string>
          <key>database</key>
          <string>app</string>
        </dict>
      </dict>
    </dict>
  </dict>
</dict>
</plist>
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

#[test]
fn navicat_plist_parser_reads_conn_plist_connections() {
    let imported = parse_navicat_connections_plist(
        NAVICAT_CONN_PLIST_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("Navicat conn.plist should parse");

    assert_navicat_conn_plist_connection(&imported[0]);
}

#[test]
fn navicat_preview_reads_conn_plist_file_from_path() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let export = temp_dir.path().join("conn.plist");
    fs::write(&export, NAVICAT_CONN_PLIST_SAMPLE).expect("write Navicat conn.plist");

    let imported = preview_connections_from_path(
        ImportSourceKind::Navicat,
        &export,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("Navicat conn.plist should preview");

    assert_navicat_conn_plist_connection(&imported[0]);
}

fn assert_navicat_conn_plist_connection(mysql: &connection_importer::ImportedConnection) {
    assert_eq!(ImportSourceKind::Navicat, mysql.source);
    assert_eq!("0/0/MySQL/10.2.4.55", mysql.source_id);
    assert_eq!("10.2.4.55", mysql.name);
    assert_eq!(DatabaseType::MySQL, mysql.database_type);
    assert_eq!("10.2.4.55", mysql.host);
    assert_eq!(Some(3306), mysql.port);
    assert_eq!("root", mysql.username);
    assert_eq!(Some("app".to_string()), mysql.database);
    assert_eq!(None, mysql.password);
    assert_eq!(PasswordImportStatus::Unsupported, mysql.password_status);
}
