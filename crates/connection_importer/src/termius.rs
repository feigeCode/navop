use crate::{
    ImportError, ImportOptions, ImportSourceKind, ImportedSshAuthMethod, ImportedSshConnection,
    PasswordImportStatus, SourceAvailability,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn detect_availability() -> SourceAvailability {
    let Some(path) = default_hosts_path() else {
        return SourceAvailability::NotInstalled;
    };
    match preview_ssh_connections_from_path(
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

pub fn preview_default_ssh_connections(
    options: ImportOptions,
) -> Result<Vec<ImportedSshConnection>, ImportError> {
    let path = default_hosts_path()
        .ok_or_else(|| ImportError::SourceDataNotFound("Termius hosts".to_string()))?;
    preview_ssh_connections_from_path(path, options)
}

pub fn preview_ssh_connections_from_path(
    path: impl AsRef<Path>,
    options: ImportOptions,
) -> Result<Vec<ImportedSshConnection>, ImportError> {
    let mut imported = Vec::new();
    for path in json_files(path.as_ref())? {
        let contents = std::fs::read_to_string(&path)
            .map_err(|error| ImportError::ReadSourceData(error.to_string()))?;
        imported.extend(parse_termius_hosts_json(&contents, options)?);
    }
    Ok(imported)
}

pub fn parse_termius_hosts_json(
    contents: &str,
    options: ImportOptions,
) -> Result<Vec<ImportedSshConnection>, ImportError> {
    let root: Value = serde_json::from_str(contents)
        .map_err(|error| ImportError::InvalidSourceData(error.to_string()))?;
    host_items(&root)
        .iter()
        .map(|item| parse_host(item, options))
        .collect()
}

fn parse_host(value: &Value, options: ImportOptions) -> Result<ImportedSshConnection, ImportError> {
    let host = required_string_any(value, &["address", "hostname", "host"])?;
    let source_id = string_any(value, &["id", "uuid", "key"]).unwrap_or(host);
    Ok(ImportedSshConnection {
        source: ImportSourceKind::Termius,
        source_id: source_id.to_string(),
        name: string_any(value, &["label", "name", "title"])
            .unwrap_or(source_id)
            .to_string(),
        host: host.to_string(),
        port: port(value).unwrap_or(22),
        username: string_any(value, &["username", "user", "login"])
            .unwrap_or_default()
            .to_string(),
        auth_method: auth_method(value),
        password_status: password_status(options),
    })
}

fn host_items(root: &Value) -> Vec<&Value> {
    root.as_array()
        .or_else(|| root.get("hosts").and_then(Value::as_array))
        .or_else(|| root.get("items").and_then(Value::as_array))
        .or_else(|| root.get("data").and_then(Value::as_array))
        .map(|items| items.iter().filter(|item| has_host(item)).collect())
        .unwrap_or_else(|| {
            if has_host(root) {
                vec![root]
            } else {
                Vec::new()
            }
        })
}

fn auth_method(value: &Value) -> ImportedSshAuthMethod {
    if let Some(key_path) = key_path(value) {
        return ImportedSshAuthMethod::PrivateKey {
            key_path,
            passphrase: None,
        };
    }
    ImportedSshAuthMethod::Password { password: None }
}

fn key_path(value: &Value) -> Option<String> {
    string_any(value, &["key_path", "keyPath", "private_key", "privateKey"])
        .map(str::to_string)
        .or_else(|| {
            value.get("identity").and_then(|identity| {
                string_any(
                    identity,
                    &["key_path", "keyPath", "private_key", "privateKey"],
                )
                .map(str::to_string)
            })
        })
}

fn json_files(path: &Path) -> Result<Vec<PathBuf>, ImportError> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut paths = Vec::new();
    collect_json_files(path, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_json_files(path: &Path, paths: &mut Vec<PathBuf>) -> Result<(), ImportError> {
    let entries =
        std::fs::read_dir(path).map_err(|error| ImportError::ReadSourceData(error.to_string()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, paths)?;
        } else if is_json_file(&path) {
            paths.push(path);
        }
    }
    Ok(())
}

fn is_json_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

fn default_hosts_path() -> Option<PathBuf> {
    candidate_roots().into_iter().find(|path| path.exists())
}

fn candidate_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    vec![
        home.join("Library/Application Support/Termius/hosts.json"),
        home.join("Library/Application Support/Termius/connections.json"),
        home.join(".config/Termius/hosts.json"),
        home.join(".config/termius/hosts.json"),
        home.join("AppData/Roaming/Termius/hosts.json"),
    ]
}

fn has_host(value: &Value) -> bool {
    string_any(value, &["address", "hostname", "host"]).is_some()
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
                .or_else(|| port.as_u64().map(|value| value.to_string()))
        })
        .and_then(|port| port.parse().ok())
}

fn password_status(_options: ImportOptions) -> PasswordImportStatus {
    PasswordImportStatus::Unsupported
}
