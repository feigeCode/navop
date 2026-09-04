use one_core::storage::DatabaseType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DatabaseFamily {
    MySql,
    PostgreSql,
    SqlServer,
    Oracle,
    ClickHouse,
    Sqlite,
    DuckDb,
    Other,
}

pub(crate) fn database_family(database_type: &DatabaseType) -> DatabaseFamily {
    match database_type {
        DatabaseType::MySQL => DatabaseFamily::MySql,
        // TDengine 类型系统按 MySQL 方言同臂处理。
        DatabaseType::TDengine => DatabaseFamily::MySql,
        DatabaseType::PostgreSQL => DatabaseFamily::PostgreSql,
        DatabaseType::MSSQL => DatabaseFamily::SqlServer,
        DatabaseType::Oracle => DatabaseFamily::Oracle,
        DatabaseType::ClickHouse => DatabaseFamily::ClickHouse,
        DatabaseType::SQLite => DatabaseFamily::Sqlite,
        DatabaseType::DuckDB => DatabaseFamily::DuckDb,
        DatabaseType::External { driver_id } => external_database_family(driver_id),
    }
}

fn external_database_family(driver_id: &str) -> DatabaseFamily {
    match driver_id.trim().to_ascii_lowercase().as_str() {
        "mysql" | "mariadb" | "oceanbase" => DatabaseFamily::MySql,
        "postgres" | "postgresql" | "kingbase" | "opengauss" => DatabaseFamily::PostgreSql,
        "mssql" | "sqlserver" | "sql server" => DatabaseFamily::SqlServer,
        "oracle" | "dm" | "dameng" | "oracle-go" => DatabaseFamily::Oracle,
        "clickhouse" => DatabaseFamily::ClickHouse,
        "sqlite" => DatabaseFamily::Sqlite,
        "duckdb" => DatabaseFamily::DuckDb,
        _ => DatabaseFamily::Other,
    }
}
