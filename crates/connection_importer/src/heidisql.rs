use crate::{
    ImportError, ImportOptions, ImportSourceKind, ImportedConnection, PasswordImportStatus,
    SourceAvailability,
};
use one_core::storage::DatabaseType;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

type Ini = Vec<(String, HashMap<String, String>)>;

pub fn detect_availability() -> SourceAvailability {
    let Some(path) = default_settings_path() else {
        return SourceAvailability::NotInstalled;
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return SourceAvailability::PermissionRequired;
    };
    match parse_heidisql_settings_ini(
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
    let path = default_settings_path()
        .ok_or_else(|| ImportError::SourceDataNotFound("HeidiSQL settings".to_string()))?;
    preview_connections_from_path(path, options)
}

pub fn preview_connections_from_path(
    path: impl AsRef<Path>,
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let contents = std::fs::read_to_string(path.as_ref())
        .map_err(|error| ImportError::ReadSourceData(error.to_string()))?;
    parse_heidisql_settings_ini(&contents, options)
}

pub fn parse_heidisql_settings_ini(
    contents: &str,
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    parse_ini(contents)
        .iter()
        .filter(|(section, values)| is_server_section(section) && has_host(values))
        .map(|(section, values)| parse_server(section, values, options))
        .collect()
}

fn parse_server(
    section: &str,
    values: &HashMap<String, String>,
    options: ImportOptions,
) -> Result<ImportedConnection, ImportError> {
    let name = server_name(section);
    let database_type = database_type(values)?;
    Ok(ImportedConnection {
        source: ImportSourceKind::HeidiSQL,
        source_id: name.clone(),
        name,
        database_type: database_type.clone(),
        host: required_value(values, "host")?.to_string(),
        port: port(values).or_else(|| default_port(&database_type)),
        username: value_any(values, &["user", "username"])
            .unwrap_or_default()
            .to_string(),
        password: None,
        database: value_any(values, &["database", "db"]).map(str::to_string),
        extra_params: HashMap::new(),
        password_status: password_status(options),
    })
}

fn parse_ini(contents: &str) -> Ini {
    let mut ini = Vec::new();
    let mut section_index = None;
    for line in contents.lines().map(str::trim) {
        if line.is_empty() || line.starts_with([';', '#']) {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            ini.push((line[1..line.len() - 1].to_string(), HashMap::new()));
            section_index = Some(ini.len() - 1);
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let Some(index) = section_index else {
                continue;
            };
            ini[index]
                .1
                .insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    ini
}

fn database_type(values: &HashMap<String, String>) -> Result<DatabaseType, ImportError> {
    let raw = value_any(values, &["driver", "library", "nettype"])
        .unwrap_or_default()
        .to_lowercase();
    match raw.as_str() {
        value if value.contains("postgre") || value == "4" => Ok(DatabaseType::PostgreSQL),
        value if value.contains("mssql") || value.contains("sqlserver") || value == "3" => {
            Ok(DatabaseType::MSSQL)
        }
        value if value.contains("mysql") || value.contains("maria") || value == "0" => {
            Ok(DatabaseType::MySQL)
        }
        _ if raw.is_empty() => Ok(DatabaseType::MySQL),
        _ => Err(ImportError::UnsupportedDatabaseType(raw)),
    }
}

fn default_settings_path() -> Option<PathBuf> {
    candidate_paths().into_iter().find(|path| path.exists())
}

fn candidate_paths() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    vec![
        home.join("portable_settings.txt"),
        home.join("AppData/Roaming/HeidiSQL/portable_settings.txt"),
        home.join(".config/HeidiSQL/portable_settings.txt"),
        home.join(".config/heidisql/portable_settings.txt"),
    ]
}

fn is_server_section(section: &str) -> bool {
    section.to_ascii_lowercase().starts_with("servers\\")
}

fn server_name(section: &str) -> String {
    section
        .rsplit_once('\\')
        .map(|(_, name)| name)
        .unwrap_or(section)
        .to_string()
}

fn has_host(values: &HashMap<String, String>) -> bool {
    values.get("host").is_some_and(|value| !value.is_empty())
}

fn required_value<'a>(
    values: &'a HashMap<String, String>,
    field: &'static str,
) -> Result<&'a str, ImportError> {
    value_any(values, &[field]).ok_or(ImportError::MissingField(field))
}

fn value_any<'a>(values: &'a HashMap<String, String>, fields: &[&str]) -> Option<&'a str> {
    fields
        .iter()
        .find_map(|field| values.get(&field.to_ascii_lowercase()))
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

fn port(values: &HashMap<String, String>) -> Option<u16> {
    value_any(values, &["port"]).and_then(|value| value.parse().ok())
}

fn default_port(database_type: &DatabaseType) -> Option<u16> {
    match database_type {
        DatabaseType::MySQL => Some(3306),
        DatabaseType::PostgreSQL => Some(5432),
        DatabaseType::MSSQL => Some(1433),
        _ => None,
    }
}

fn password_status(_options: ImportOptions) -> PasswordImportStatus {
    PasswordImportStatus::Unsupported
}
