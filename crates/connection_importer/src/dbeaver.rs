use crate::{
    ImportError, ImportOptions, ImportSourceKind, ImportedConnection, PasswordImportStatus,
    SourceAvailability,
};
use one_core::storage::DatabaseType;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn detect_availability() -> SourceAvailability {
    let Some(path) = default_data_sources_path() else {
        return SourceAvailability::NotInstalled;
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return SourceAvailability::PermissionRequired;
    };
    match parse_dbeaver_data_sources_json(
        &contents,
        ImportOptions {
            include_passwords: false,
        },
    ) {
        Ok(connections) if connections.is_empty() => SourceAvailability::NoConnections,
        Ok(connections) => SourceAvailability::Available {
            connection_count: connections.len(),
        },
        Err(error) => SourceAvailability::Error {
            message: error.to_string(),
        },
    }
}

pub fn preview_default_connections(
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let path = default_data_sources_path()
        .ok_or_else(|| ImportError::SourceDataNotFound("DBeaver data-sources.json".to_string()))?;
    preview_connections_from_path(path, options)
}

pub fn preview_connections_from_path(
    path: impl AsRef<Path>,
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let contents = std::fs::read_to_string(path.as_ref())
        .map_err(|error| ImportError::ReadSourceData(error.to_string()))?;
    parse_dbeaver_data_sources_json(&contents, options)
}

pub fn parse_dbeaver_data_sources_json(
    contents: &str,
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let root: Value = serde_json::from_str(contents)
        .map_err(|error| ImportError::InvalidSourceData(error.to_string()))?;
    let connections = root
        .get("connections")
        .and_then(Value::as_object)
        .ok_or(ImportError::MissingField("connections"))?;

    connections
        .iter()
        .map(|(source_id, value)| parse_connection(source_id, value, options))
        .collect()
}

fn parse_connection(
    source_id: &str,
    value: &Value,
    options: ImportOptions,
) -> Result<ImportedConnection, ImportError> {
    let configuration = value.get("configuration").unwrap_or(value);
    let database_type = database_type(value)?;
    Ok(ImportedConnection {
        source: ImportSourceKind::DBeaver,
        source_id: source_id.to_string(),
        name: string_at(value, "name").unwrap_or(source_id).to_string(),
        database_type: database_type.clone(),
        host: required_string(configuration, "host")?.to_string(),
        port: port(configuration).or_else(|| default_port(&database_type)),
        username: string_at(value, "user").unwrap_or_default().to_string(),
        password: None,
        database: string_at(configuration, "database").map(str::to_string),
        extra_params: HashMap::new(),
        password_status: password_status(options),
    })
}

fn database_type(value: &Value) -> Result<DatabaseType, ImportError> {
    let raw = string_at(value, "provider")
        .or_else(|| string_at(value, "driver"))
        .unwrap_or_default()
        .to_lowercase();
    match raw.as_str() {
        value if value.contains("mysql") || value.contains("maria") => Ok(DatabaseType::MySQL),
        value if value.contains("postgres") => Ok(DatabaseType::PostgreSQL),
        value if value.contains("sqlite") => Ok(DatabaseType::SQLite),
        value if value.contains("duckdb") => Ok(DatabaseType::DuckDB),
        value if value.contains("sqlserver") || value.contains("mssql") => Ok(DatabaseType::MSSQL),
        value if value.contains("oracle") => Ok(DatabaseType::Oracle),
        value if value.contains("clickhouse") => Ok(DatabaseType::ClickHouse),
        _ => Err(ImportError::UnsupportedDatabaseType(raw)),
    }
}

fn default_data_sources_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    [
        home.join("Library/DBeaverData/workspace6/General/.dbeaver/data-sources.json"),
        home.join(".local/share/DBeaverData/workspace6/General/.dbeaver/data-sources.json"),
        home.join(".dbeaver4/General/.dbeaver/data-sources.json"),
    ]
    .into_iter()
    .find(|path| path.exists())
}

fn required_string<'a>(value: &'a Value, field: &'static str) -> Result<&'a str, ImportError> {
    string_at(value, field).ok_or(ImportError::MissingField(field))
}

fn string_at<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

fn port(value: &Value) -> Option<u16> {
    value
        .get("port")
        .and_then(|port| {
            port.as_str()
                .map(str::to_string)
                .or_else(|| port.as_u64().map(|v| v.to_string()))
        })
        .and_then(|port| port.parse().ok())
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

fn password_status(options: ImportOptions) -> PasswordImportStatus {
    let _ = options;
    PasswordImportStatus::Unsupported
}
