use crate::{
    CredentialQuery, CredentialStore, ImportError, ImportOptions, ImportSourceKind,
    ImportedConnection, PasswordImportStatus, SourceAvailability,
};
use one_core::storage::DatabaseType;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn detect_availability() -> SourceAvailability {
    let Some(path) = default_connections_path() else {
        return SourceAvailability::NotInstalled;
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return SourceAvailability::PermissionRequired;
    };
    match parse_tableplus_connections_json_with_credentials(
        &contents,
        ImportOptions {
            include_passwords: false,
        },
        &crate::NoopCredentialStore,
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
    credentials: &dyn CredentialStore,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let path = default_connections_path()
        .ok_or_else(|| ImportError::SourceDataNotFound("TablePlus connections file".to_string()))?;
    preview_connections_from_path(path, options, credentials)
}

pub fn preview_connections_from_path(
    path: impl AsRef<Path>,
    options: ImportOptions,
    credentials: &dyn CredentialStore,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let contents = std::fs::read_to_string(path.as_ref())
        .map_err(|error| ImportError::ReadSourceData(error.to_string()))?;
    parse_tableplus_connections_json_with_credentials(&contents, options, credentials)
}

pub fn parse_tableplus_connections_json_with_credentials(
    contents: &str,
    options: ImportOptions,
    credentials: &dyn CredentialStore,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let root: Value = serde_json::from_str(contents)
        .map_err(|error| ImportError::InvalidSourceData(error.to_string()))?;
    let items = connection_items(&root)?;
    items
        .iter()
        .map(|value| parse_connection(value, options, credentials))
        .collect()
}

fn connection_items(root: &Value) -> Result<&Vec<Value>, ImportError> {
    root.as_array()
        .or_else(|| root.get("connections").and_then(Value::as_array))
        .or_else(|| root.get("items").and_then(Value::as_array))
        .or_else(|| root.get("data").and_then(Value::as_array))
        .ok_or(ImportError::MissingField("connections"))
}

fn parse_connection(
    value: &Value,
    options: ImportOptions,
    credentials: &dyn CredentialStore,
) -> Result<ImportedConnection, ImportError> {
    let database_type = database_type(value)?;
    let source_id = string_any(value, &["id", "uuid", "objectID"]).unwrap_or("tableplus");
    let username = string_any(value, &["user", "username"]).unwrap_or_default();
    let port = port(value).or_else(|| default_port(&database_type));
    let password = password(value, source_id, username, port, options, credentials);

    Ok(ImportedConnection {
        source: ImportSourceKind::TablePlus,
        source_id: source_id.to_string(),
        name: string_any(value, &["name", "title"])
            .unwrap_or(source_id)
            .to_string(),
        database_type,
        host: required_string_any(value, &["host", "server", "hostname"])?.to_string(),
        port,
        username: username.to_string(),
        password: password.value,
        database: string_any(value, &["database", "dbname", "schema"]).map(str::to_string),
        extra_params: HashMap::new(),
        password_status: password.status,
    })
}

struct PasswordLookup {
    value: Option<String>,
    status: PasswordImportStatus,
}

fn password(
    value: &Value,
    source_id: &str,
    username: &str,
    port: Option<u16>,
    options: ImportOptions,
    credentials: &dyn CredentialStore,
) -> PasswordLookup {
    if !options.include_passwords {
        return PasswordLookup {
            value: None,
            status: PasswordImportStatus::Unsupported,
        };
    }

    for query in credential_queries(value, source_id, username, port) {
        if let Some(password) = credentials.get_password(&query) {
            return PasswordLookup {
                value: Some(password),
                status: PasswordImportStatus::Included,
            };
        }
    }

    PasswordLookup {
        value: None,
        status: PasswordImportStatus::Missing,
    }
}

fn credential_queries(
    value: &Value,
    source_id: &str,
    username: &str,
    port: Option<u16>,
) -> Vec<CredentialQuery> {
    let service =
        string_any(value, &["keychain_service", "keychainService"]).unwrap_or("TablePlus");
    let mut accounts = Vec::new();
    if let Some(account) = string_any(value, &["keychain_account", "keychainAccount"]) {
        accounts.push(account.to_string());
    }
    accounts.push(source_id.to_string());
    if let Some(host) = string_any(value, &["host", "server", "hostname"]) {
        accounts.push(format!(
            "{}@{}:{}",
            username,
            host,
            port.unwrap_or_default()
        ));
        accounts.push(format!("{}@{}", username, host));
    }
    accounts
        .into_iter()
        .map(|account| CredentialQuery::new(service, account))
        .collect()
}

fn database_type(value: &Value) -> Result<DatabaseType, ImportError> {
    let raw = string_any(value, &["driver", "type", "databaseType"])
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

fn default_connections_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    [
        home.join("Library/Application Support/com.tinyapp.TablePlus/Data/Connections.json"),
        home.join(".config/TablePlus/Connections.json"),
        home.join("AppData/Roaming/TablePlus/Connections.json"),
    ]
    .into_iter()
    .find(|path| path.exists())
}

fn required_string_any<'a>(
    value: &'a Value,
    fields: &[&'static str],
) -> Result<&'a str, ImportError> {
    string_any(value, fields).ok_or(ImportError::MissingField(fields[0]))
}

fn string_any<'a>(value: &'a Value, fields: &[&str]) -> Option<&'a str> {
    fields
        .iter()
        .find_map(|field| value.get(*field).and_then(Value::as_str))
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
