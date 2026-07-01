use crate::{
    ImportError, ImportOptions, ImportSourceKind, ImportedConnection, PasswordImportStatus,
};
use one_core::storage::DatabaseType;
use plist::{Dictionary, Value};
use std::collections::HashMap;
use std::io::Cursor;

pub fn parse_navicat_connections_plist(
    contents: &[u8],
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let root = Value::from_reader(Cursor::new(contents))
        .map_err(|error| ImportError::InvalidSourceData(error.to_string()))?;
    let Some(dictionary) = root.as_dictionary() else {
        return Err(ImportError::MissingField("root"));
    };
    let mut connections = Vec::new();
    collect_connections(dictionary, &mut Vec::new(), None, options, &mut connections)?;
    Ok(connections)
}

fn collect_connections(
    dictionary: &Dictionary,
    path: &mut Vec<String>,
    database_type: Option<DatabaseType>,
    options: ImportOptions,
    connections: &mut Vec<ImportedConnection>,
) -> Result<(), ImportError> {
    if let Some(database_type) = database_type.clone()
        && let Some(connection) = parse_connection(dictionary, path, database_type, options)
    {
        connections.push(connection?);
        return Ok(());
    }
    for (key, value) in dictionary {
        let Some(child) = value.as_dictionary() else {
            continue;
        };
        path.push(key.clone());
        let next_type = database_type
            .clone()
            .or_else(|| database_type_from_key(key));
        collect_connections(child, path, next_type, options, connections)?;
        path.pop();
    }
    Ok(())
}

fn parse_connection(
    dictionary: &Dictionary,
    path: &[String],
    database_type: DatabaseType,
    options: ImportOptions,
) -> Option<Result<ImportedConnection, ImportError>> {
    let host = string_value(dictionary, "host")?;
    let name = path.last().cloned().unwrap_or_else(|| host.clone());
    Some(Ok(ImportedConnection {
        source: ImportSourceKind::Navicat,
        source_id: source_id(dictionary, path),
        name,
        database_type: database_type.clone(),
        host,
        port: port(dictionary).or_else(|| default_port(&database_type)),
        username: string_value(dictionary, "username").unwrap_or_default(),
        password: None,
        database: database_name(dictionary),
        extra_params: HashMap::new(),
        password_status: password_status(options),
    }))
}

fn source_id(dictionary: &Dictionary, path: &[String]) -> String {
    string_value(dictionary, "connection_uuid")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| path.join("/"))
}

fn database_name(dictionary: &Dictionary) -> Option<String> {
    ["database", "dbname", "schema"]
        .iter()
        .find_map(|key| string_value(dictionary, key))
        .filter(|value| !value.is_empty())
}

fn string_value(dictionary: &Dictionary, key: &str) -> Option<String> {
    dictionary.get(key).and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Integer(value) => Some(value.to_string()),
        _ => None,
    })
}

fn port(dictionary: &Dictionary) -> Option<u16> {
    string_value(dictionary, "port").and_then(|value| value.parse().ok())
}

fn database_type_from_key(key: &str) -> Option<DatabaseType> {
    let normalized = key.to_ascii_lowercase();
    match normalized.as_str() {
        value if value.contains("mysql") || value.contains("maria") => Some(DatabaseType::MySQL),
        value if value.contains("postgre") || value.contains("pgsql") => {
            Some(DatabaseType::PostgreSQL)
        }
        value if value.contains("mssql") || value.contains("sqlserver") => {
            Some(DatabaseType::MSSQL)
        }
        value if value.contains("ora") || value.contains("oracle") => Some(DatabaseType::Oracle),
        value if value.contains("sqlite") => Some(DatabaseType::SQLite),
        value if value.contains("clickhouse") => Some(DatabaseType::ClickHouse),
        _ => None,
    }
}

fn default_port(database_type: &DatabaseType) -> Option<u16> {
    match database_type {
        DatabaseType::MySQL => Some(3306),
        DatabaseType::PostgreSQL => Some(5432),
        DatabaseType::MSSQL => Some(1433),
        DatabaseType::Oracle => Some(1521),
        DatabaseType::ClickHouse => Some(8123),
        DatabaseType::SQLite | DatabaseType::DuckDB | DatabaseType::External { .. } => None,
    }
}

fn password_status(_options: ImportOptions) -> PasswordImportStatus {
    PasswordImportStatus::Unsupported
}
