use crate::{
    ImportError, ImportOptions, ImportSourceKind, ImportedConnection, PasswordImportStatus,
    SourceAvailability,
};
use one_core::storage::DatabaseType;
use roxmltree::{Document, Node};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn detect_availability() -> SourceAvailability {
    let Some(path) = default_data_sources_path() else {
        return SourceAvailability::NotInstalled;
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return SourceAvailability::PermissionRequired;
    };
    match parse_datagrip_data_sources_xml(
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
        .ok_or_else(|| ImportError::SourceDataNotFound("DataGrip dataSources.xml".to_string()))?;
    preview_connections_from_path(path, options)
}

pub fn preview_connections_from_path(
    path: impl AsRef<Path>,
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let path = data_sources_file(path.as_ref());
    let contents = std::fs::read_to_string(&path)
        .map_err(|error| ImportError::ReadSourceData(error.to_string()))?;
    parse_datagrip_data_sources_xml(&contents, options)
}

pub fn parse_datagrip_data_sources_xml(
    contents: &str,
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let document = Document::parse(contents)
        .map_err(|error| ImportError::InvalidSourceData(error.to_string()))?;
    document
        .descendants()
        .filter(|node| node.has_tag_name("data-source"))
        .map(|node| parse_data_source(node, options))
        .collect()
}

fn parse_data_source(
    node: Node<'_, '_>,
    options: ImportOptions,
) -> Result<ImportedConnection, ImportError> {
    let jdbc_url = child_text(node, "jdbc-url").ok_or(ImportError::MissingField("jdbc-url"))?;
    let database_type = database_type(node, &jdbc_url)?;
    let parts = jdbc_parts(&jdbc_url, &database_type)?;
    let source_id = node.attribute("uuid").unwrap_or(parts.host.as_str());
    Ok(ImportedConnection {
        source: ImportSourceKind::DataGrip,
        source_id: source_id.to_string(),
        name: node.attribute("name").unwrap_or(source_id).to_string(),
        database_type: database_type.clone(),
        host: parts.host,
        port: parts.port.or_else(|| default_port(&database_type)),
        username: username(node),
        password: None,
        database: parts.database,
        extra_params: HashMap::new(),
        password_status: password_status(options),
    })
}

struct JdbcParts {
    host: String,
    port: Option<u16>,
    database: Option<String>,
}

fn jdbc_parts(url: &str, database_type: &DatabaseType) -> Result<JdbcParts, ImportError> {
    match database_type {
        DatabaseType::SQLite => Ok(file_jdbc_parts(url, "jdbc:sqlite:")),
        DatabaseType::DuckDB => Ok(file_jdbc_parts(url, "jdbc:duckdb:")),
        DatabaseType::Oracle => parse_oracle_url(url),
        _ => parse_host_url(url),
    }
}

fn parse_host_url(url: &str) -> Result<JdbcParts, ImportError> {
    let after_scheme = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .ok_or_else(|| ImportError::InvalidSourceData(format!("unsupported JDBC URL: {url}")))?;
    let before_query = after_scheme.split('?').next().unwrap_or(after_scheme);
    let (endpoint, params) = before_query.split_once(';').unwrap_or((before_query, ""));
    let (host_port, database) = endpoint.split_once('/').unwrap_or((endpoint, ""));
    let (host, port) = split_host_port(host_port);
    Ok(JdbcParts {
        host: host.to_string(),
        port,
        database: non_empty(database)
            .map(str::to_string)
            .or_else(|| jdbc_param(params, &["databaseName", "database"])),
    })
}

fn parse_oracle_url(url: &str) -> Result<JdbcParts, ImportError> {
    let rest = url
        .split_once("@//")
        .map(|(_, rest)| rest)
        .or_else(|| url.split_once('@').map(|(_, rest)| rest))
        .ok_or_else(|| ImportError::InvalidSourceData(format!("unsupported JDBC URL: {url}")))?;
    parse_host_url(&format!("jdbc:oracle://{rest}"))
}

fn file_jdbc_parts(url: &str, prefix: &str) -> JdbcParts {
    let path = url.strip_prefix(prefix).unwrap_or(url).to_string();
    JdbcParts {
        host: path.clone(),
        port: None,
        database: non_empty(&path).map(str::to_string),
    }
}

fn database_type(node: Node<'_, '_>, jdbc_url: &str) -> Result<DatabaseType, ImportError> {
    let raw = child_text(node, "driver-ref")
        .unwrap_or_else(|| jdbc_url.to_string())
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

fn child_text(node: Node<'_, '_>, tag: &str) -> Option<String> {
    node.children()
        .find(|child| child.has_tag_name(tag))
        .and_then(|child| child.text())
        .map(str::trim)
        .and_then(non_empty)
        .map(str::to_string)
}

fn username(node: Node<'_, '_>) -> String {
    child_text(node, "user-name")
        .or_else(|| property(node, "user"))
        .unwrap_or_default()
}

fn property(node: Node<'_, '_>, name: &str) -> Option<String> {
    node.descendants()
        .filter(|child| child.has_tag_name("property"))
        .find(|child| child.attribute("name") == Some(name))
        .and_then(|child| child.attribute("value"))
        .map(str::to_string)
}

fn split_host_port(value: &str) -> (&str, Option<u16>) {
    let Some((host, port)) = value.rsplit_once(':') else {
        return (value, None);
    };
    match port.parse() {
        Ok(port) => (host, Some(port)),
        Err(_) => (value, None),
    }
}

fn jdbc_param(params: &str, names: &[&str]) -> Option<String> {
    params
        .split(';')
        .filter_map(|item| item.split_once('='))
        .find(|(key, _)| names.iter().any(|name| key.eq_ignore_ascii_case(name)))
        .map(|(_, value)| value.to_string())
}

fn default_data_sources_path() -> Option<PathBuf> {
    datagrip_config_roots()
        .into_iter()
        .flat_map(datagrip_options_files)
        .find(|path| path.exists())
}

fn datagrip_config_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(config) = dirs::config_dir() {
        roots.push(config.join("JetBrains"));
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Library/Application Support/JetBrains"));
        roots.push(home.join("AppData/Roaming/JetBrains"));
        roots.push(home.join(".config/JetBrains"));
    }
    roots
}

fn datagrip_options_files(root: PathBuf) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_datagrip_dir(path))
        .map(|path| path.join("options/dataSources.xml"))
        .collect()
}

fn is_datagrip_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("DataGrip"))
}

fn data_sources_file(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join("dataSources.xml")
    } else {
        path.to_path_buf()
    }
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn password_status(_options: ImportOptions) -> PasswordImportStatus {
    PasswordImportStatus::Unsupported
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
