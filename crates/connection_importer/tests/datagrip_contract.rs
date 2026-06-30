use connection_importer::{
    ImportOptions, ImportSourceKind, PasswordImportStatus, parse_datagrip_data_sources_xml,
    preview_connections_from_path,
};
use one_core::storage::DatabaseType;
use std::fs;

const DATAGRIP_SAMPLE: &str = r#"
<project version="4">
  <component name="DataSourceManagerImpl" format="xml">
    <data-source source="LOCAL" name="Prod MySQL" uuid="mysql-prod">
      <driver-ref>mysql.8</driver-ref>
      <jdbc-url>jdbc:mysql://db.example.com:3307/app</jdbc-url>
      <user-name>app_user</user-name>
    </data-source>
    <data-source source="LOCAL" name="Local PG" uuid="pg-local">
      <driver-ref>postgresql</driver-ref>
      <jdbc-url>jdbc:postgresql://localhost/postgres</jdbc-url>
      <properties>
        <property name="user" value="postgres" />
      </properties>
    </data-source>
  </component>
</project>
"#;

const DATAGRIP_MSSQL_SAMPLE: &str = r#"
<project version="4">
  <component name="DataSourceManagerImpl" format="xml">
    <data-source source="LOCAL" name="Warehouse" uuid="mssql-prod">
      <driver-ref>sqlserver.ms</driver-ref>
      <jdbc-url>jdbc:sqlserver://mssql.example.com:1434;databaseName=warehouse</jdbc-url>
      <user-name>etl</user-name>
    </data-source>
  </component>
</project>
"#;

#[test]
fn datagrip_parser_reads_jdbc_connection_fields() {
    let imported = parse_datagrip_data_sources_xml(
        DATAGRIP_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("DataGrip XML should parse");

    assert_eq!(2, imported.len());
    let mysql = &imported[0];
    assert_eq!(ImportSourceKind::DataGrip, mysql.source);
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
fn datagrip_parser_uses_default_port_and_property_username() {
    let imported = parse_datagrip_data_sources_xml(
        DATAGRIP_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("DataGrip XML should parse");

    let pg = &imported[1];
    assert_eq!(DatabaseType::PostgreSQL, pg.database_type);
    assert_eq!("localhost", pg.host);
    assert_eq!(Some(5432), pg.port);
    assert_eq!("postgres", pg.username);
    assert_eq!(Some("postgres".to_string()), pg.database);
}

#[test]
fn preview_connections_reads_datagrip_xml_from_path() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let data_sources = temp_dir.path().join("dataSources.xml");
    fs::write(&data_sources, DATAGRIP_SAMPLE).expect("write data sources");

    let imported = preview_connections_from_path(
        ImportSourceKind::DataGrip,
        &data_sources,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("DataGrip file should preview");

    assert_eq!(2, imported.len());
    assert_eq!("Prod MySQL", imported[0].name);
    assert_eq!("Local PG", imported[1].name);
}

#[test]
fn datagrip_parser_reads_sqlserver_database_name_property() {
    let imported = parse_datagrip_data_sources_xml(
        DATAGRIP_MSSQL_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("DataGrip XML should parse");

    let mssql = &imported[0];
    assert_eq!(DatabaseType::MSSQL, mssql.database_type);
    assert_eq!("mssql.example.com", mssql.host);
    assert_eq!(Some(1434), mssql.port);
    assert_eq!(Some("warehouse".to_string()), mssql.database);
    assert_eq!("etl", mssql.username);
}
