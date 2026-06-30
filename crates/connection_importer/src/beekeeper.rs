use crate::{
    ImportError, ImportOptions, ImportSourceKind, ImportedConnection, PasswordImportStatus,
    SourceAvailability, simple_encryptor,
};
use one_core::storage::DatabaseType;
use rusqlite::{Connection, OpenFlags, Row};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const DEFAULT_KEY: &str = "38782F413F442A472D4B6150645367566B59703373676339792442264E29482B";

pub fn detect_availability() -> SourceAvailability {
    let Some(path) = default_app_db_path() else {
        return SourceAvailability::NotInstalled;
    };
    match preview_connections_from_path(
        path,
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
    let path = default_app_db_path()
        .ok_or_else(|| ImportError::SourceDataNotFound("Beekeeper Studio app.db".to_string()))?;
    preview_connections_from_path(path, options)
}

pub fn preview_connections_from_path(
    path: impl AsRef<Path>,
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let path = path.as_ref();
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| ImportError::ReadSourceData(error.to_string()))?;
    let key = options
        .include_passwords
        .then(|| load_encryption_key(path))
        .flatten();
    read_saved_connections(&db, options, key.as_deref())
}

fn read_saved_connections(
    db: &Connection,
    options: ImportOptions,
    key: Option<&str>,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let mut stmt = db
        .prepare(
            r#"
        SELECT id, name, connectionType, host, port, username, defaultDatabase,
               password, rememberPassword
        FROM saved_connection
        ORDER BY id
        "#,
        )
        .map_err(|error| ImportError::InvalidSourceData(error.to_string()))?;
    let rows = stmt
        .query_map([], |row| parse_row(row, options, key))
        .map_err(|error| ImportError::InvalidSourceData(error.to_string()))?;
    collect_supported(rows)
}

fn collect_supported(
    rows: impl Iterator<Item = rusqlite::Result<Result<Option<ImportedConnection>, ImportError>>>,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let mut imported = Vec::new();
    for row in rows {
        match row.map_err(|error| ImportError::InvalidSourceData(error.to_string()))?? {
            Some(connection) => imported.push(connection),
            None => continue,
        }
    }
    Ok(imported)
}

fn parse_row(
    row: &Row<'_>,
    options: ImportOptions,
    key: Option<&str>,
) -> rusqlite::Result<Result<Option<ImportedConnection>, ImportError>> {
    let source_id = row.get::<_, i64>(0)?.to_string();
    let name = row.get::<_, String>(1)?;
    let raw_type = row.get::<_, Option<String>>(2)?.unwrap_or_default();
    let database_type = match database_type(&raw_type) {
        Ok(database_type) => database_type,
        Err(ImportError::UnsupportedDatabaseType(_)) => return Ok(Ok(None)),
        Err(error) => return Ok(Err(error)),
    };
    let host = match host(row.get(3)?, row.get(6)?, &database_type) {
        Ok(host) => host,
        Err(error) => return Ok(Err(error)),
    };
    let password = password(row.get(7)?, row.get(8)?, options, key);

    Ok(Ok(Some(ImportedConnection {
        source: ImportSourceKind::BeekeeperStudio,
        source_id,
        name,
        database_type: database_type.clone(),
        host,
        port: port(row.get(4)?).or_else(|| default_port(&database_type)),
        username: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        password: password.value,
        database: database(row.get(6)?, &database_type),
        extra_params: HashMap::new(),
        password_status: password.status,
    })))
}

struct PasswordLookup {
    value: Option<String>,
    status: PasswordImportStatus,
}

fn password(
    encrypted: Option<String>,
    remember_password: Option<bool>,
    options: ImportOptions,
    key: Option<&str>,
) -> PasswordLookup {
    if !options.include_passwords {
        return password_lookup(None, PasswordImportStatus::Unsupported);
    }
    let Some(value) = encrypted.filter(|value| !value.is_empty()) else {
        return password_lookup(None, PasswordImportStatus::Missing);
    };
    if !remember_password.unwrap_or(true) {
        return password_lookup(None, PasswordImportStatus::Missing);
    }
    if !simple_encryptor::looks_encrypted(&value) {
        return password_lookup(Some(value), PasswordImportStatus::Included);
    }
    key.and_then(|key| decrypt_string(key, &value))
        .map(|value| password_lookup(Some(value), PasswordImportStatus::Included))
        .unwrap_or_else(|| password_lookup(None, PasswordImportStatus::Missing))
}

fn password_lookup(value: Option<String>, status: PasswordImportStatus) -> PasswordLookup {
    PasswordLookup { value, status }
}

fn decrypt_string(key: &str, value: &str) -> Option<String> {
    simple_encryptor::decrypt_value(key, value)?
        .as_str()
        .map(str::to_string)
}

fn load_encryption_key(app_db_path: &Path) -> Option<String> {
    let key_path = app_db_path.parent()?.join(".key");
    let encrypted = std::fs::read_to_string(key_path).ok()?;
    let data = simple_encryptor::decrypt_value(DEFAULT_KEY, encrypted.trim())?;
    data.get("encryptionKey")?.as_str().map(str::to_string)
}

fn host(
    value: Option<String>,
    database: Option<String>,
    database_type: &DatabaseType,
) -> Result<String, ImportError> {
    if matches!(database_type, DatabaseType::SQLite | DatabaseType::DuckDB) {
        return database
            .filter(|value| !value.is_empty())
            .ok_or(ImportError::MissingField("defaultDatabase"));
    }
    Ok(value
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "localhost".to_string()))
}

fn database(value: Option<String>, database_type: &DatabaseType) -> Option<String> {
    if matches!(database_type, DatabaseType::SQLite | DatabaseType::DuckDB) {
        return None;
    }
    value.filter(|value| !value.is_empty())
}

fn database_type(raw: &str) -> Result<DatabaseType, ImportError> {
    match raw.to_lowercase().as_str() {
        "mysql" | "mariadb" => Ok(DatabaseType::MySQL),
        "postgresql" | "postgres" | "psql" | "redshift" | "greengage" => {
            Ok(DatabaseType::PostgreSQL)
        }
        "sqlserver" | "mssql" => Ok(DatabaseType::MSSQL),
        "oracle" => Ok(DatabaseType::Oracle),
        "clickhouse" => Ok(DatabaseType::ClickHouse),
        "sqlite" => Ok(DatabaseType::SQLite),
        "duckdb" => Ok(DatabaseType::DuckDB),
        value => Err(ImportError::UnsupportedDatabaseType(value.to_string())),
    }
}

fn port(value: Option<i64>) -> Option<u16> {
    value.and_then(|value| u16::try_from(value).ok())
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

fn default_app_db_path() -> Option<PathBuf> {
    portable_path()
        .into_iter()
        .chain(user_data_paths())
        .find(|path| path.exists())
}

fn portable_path() -> Option<PathBuf> {
    std::env::var_os("PORTABLE_EXECUTABLE_DIR")
        .map(PathBuf::from)
        .map(|path| path.join("beekeeper_studio_data/app.db"))
}

fn user_data_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for base in [dirs::data_dir(), dirs::config_dir()].into_iter().flatten() {
        for directory in ["Beekeeper Studio", "beekeeper-studio", "beekeeperstudio"] {
            paths.push(base.join(directory).join("app.db"));
        }
    }
    paths
}
