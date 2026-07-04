use db::ipc::{
    IpcDriverRegistry,
    protocol::{method, schema_users_wire_sql},
};
use one_core::storage::{DatabaseType, DbConnectionConfig};

pub(crate) fn user_listing_sql(database_type: &DatabaseType) -> Option<&'static str> {
    match database_type {
        DatabaseType::MySQL => Some(MYSQL_USER_LISTING_SQL),
        DatabaseType::PostgreSQL => Some(POSTGRES_USER_LISTING_SQL),
        DatabaseType::MSSQL => Some(MSSQL_USER_LISTING_SQL),
        DatabaseType::Oracle => Some(ORACLE_USER_LISTING_SQL),
        DatabaseType::ClickHouse => Some(CLICKHOUSE_USER_LISTING_SQL),
        DatabaseType::SQLite | DatabaseType::DuckDB | DatabaseType::External { .. } => None,
    }
}

pub(crate) fn user_listing_sql_for_config(
    config: &DbConnectionConfig,
    registry: &IpcDriverRegistry,
) -> Option<String> {
    match &config.database_type {
        DatabaseType::External { .. } => external_driver_user_listing_sql(config, registry),
        database_type => user_listing_sql(database_type).map(str::to_string),
    }
}

fn external_driver_user_listing_sql(
    config: &DbConnectionConfig,
    registry: &IpcDriverRegistry,
) -> Option<String> {
    let driver_id = config.database_type.external_driver_id()?;
    let driver = registry.find(driver_id)?;
    let fallback = driver
        .dialect
        .compatible_database_type
        .as_ref()
        .and_then(user_listing_sql);
    let declares_methods = !driver.methods.is_empty();
    let supports_users_method = driver
        .methods
        .iter()
        .any(|driver_method| driver_method == method::SCHEMA_USERS);

    if supports_users_method || !declares_methods {
        return Some(schema_users_wire_sql(fallback));
    }

    fallback.map(str::to_string)
}

const MYSQL_USER_LISTING_SQL: &str = r#"SELECT
  User,
  Host,
  account_locked AS account_locked,
  password_expired AS password_expired,
  plugin AS authentication_plugin
FROM mysql.user
ORDER BY User, Host;"#;

const POSTGRES_USER_LISTING_SQL: &str = r#"SELECT
  rolname,
  rolcanlogin,
  rolsuper,
  rolcreatedb,
  rolcreaterole,
  rolreplication,
  rolvaliduntil
FROM pg_catalog.pg_roles
ORDER BY rolname;"#;

const MSSQL_USER_LISTING_SQL: &str = r#"SELECT
  name,
  type_desc,
  authentication_type_desc,
  default_schema_name,
  create_date,
  modify_date
FROM sys.database_principals
WHERE type IN ('S', 'U', 'G', 'R')
  AND name NOT LIKE '##%'
ORDER BY name;"#;

const ORACLE_USER_LISTING_SQL: &str = r#"SELECT
  username,
  user_id,
  created
FROM all_users
ORDER BY username;"#;

const CLICKHOUSE_USER_LISTING_SQL: &str = r#"SELECT
  name,
  storage,
  auth_type
FROM system.users
ORDER BY name;"#;

#[cfg(test)]
mod tests {
    use super::*;
    use db::ipc::{IpcDriverManifest, IpcDriverRegistry};
    use one_core::storage::DbConnectionConfig;

    #[test]
    fn mysql_user_listing_sql_reads_mysql_user_table() {
        let sql = user_listing_sql(&DatabaseType::MySQL).expect("MySQL should be supported");

        assert!(sql.contains("mysql.user"));
        assert!(sql.contains("User"));
        assert!(sql.contains("Host"));
    }

    #[test]
    fn postgres_user_listing_sql_reads_roles() {
        let sql =
            user_listing_sql(&DatabaseType::PostgreSQL).expect("PostgreSQL should be supported");

        assert!(sql.contains("pg_catalog.pg_roles"));
        assert!(sql.contains("rolname"));
        assert!(sql.contains("rolcanlogin"));
    }

    #[test]
    fn sqlite_user_listing_sql_is_not_supported() {
        assert!(user_listing_sql(&DatabaseType::SQLite).is_none());
    }

    #[test]
    fn external_driver_user_listing_sql_uses_schema_users_method() {
        let registry =
            IpcDriverRegistry::from_drivers(vec![driver_manifest(r#""methods":["schema/users"]"#)]);
        let config = external_config("demo");

        let sql = user_listing_sql_for_config(&config, &registry)
            .expect("schema/users method should be used");
        let envelope: serde_json::Value =
            serde_json::from_str(sql.strip_prefix(db::ipc::protocol::WIRE_PREFIX).unwrap())
                .expect("wire SQL should contain JSON envelope");

        assert_eq!(Some("schema/users"), envelope["method"].as_str());
    }

    #[test]
    fn external_driver_user_listing_sql_falls_back_when_method_declares_unsupported() {
        let registry = IpcDriverRegistry::from_drivers(vec![driver_manifest(
            r#""methods":["schema/databases"],"dialect":{"compatible_database_type":"PostgreSQL"}"#,
        )]);
        let config = external_config("demo");

        let sql = user_listing_sql_for_config(&config, &registry)
            .expect("compatible PostgreSQL SQL should be used as fallback");

        assert!(sql.contains("pg_catalog.pg_roles"));
        assert!(
            !sql.starts_with(db::ipc::protocol::WIRE_PREFIX),
            "declared method sets should not call unsupported schema/users"
        );
    }

    #[test]
    fn external_driver_user_listing_sql_uses_method_with_compatible_fallback_for_legacy_driver() {
        let registry = IpcDriverRegistry::from_drivers(vec![driver_manifest(
            r#""dialect":{"compatible_database_type":"PostgreSQL"}"#,
        )]);
        let config = external_config("demo");

        let sql = user_listing_sql_for_config(&config, &registry)
            .expect("legacy drivers should receive schema/users with fallback SQL");
        let envelope: serde_json::Value =
            serde_json::from_str(sql.strip_prefix(db::ipc::protocol::WIRE_PREFIX).unwrap())
                .expect("wire SQL should contain JSON envelope");

        assert_eq!(Some("schema/users"), envelope["method"].as_str());
        assert!(
            envelope["params"]["fallback_sql"]
                .as_str()
                .is_some_and(|sql| sql.contains("pg_catalog.pg_roles")),
            "fallback SQL should be available to external drivers"
        );
    }

    fn external_config(driver_id: &str) -> DbConnectionConfig {
        DbConnectionConfig {
            id: "1".to_string(),
            database_type: DatabaseType::external(driver_id),
            name: "External".to_string(),
            host: "localhost".to_string(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            extra_params: Default::default(),
        }
    }

    fn driver_manifest(extra_json: &str) -> IpcDriverManifest {
        serde_json::from_str(&format!(
            r#"{{
                "id":"demo",
                "name":"Demo",
                "entry":{{"command":"./driver"}},
                "transport":{{"name":"demo.sock"}},
                {extra_json}
            }}"#
        ))
        .expect("driver manifest should parse")
    }
}
