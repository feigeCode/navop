use crate::{
    ImportError, ImportOptions, ImportSourceKind, ImportedSshAuthMethod, ImportedSshConnection,
    PasswordImportStatus, SourceAvailability,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

type Ini = HashMap<String, HashMap<String, String>>;

pub fn detect_availability() -> SourceAvailability {
    let Some(path) = default_sessions_path() else {
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
    let path = default_sessions_path()
        .ok_or_else(|| ImportError::SourceDataNotFound("Xshell sessions".to_string()))?;
    preview_ssh_connections_from_path(path, options)
}

pub fn preview_ssh_connections_from_path(
    path: impl AsRef<Path>,
    options: ImportOptions,
) -> Result<Vec<ImportedSshConnection>, ImportError> {
    let paths = session_files(path.as_ref())?;
    paths
        .iter()
        .filter_map(|path| parse_session_file(path, options).transpose())
        .collect()
}

pub fn parse_xshell_session(
    contents: &[u8],
    source_id: &str,
    fallback_name: &str,
    options: ImportOptions,
) -> Result<Option<ImportedSshConnection>, ImportError> {
    let text = decode_session(contents)?;
    let ini = parse_ini(&text);
    if !is_ssh_session(&ini) {
        return Ok(None);
    }
    let host = required_value(&ini, &["CONNECTION"], "Host")?;
    Ok(Some(ImportedSshConnection {
        source: ImportSourceKind::Xshell,
        source_id: source_id.to_string(),
        name: value(&ini, &["CONNECTION"], "Name").unwrap_or_else(|| fallback_name.to_string()),
        host,
        port: value(&ini, &["CONNECTION"], "Port")
            .and_then(|port| port.parse().ok())
            .unwrap_or(22),
        username: username(&ini),
        auth_method: auth_method(&ini),
        password_status: password_status(options),
    }))
}

fn parse_session_file(
    path: &Path,
    options: ImportOptions,
) -> Result<Option<ImportedSshConnection>, ImportError> {
    let contents =
        std::fs::read(path).map_err(|error| ImportError::ReadSourceData(error.to_string()))?;
    let source_id = path.to_string_lossy().to_string();
    let fallback_name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Xshell")
        .to_string();
    parse_xshell_session(&contents, &source_id, &fallback_name, options)
}

fn session_files(path: &Path) -> Result<Vec<PathBuf>, ImportError> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut paths = Vec::new();
    collect_session_files(path, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_session_files(path: &Path, paths: &mut Vec<PathBuf>) -> Result<(), ImportError> {
    let entries =
        std::fs::read_dir(path).map_err(|error| ImportError::ReadSourceData(error.to_string()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_session_files(&path, paths)?;
        } else if is_xsh_file(&path) {
            paths.push(path);
        }
    }
    Ok(())
}

fn is_xsh_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xsh"))
}

fn parse_ini(text: &str) -> Ini {
    let mut ini = Ini::new();
    let mut section = String::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with([';', '#']) {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_ascii_lowercase();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            ini.entry(section.clone())
                .or_default()
                .insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    ini
}

fn is_ssh_session(ini: &Ini) -> bool {
    value(ini, &["CONNECTION"], "Protocol")
        .map(|protocol| protocol.eq_ignore_ascii_case("SSH"))
        .unwrap_or(true)
}

fn username(ini: &Ini) -> String {
    value(ini, &["AUTHENTICATION", "CONNECTION"], "UserName").unwrap_or_default()
}

fn auth_method(ini: &Ini) -> ImportedSshAuthMethod {
    let key_path = value(ini, &["AUTHENTICATION"], "UserKey")
        .or_else(|| value(ini, &["AUTHENTICATION"], "UserKeyFile"))
        .filter(|value| !value.is_empty());
    let method = value(ini, &["AUTHENTICATION"], "Method")
        .unwrap_or_default()
        .to_ascii_uppercase();
    if let Some(key_path) = key_path {
        return ImportedSshAuthMethod::PrivateKey {
            key_path,
            passphrase: None,
        };
    }
    if method.contains("PASSWORD") {
        return ImportedSshAuthMethod::Password { password: None };
    }
    ImportedSshAuthMethod::AutoPublicKey
}

fn required_value(ini: &Ini, sections: &[&str], key: &'static str) -> Result<String, ImportError> {
    value(ini, sections, key).ok_or(ImportError::MissingField(key))
}

fn value(ini: &Ini, sections: &[&str], key: &str) -> Option<String> {
    let key = key.to_ascii_lowercase();
    sections
        .iter()
        .filter_map(|section| ini.get(&section.to_ascii_lowercase()))
        .find_map(|section| section.get(&key).filter(|value| !value.is_empty()))
        .cloned()
}

fn decode_session(contents: &[u8]) -> Result<String, ImportError> {
    if contents.starts_with(&[0xff, 0xfe]) {
        return decode_utf16(&contents[2..], true);
    }
    if contents.starts_with(&[0xfe, 0xff]) {
        return decode_utf16(&contents[2..], false);
    }
    if looks_utf16_le(contents) {
        return decode_utf16(contents, true);
    }
    String::from_utf8(contents.to_vec())
        .map_err(|error| ImportError::InvalidSourceData(error.to_string()))
}

fn decode_utf16(contents: &[u8], little_endian: bool) -> Result<String, ImportError> {
    let words = contents.chunks_exact(2).map(|chunk| {
        if little_endian {
            u16::from_le_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_be_bytes([chunk[0], chunk[1]])
        }
    });
    String::from_utf16(&words.collect::<Vec<_>>())
        .map_err(|error| ImportError::InvalidSourceData(error.to_string()))
}

fn looks_utf16_le(contents: &[u8]) -> bool {
    contents.len() > 4
        && contents
            .iter()
            .skip(1)
            .step_by(2)
            .take(8)
            .all(|byte| *byte == 0)
}

fn default_sessions_path() -> Option<PathBuf> {
    session_roots()
        .into_iter()
        .flat_map(versioned_session_paths)
        .find(|path| path.exists())
}

fn session_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(documents) = dirs::document_dir() {
        roots.push(documents.join("NetSarang Computer"));
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Documents/NetSarang Computer"));
    }
    roots
}

fn versioned_session_paths(root: PathBuf) -> Vec<PathBuf> {
    (5..=8)
        .rev()
        .map(|version| root.join(version.to_string()).join("Xshell/Sessions"))
        .collect()
}

fn password_status(_options: ImportOptions) -> PasswordImportStatus {
    PasswordImportStatus::Unsupported
}
