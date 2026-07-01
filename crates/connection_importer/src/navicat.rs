use crate::navicat_plist::parse_navicat_connections_plist;
use crate::{
    ImportError, ImportOptions, ImportSourceKind, ImportedConnection, PasswordImportStatus,
    SourceAvailability,
};
use one_core::storage::DatabaseType;
use roxmltree::{Document, Node};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn detect_availability() -> SourceAvailability {
    let Some(path) = default_connections_path() else {
        return if installed_marker_path().is_some() {
            SourceAvailability::Installed
        } else {
            SourceAvailability::NotInstalled
        };
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
        Err(ImportError::ReadSourceData(_)) => SourceAvailability::PermissionRequired,
        Err(error) => SourceAvailability::Error {
            message: error.to_string(),
        },
    }
}

pub fn preview_default_connections(
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let path = default_connections_path()
        .ok_or_else(|| ImportError::SourceDataNotFound("Navicat connections".to_string()))?;
    preview_connections_from_path(path, options)
}

pub fn preview_connections_from_path(
    path: impl AsRef<Path>,
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let path = path.as_ref();
    let contents =
        std::fs::read(path).map_err(|error| ImportError::ReadSourceData(error.to_string()))?;
    if path
        .extension()
        .is_some_and(|extension| extension == "plist")
    {
        return parse_navicat_connections_plist(&contents, options);
    }
    let contents = String::from_utf8(contents)
        .map_err(|error| ImportError::ReadSourceData(error.to_string()))?;
    parse_navicat_connections_xml(&contents, options)
}

pub fn parse_navicat_connections_xml(
    contents: &str,
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let document = Document::parse(contents)
        .map_err(|error| ImportError::InvalidSourceData(error.to_string()))?;
    document
        .descendants()
        .filter(|node| node.has_tag_name("Connection") && attr_any(*node, &["Host"]).is_some())
        .map(|node| parse_connection(node, options))
        .collect()
}

fn parse_connection(
    node: Node<'_, '_>,
    options: ImportOptions,
) -> Result<ImportedConnection, ImportError> {
    let name = attr_any(node, &["ConnectionName", "Name", "ConnName"]).unwrap_or("Navicat");
    let database_type = database_type(node)?;
    Ok(ImportedConnection {
        source: ImportSourceKind::Navicat,
        source_id: name.to_string(),
        name: name.to_string(),
        database_type: database_type.clone(),
        host: required_attr_any(node, &["Host"])?.to_string(),
        port: port(node).or_else(|| default_port(&database_type)),
        username: attr_any(node, &["UserName", "User", "Username"])
            .unwrap_or_default()
            .to_string(),
        password: None,
        database: attr_any(node, &["Database", "DBName", "Schema"]).map(str::to_string),
        extra_params: HashMap::new(),
        password_status: password_status(options),
    })
}

fn database_type(node: Node<'_, '_>) -> Result<DatabaseType, ImportError> {
    let raw = attr_any(node, &["ConnType", "Type", "DatabaseType"])
        .unwrap_or_default()
        .to_lowercase();
    match raw.as_str() {
        value if value.contains("mysql") || value.contains("maria") => Ok(DatabaseType::MySQL),
        value if value.contains("postgre") || value.contains("pgsql") => {
            Ok(DatabaseType::PostgreSQL)
        }
        value if value.contains("mssql") || value.contains("sqlserver") => Ok(DatabaseType::MSSQL),
        value if value.contains("oracle") => Ok(DatabaseType::Oracle),
        value if value.contains("sqlite") => Ok(DatabaseType::SQLite),
        value if value.contains("clickhouse") => Ok(DatabaseType::ClickHouse),
        _ => Err(ImportError::UnsupportedDatabaseType(raw)),
    }
}

fn default_connections_path() -> Option<PathBuf> {
    candidate_connection_paths()
        .into_iter()
        .find(|path| path.exists())
}

fn installed_marker_path() -> Option<PathBuf> {
    candidate_installed_marker_paths()
        .into_iter()
        .find(|path| path.exists())
}

fn candidate_connection_paths() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    candidate_connection_paths_for_home(&home)
}

fn candidate_installed_marker_paths() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    candidate_installed_marker_paths_for_home(&home)
}

fn candidate_connection_paths_for_home(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join("Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist"),
        home.join(
            "Library/Application Support/PremiumSoft CyberTech/Navicat Premium/Common/conn.plist",
        ),
        home.join("Library/Application Support/PremiumSoft CyberTech/Navicat/Common/conn.plist"),
        home.join("Documents/Navicat/connections.ncx"),
        home.join("Documents/Navicat/connections.xml"),
        home.join("AppData/Roaming/PremiumSoft/Navicat/connections.ncx"),
        home.join("Library/Application Support/PremiumSoft CyberTech/Navicat/connections.ncx"),
    ]
}

fn candidate_installed_marker_paths_for_home(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join("Library/Preferences/com.navicat.NavicatPremiumLite.plist"),
        home.join("Library/Preferences/com.prect.Navicat.plist"),
        home.join("Library/Preferences/com.prect.NavicatPremium.plist"),
        home.join("Library/Preferences/com.prect.NavicatPremiumEssentials.plist"),
    ]
}

fn attr_any<'a>(node: Node<'a, 'a>, fields: &[&str]) -> Option<&'a str> {
    fields.iter().find_map(|field| node.attribute(*field))
}

fn required_attr_any<'a>(
    node: Node<'a, 'a>,
    fields: &[&'static str],
) -> Result<&'a str, ImportError> {
    attr_any(node, fields).ok_or(ImportError::MissingField(fields[0]))
}

fn port(node: Node<'_, '_>) -> Option<u16> {
    attr_any(node, &["Port"]).and_then(|value| value.parse().ok())
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{candidate_connection_paths_for_home, candidate_installed_marker_paths_for_home};

    #[test]
    fn navicat_candidates_include_premium_lite_preferences_marker() {
        let paths = candidate_installed_marker_paths_for_home(Path::new("/home/me"));

        assert!(paths.iter().any(|path| {
            path.ends_with("Library/Preferences/com.navicat.NavicatPremiumLite.plist")
        }));
    }

    #[test]
    fn navicat_candidates_include_exported_connection_files() {
        let paths = candidate_connection_paths_for_home(Path::new("/home/me"));

        assert!(paths.iter().any(|path| {
            path.ends_with(
                "Library/Application Support/PremiumSoft CyberTech/Navicat CC/Common/conn.plist",
            )
        }));
        assert!(
            paths
                .iter()
                .any(|path| path.ends_with("Documents/Navicat/connections.ncx"))
        );
        assert!(
            paths
                .iter()
                .any(|path| path.ends_with("Documents/Navicat/connections.xml"))
        );
    }
}
