use crate::{
    CredentialQuery, CredentialStore, ImportError, ImportOptions, ImportSourceKind,
    ImportedConnection, PasswordImportStatus, SourceAvailability,
};
use one_core::storage::DatabaseType;
use plist::{Dictionary, Value};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

const ROOT_KEY: &str = "Favorites Root";
const EXPORT_ROOT_KEY: &str = "SPConnectionFavorites";
const OLD_FAVORITES_KEY: &str = "favorites";
const SOCKET_CONNECTION_TYPE: i64 = 1;
const DEFAULT_MYSQL_PORT: u16 = 3306;

pub fn detect_availability() -> SourceAvailability {
    let Some(path) = default_favorites_path() else {
        return SourceAvailability::NotInstalled;
    };
    let Ok(contents) = std::fs::read(path) else {
        return SourceAvailability::PermissionRequired;
    };
    match parse_sequel_ace_favorites_plist_with_credentials(
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
    let path = default_favorites_path()
        .ok_or_else(|| ImportError::SourceDataNotFound("Sequel Ace Favorites.plist".to_string()))?;
    preview_connections_from_path(path, options, credentials)
}

pub fn preview_connections_from_path(
    path: impl AsRef<Path>,
    options: ImportOptions,
    credentials: &dyn CredentialStore,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let contents = std::fs::read(path.as_ref())
        .map_err(|error| ImportError::ReadSourceData(error.to_string()))?;
    parse_sequel_ace_favorites_plist_with_credentials(&contents, options, credentials)
}

pub fn parse_sequel_ace_favorites_plist_with_credentials(
    contents: &[u8],
    options: ImportOptions,
    credentials: &dyn CredentialStore,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let root = Value::from_reader(Cursor::new(contents))
        .map_err(|error| ImportError::InvalidSourceData(error.to_string()))?;
    let mut favorites = Vec::new();
    collect_favorites(&root, &mut favorites)?;
    favorites
        .iter()
        .map(|favorite| parse_favorite(favorite, options, credentials))
        .collect()
}

fn collect_favorites<'a>(
    value: &'a Value,
    favorites: &mut Vec<&'a Dictionary>,
) -> Result<(), ImportError> {
    let dictionary = value
        .as_dictionary()
        .ok_or(ImportError::MissingField(ROOT_KEY))?;
    if let Some(children) = children_array(dictionary) {
        collect_favorite_items(children, favorites);
        return Ok(());
    }
    if is_favorite_dictionary(dictionary) {
        favorites.push(dictionary);
    }
    Ok(())
}

fn children_array(dictionary: &Dictionary) -> Option<&Vec<Value>> {
    dictionary
        .get(ROOT_KEY)
        .and_then(Value::as_dictionary)
        .and_then(|root| root.get("Children"))
        .and_then(Value::as_array)
        .or_else(|| dictionary.get(EXPORT_ROOT_KEY).and_then(Value::as_array))
        .or_else(|| dictionary.get(OLD_FAVORITES_KEY).and_then(Value::as_array))
}

fn collect_favorite_items<'a>(items: &'a [Value], favorites: &mut Vec<&'a Dictionary>) {
    for item in items {
        let Some(dictionary) = item.as_dictionary() else {
            continue;
        };
        if let Some(children) = dictionary.get("Children").and_then(Value::as_array) {
            collect_favorite_items(children, favorites);
        } else if is_favorite_dictionary(dictionary) {
            favorites.push(dictionary);
        }
    }
}

fn parse_favorite(
    favorite: &Dictionary,
    options: ImportOptions,
    credentials: &dyn CredentialStore,
) -> Result<ImportedConnection, ImportError> {
    let source_id = string_value(favorite, "id").ok_or(ImportError::MissingField("id"))?;
    let name = string_value(favorite, "name").unwrap_or_else(|| source_id.clone());
    let host = host(favorite)?;
    let username = string_value(favorite, "user").unwrap_or_default();
    let database = string_value(favorite, "database").filter(|value| !value.is_empty());
    let password = password(favorite, &source_id, &name, &host, options, credentials);

    Ok(ImportedConnection {
        source: ImportSourceKind::SequelAce,
        source_id,
        name,
        database_type: DatabaseType::MySQL,
        host,
        port: port(favorite).or(Some(DEFAULT_MYSQL_PORT)),
        username,
        password: password.value,
        database,
        extra_params: HashMap::new(),
        password_status: password.status,
    })
}

fn is_favorite_dictionary(dictionary: &Dictionary) -> bool {
    dictionary.contains_key("id")
        && (dictionary.contains_key("host") || dictionary.contains_key("socket"))
}

fn host(favorite: &Dictionary) -> Result<String, ImportError> {
    if integer_value(favorite, "type") == Some(SOCKET_CONNECTION_TYPE) {
        return Ok("localhost".to_string());
    }
    string_value(favorite, "host").ok_or(ImportError::MissingField("host"))
}

struct PasswordLookup {
    value: Option<String>,
    status: PasswordImportStatus,
}

fn password(
    favorite: &Dictionary,
    source_id: &str,
    name: &str,
    host: &str,
    options: ImportOptions,
    credentials: &dyn CredentialStore,
) -> PasswordLookup {
    if !options.include_passwords {
        return PasswordLookup {
            value: None,
            status: PasswordImportStatus::Unsupported,
        };
    }
    let Some(query) = credential_query(favorite, source_id, name, host) else {
        return missing_password();
    };
    credentials
        .get_password(&query)
        .map(|password| PasswordLookup {
            value: Some(password),
            status: PasswordImportStatus::Included,
        })
        .unwrap_or_else(missing_password)
}

fn credential_query(
    favorite: &Dictionary,
    source_id: &str,
    name: &str,
    host: &str,
) -> Option<CredentialQuery> {
    let user = string_value(favorite, "user")?;
    let database = string_value(favorite, "database").unwrap_or_default();
    let service = format!("Sequel Ace : {} ({})", name, source_id);
    let account = format!("{}@{}/{}", user, host, database);
    Some(CredentialQuery::new(service, account))
}

fn missing_password() -> PasswordLookup {
    PasswordLookup {
        value: None,
        status: PasswordImportStatus::Missing,
    }
}

#[cfg(target_os = "macos")]
fn default_favorites_path() -> Option<std::path::PathBuf> {
    dirs::home_dir()
        .map(|home| home.join("Library/Application Support/Sequel Ace/Data/Favorites.plist"))
        .filter(|path| path.exists())
}

#[cfg(not(target_os = "macos"))]
fn default_favorites_path() -> Option<std::path::PathBuf> {
    None
}

fn string_value(dictionary: &Dictionary, key: &str) -> Option<String> {
    match dictionary.get(key)? {
        Value::String(value) => Some(value.clone()),
        Value::Integer(value) => Some(value.to_string()),
        Value::Real(value) => Some(value.to_string()),
        Value::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}

fn integer_value(dictionary: &Dictionary, key: &str) -> Option<i64> {
    match dictionary.get(key)? {
        Value::Integer(value) => value.as_signed(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn port(dictionary: &Dictionary) -> Option<u16> {
    string_value(dictionary, "port").and_then(|value| value.parse().ok())
}
