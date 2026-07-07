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
    PortForwarding,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImporterCapabilities {
    pub supports_scan: bool,
    pub supports_password_import: bool,
    pub supports_manual_file_pick: bool,
    #[serde(default)]
    pub manual_file_pick_prompt: Option<String>,
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
    #[serde(default)]
    pub source_id: Option<String>,
    pub kind: ImportRecordKind,
    pub display_name: String,
    pub database: Option<DatabaseImportRecord>,
    pub ssh: Option<SshImportRecord>,
    #[serde(default)]
    pub port_forwarding: Option<PortForwardingImportRecord>,
    pub password_status: PasswordImportStatus,
    pub warnings: Vec<ImportWarning>,
}

impl ImportRecord {
    pub fn validate_shape(&self) -> Result<(), ImportProtocolError> {
        let matches_payload = matches!(
            (
                self.kind,
                self.database.is_some(),
                self.ssh.is_some(),
                self.port_forwarding.is_some()
            ),
            (ImportRecordKind::Database, true, false, false)
                | (ImportRecordKind::Ssh, false, true, false)
                | (ImportRecordKind::PortForwarding, false, false, true)
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
    #[serde(default)]
    pub init_script: Option<String>,
    #[serde(default)]
    pub jump_server: Option<SshJumpServerImportRecord>,
    #[serde(default)]
    pub proxy: Option<SshProxyImportRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshJumpServerImportRecord {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: SshImportAuthMethod,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshProxyImportRecord {
    pub kind: SshProxyImportKind,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshProxyImportKind {
    Socks5,
    Http,
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
    PrivateKeyMaterial {
        private_key: Option<String>,
        passphrase: Option<String>,
        file_name_hint: Option<String>,
    },
    Agent,
    AutoPublicKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortForwardingImportRecord {
    pub name: String,
    pub ssh_source_id: String,
    pub kind: PortForwardingImportKind,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortForwardingImportKind {
    Local,
    Dynamic,
    Remote,
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
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn base_record(kind: ImportRecordKind) -> ImportRecord {
        ImportRecord {
            id: "termius:record".to_string(),
            importer_id: "termius".to_string(),
            source_label: "Termius".to_string(),
            source_id: Some("host-local-1".to_string()),
            kind,
            display_name: "record".to_string(),
            database: None,
            ssh: None,
            port_forwarding: None,
            password_status: PasswordImportStatus::Unsupported,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn validates_port_forwarding_payload_shape() {
        let mut record = base_record(ImportRecordKind::PortForwarding);
        record.port_forwarding = Some(PortForwardingImportRecord {
            name: "db tunnel".to_string(),
            ssh_source_id: "termius:host:1".to_string(),
            kind: PortForwardingImportKind::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 15432,
            target_host: "db.internal".to_string(),
            target_port: 5432,
        });

        assert_eq!(Ok(()), record.validate_shape());
    }

    #[test]
    fn rejects_port_forwarding_without_port_forwarding_payload() {
        let record = base_record(ImportRecordKind::PortForwarding);

        assert!(matches!(
            record.validate_shape(),
            Err(ImportProtocolError::MismatchedRecordPayload { .. })
        ));
    }

    #[test]
    fn ssh_record_round_trips_init_script_proxy_jump_and_key_material() {
        let record = SshImportRecord {
            name: "prod".to_string(),
            host: "prod.example.test".to_string(),
            port: Some(22),
            username: "deploy".to_string(),
            auth_method: SshImportAuthMethod::PrivateKeyMaterial {
                private_key: Some("-----BEGIN OPENSSH PRIVATE KEY-----\nfixture\n".to_string()),
                passphrase: Some("secret".to_string()),
                file_name_hint: Some("key-local-1".to_string()),
            },
            init_script: Some("echo ready".to_string()),
            jump_server: Some(SshJumpServerImportRecord {
                host: "jump.example.test".to_string(),
                port: 22,
                username: "jump".to_string(),
                auth_method: SshImportAuthMethod::Agent,
            }),
            proxy: Some(SshProxyImportRecord {
                kind: SshProxyImportKind::Socks5,
                host: "proxy.example.test".to_string(),
                port: 1080,
                username: Some("proxy-user".to_string()),
                password: None,
            }),
        };

        let json = serde_json::to_string(&record).unwrap();
        let decoded: SshImportRecord = serde_json::from_str(&json).unwrap();

        assert_eq!(record, decoded);
    }

    #[test]
    fn secret_query_round_trips_permission_scope() {
        let query = SecretQuery {
            service: "Termius".to_string(),
            account: "localKey".to_string(),
            namespace: Some("termius".to_string()),
            key: Some("localkey".to_string()),
        };

        let json = serde_json::to_string(&query).unwrap();
        let decoded: SecretQuery = serde_json::from_str(&json).unwrap();

        assert_eq!(query, decoded);
    }
}
