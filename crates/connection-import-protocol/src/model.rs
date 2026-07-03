use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImporterDescriptor {
    pub id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub vendor: Option<String>,
    pub supported_platforms: Vec<Platform>,
    pub output_kinds: Vec<ImportRecordKind>,
    pub capabilities: ImporterCapabilities,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Macos,
    Windows,
    Linux,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportRecordKind {
    Database,
    Ssh,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImporterCapabilities {
    pub supports_scan: bool,
    pub supports_password_import: bool,
    pub supports_manual_file_pick: bool,
    pub supports_incremental_preview: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportScanReport {
    pub importer_id: String,
    pub availability: ImporterAvailability,
    pub discovered_files: Vec<DiscoveredFile>,
    pub warnings: Vec<ImportWarning>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImporterAvailability {
    Available { estimated_count: Option<u32> },
    Installed,
    NotInstalled,
    NoData,
    PermissionRequired,
    UnsupportedPlatform,
    Error { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredFile {
    pub candidate_id: String,
    pub display_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportWarning {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRecord {
    pub id: String,
    pub importer_id: String,
    pub source_label: String,
    pub kind: ImportRecordKind,
    pub display_name: String,
    pub database: Option<DatabaseImportRecord>,
    pub ssh: Option<SshImportRecord>,
    pub password_status: PasswordImportStatus,
    pub warnings: Vec<ImportWarning>,
}

impl ImportRecord {
    pub fn validate_shape(&self) -> Result<(), ImportProtocolError> {
        let matches_payload = matches!(
            (self.kind, self.database.is_some(), self.ssh.is_some()),
            (ImportRecordKind::Database, true, false) | (ImportRecordKind::Ssh, false, true)
        );
        if matches_payload {
            Ok(())
        } else {
            Err(ImportProtocolError::MismatchedRecordPayload {
                id: self.id.clone(),
                kind: self.kind,
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseImportRecord {
    pub database_type: ImportDatabaseType,
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub password: Option<String>,
    pub database: Option<String>,
    pub extra_params: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportDatabaseType {
    MySql,
    PostgreSql,
    Sqlite,
    DuckDb,
    SqlServer,
    Oracle,
    ClickHouse,
    External { id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshImportRecord {
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub auth_method: SshImportAuthMethod,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshImportAuthMethod {
    Password {
        password: Option<String>,
    },
    PrivateKey {
        key_path: String,
        passphrase: Option<String>,
    },
    Agent,
    AutoPublicKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordImportStatus {
    Included,
    Missing,
    Unsupported,
    PermissionDenied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportOptions {
    pub include_passwords: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateFile {
    pub id: String,
    pub platform: Option<Platform>,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub candidate_id: String,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretQuery {
    pub service: String,
    pub account: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretResult {
    Included { value: String },
    Missing,
    PermissionDenied,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImportProtocolError {
    #[error("import record {id} has payload that does not match kind {kind:?}")]
    MismatchedRecordPayload { id: String, kind: ImportRecordKind },
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HostAccessError {
    #[error("candidate id not declared: {0}")]
    UndeclaredCandidate(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("host io failed: {0}")]
    Io(String),
}
