use std::ffi::OsString;
use std::path::PathBuf;

use connection_import_protocol::{
    CandidateFile, DirectoryEntry, HostAccessError, Platform, SecretQuery, SecretResult,
};
use extension_component::{CandidateFileAccess, ExtensionConnectionImportHost, PermissionSet};

pub(crate) struct ManifestConnectionImportHost {
    candidates: Vec<CandidateFile>,
    permissions: PermissionSet,
}

impl ManifestConnectionImportHost {
    pub(crate) fn new<I, S>(candidates: Vec<CandidateFile>, permissions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            candidates,
            permissions: PermissionSet::new(permissions),
        }
    }

    fn candidate_access(&self) -> CandidateFileAccess {
        CandidateFileAccess::new(self.candidates.clone(), self.permissions.clone())
    }
}

impl ExtensionConnectionImportHost for ManifestConnectionImportHost {
    fn current_platform(&self) -> Platform {
        current_platform()
    }

    fn list_candidate_files(&self, _importer_id: &str) -> Vec<CandidateFile> {
        self.candidates.clone()
    }

    fn read_file(&self, candidate_id: &str) -> Result<Vec<u8>, HostAccessError> {
        let candidate = self.candidate_access().candidate(candidate_id)?.clone();
        std::fs::read(expand_connection_import_path(&candidate.path)).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                HostAccessError::NotFound(candidate.path)
            } else {
                HostAccessError::Io(error.to_string())
            }
        })
    }

    fn read_directory(&self, candidate_id: &str) -> Result<Vec<DirectoryEntry>, HostAccessError> {
        let candidate = self.candidate_access().candidate(candidate_id)?.clone();
        let entries = std::fs::read_dir(expand_connection_import_path(&candidate.path))
            .map_err(|error| HostAccessError::Io(error.to_string()))?;
        let mut out = Vec::new();
        for entry in entries.flatten() {
            out.push(DirectoryEntry {
                candidate_id: candidate_id.to_string(),
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir: entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false),
            });
        }
        Ok(out)
    }

    fn read_candidate_child_file(
        &self,
        candidate_id: &str,
        relative_path: &str,
    ) -> Result<Vec<u8>, HostAccessError> {
        let (candidate, child) = self
            .candidate_access()
            .validate_child(candidate_id, relative_path)?;
        let path = expand_connection_import_path(&candidate.path).join(child);
        std::fs::read(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                HostAccessError::NotFound(path.display().to_string())
            } else {
                HostAccessError::Io(error.to_string())
            }
        })
    }

    fn read_secret(&self, query: SecretQuery) -> SecretResult {
        let (namespace, key) = secret_scope(&query);
        if !self.permissions.allows_secret_read(&namespace, &key) {
            return SecretResult::PermissionDenied;
        }
        read_platform_secret(&query)
    }

    fn log(&self, level: &str, message: &str) {
        tracing::debug!(target: "connection_import", level, message);
    }
}

fn secret_scope(query: &SecretQuery) -> (String, String) {
    let namespace = query
        .namespace
        .clone()
        .unwrap_or_else(|| secret_scope_part(&query.service, true));
    let key = query
        .key
        .clone()
        .unwrap_or_else(|| secret_scope_part(&query.account, false));
    (namespace, key)
}

fn secret_scope_part(value: &str, keep_dash: bool) -> String {
    value
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if keep_dash && (ch == '-' || ch == '_' || ch.is_whitespace()) {
                Some('-')
            } else if !keep_dash && (ch == '-' || ch == '_') {
                Some(ch)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn read_platform_secret(query: &SecretQuery) -> SecretResult {
    match security_framework::passwords::get_generic_password(&query.service, &query.account) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(|value| SecretResult::Included { value })
            .unwrap_or(SecretResult::Unsupported),
        Err(_) => SecretResult::Missing,
    }
}

#[cfg(not(target_os = "macos"))]
fn read_platform_secret(_query: &SecretQuery) -> SecretResult {
    SecretResult::Unsupported
}

fn current_platform() -> Platform {
    if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::Macos
    }
}

fn expand_connection_import_path(path: &str) -> PathBuf {
    expand_connection_import_path_with(path, |name| std::env::var_os(name))
}

fn expand_connection_import_path_with<F>(path: &str, env_var: F) -> PathBuf
where
    F: FnMut(&str) -> Option<OsString>,
{
    expand_tilde_or_env(path, env_var)
}

fn expand_tilde_or_env<F>(path: &str, mut env_var: F) -> PathBuf
where
    F: FnMut(&str) -> Option<OsString>,
{
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = env_var("HOME")
    {
        return join_expanded_path(home, rest);
    }
    if let Some((name, rest)) = windows_env_prefix(path)
        && let Some(value) = env_var(name)
    {
        return join_expanded_path(value, rest);
    }
    PathBuf::from(path)
}

fn windows_env_prefix(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix('%')?;
    let end = rest.find('%')?;
    let name = &rest[..end];
    if name.is_empty() {
        return None;
    }
    let tail = &rest[end + 1..];
    if !tail.is_empty() && !tail.starts_with('/') && !tail.starts_with('\\') {
        return None;
    }
    Some((name, tail))
}

fn join_expanded_path(base: OsString, rest: &str) -> PathBuf {
    let mut path = PathBuf::from(base);
    let rest = rest.trim_start_matches(['/', '\\']);
    if !rest.is_empty() {
        path.push(rest);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::expand_connection_import_path_with;

    #[test]
    fn expands_windows_style_env_prefix_for_connection_import_candidates() {
        let path = expand_connection_import_path_with("%APPDATA%/DBeaverData/workspace6", |name| {
            (name == "APPDATA").then(|| "C:\\Users\\me\\AppData\\Roaming".into())
        });

        assert_eq!(
            std::path::PathBuf::from("C:\\Users\\me\\AppData\\Roaming")
                .join("DBeaverData/workspace6"),
            path
        );
    }
}
