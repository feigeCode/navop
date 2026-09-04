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
    QuickCommand,
    Workspace,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImporterCapabilities {
    pub supports_scan: bool,
    pub supports_password_import: bool,
    pub supports_manual_file_pick: bool,
    #[serde(default)]
    pub supports_manual_directory_pick: bool,
    #[serde(default)]
    pub manual_file_pick_prompt: Option<String>,
    #[serde(default)]
    pub manual_directory_pick_prompt: Option<String>,
    pub supports_incremental_preview: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportScanReport {
    pub importer_id: String,
    pub availability: ImporterAvailability,
    pub discovered_files: Vec<DiscoveredFile>,
    pub warnings: Vec<ImportWarning>,
    #[serde(default)]
    pub discovered_workspace_paths: Vec<String>,
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
    #[serde(default)]
    pub quick_command: Option<QuickCommandImportRecord>,
    #[serde(default)]
    pub workspace: Option<WorkspaceImportRecord>,
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
                self.port_forwarding.is_some(),
                self.quick_command.is_some(),
                self.workspace.is_some()
            ),
            (ImportRecordKind::Database, true, false, false, false, false)
                | (ImportRecordKind::Ssh, false, true, false, false, false)
                | (
                    ImportRecordKind::PortForwarding,
                    false,
                    false,
                    true,
                    false,
                    false
                )
                | (
                    ImportRecordKind::QuickCommand,
                    false,
                    false,
                    false,
                    true,
                    false
                )
                | (
                    ImportRecordKind::Workspace,
                    false,
                    false,
                    false,
                    false,
                    true
                )
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
    /// TDengine 时序数据库
    TDengine,
    External { id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshImportRecord {
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    #[serde(default)]
    pub group_path: Option<String>,
    pub auth_method: SshImportAuthMethod,
    #[serde(default)]
    pub init_script: Option<String>,
    #[serde(default)]
    pub jump_server: Option<SshJumpServerImportRecord>,
    #[serde(default)]
    pub proxy: Option<SshProxyImportRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickCommandImportRecord {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub group_name: Option<String>,
    #[serde(default)]
    pub shortcut: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub sort_order: i32,
    #[serde(default)]
    pub connection_source_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceImportRecord {
    pub path: String,
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
            quick_command: None,
            workspace: None,
            password_status: PasswordImportStatus::Unsupported,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn legacy_scan_reports_default_discovered_workspace_paths() {
        let json = r#"{
            "importer_id":"legacy",
            "availability":"no_data",
            "discovered_files":[],
            "warnings":[]
        }"#;

        let report: ImportScanReport = serde_json::from_str(json).unwrap();

        assert!(report.discovered_workspace_paths.is_empty());
    }

    #[test]
    fn scan_reports_round_trip_discovered_workspace_paths() {
        let report = ImportScanReport {
            importer_id: "securecrt".to_string(),
            availability: ImporterAvailability::Available {
                estimated_count: Some(2),
            },
            discovered_files: Vec::new(),
            warnings: Vec::new(),
            discovered_workspace_paths: vec![
                "Production".to_string(),
                "Production/Staging".to_string(),
            ],
        };

        let json = serde_json::to_string(&report).unwrap();
        let decoded: ImportScanReport = serde_json::from_str(&json).unwrap();

        assert_eq!(report, decoded);
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
    fn validates_quick_command_payload_shape_and_json_round_trip() {
        let mut record = base_record(ImportRecordKind::QuickCommand);
        record.quick_command = Some(QuickCommandImportRecord {
            name: "Interfaces".to_string(),
            command: "show ip interface brief\r".to_string(),
            group_name: Some("Operations".to_string()),
            shortcut: None,
            description: Some("SecureCRT button".to_string()),
            sort_order: 3,
            connection_source_id: None,
        });

        assert_eq!(Ok(()), record.validate_shape());
        let json = serde_json::to_string(&record).unwrap();
        let decoded: ImportRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record, decoded);
    }

    #[test]
    fn rejects_quick_command_without_quick_command_payload() {
        let record = base_record(ImportRecordKind::QuickCommand);

        assert!(matches!(
            record.validate_shape(),
            Err(ImportProtocolError::MismatchedRecordPayload { .. })
        ));
    }

    #[test]
    fn rejects_quick_command_with_another_payload() {
        let mut record = base_record(ImportRecordKind::QuickCommand);
        record.quick_command = Some(QuickCommandImportRecord {
            name: "Interfaces".to_string(),
            command: "show ip interface brief\r".to_string(),
            group_name: None,
            shortcut: None,
            description: None,
            sort_order: 0,
            connection_source_id: None,
        });
        record.ssh = Some(SshImportRecord {
            name: "unexpected".to_string(),
            host: "example.test".to_string(),
            port: Some(22),
            username: "deploy".to_string(),
            group_path: None,
            auth_method: SshImportAuthMethod::Agent,
            init_script: None,
            jump_server: None,
            proxy: None,
        });

        assert!(matches!(
            record.validate_shape(),
            Err(ImportProtocolError::MismatchedRecordPayload { .. })
        ));
    }

    #[test]
    fn validates_workspace_payload_shape_and_json_round_trip() {
        let mut record = base_record(ImportRecordKind::Workspace);
        record.workspace = Some(WorkspaceImportRecord {
            path: "Production/Staging".to_string(),
        });

        assert_eq!(Ok(()), record.validate_shape());
        let json = serde_json::to_string(&record).unwrap();
        let decoded: ImportRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record, decoded);
    }

    #[test]
    fn old_records_without_workspace_payload_still_decode() {
        let json = r#"{
            "id":"legacy:ssh",
            "importer_id":"legacy",
            "source_label":"Legacy",
            "source_id":null,
            "kind":"ssh",
            "display_name":"Legacy SSH",
            "database":null,
            "ssh":{
                "name":"Legacy SSH",
                "host":"legacy.example.test",
                "port":22,
                "username":"deploy",
                "auth_method":"agent"
            },
            "port_forwarding":null,
            "quick_command":null,
            "password_status":"missing",
            "warnings":[]
        }"#;

        let decoded: ImportRecord = serde_json::from_str(json).unwrap();
        assert!(decoded.workspace.is_none());
        assert_eq!(Ok(()), decoded.validate_shape());
    }

    #[test]
    fn ssh_record_round_trips_init_script_proxy_jump_and_key_material() {
        let record = SshImportRecord {
            name: "prod".to_string(),
            host: "prod.example.test".to_string(),
            port: Some(22),
            username: "deploy".to_string(),
            group_path: Some("Production/API".to_string()),
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
    fn ssh_record_decodes_without_optional_group_path() {
        let json = r#"{
            "name": "prod",
            "host": "prod.example.test",
            "port": 22,
            "username": "deploy",
            "auth_method": "agent"
        }"#;

        let decoded: SshImportRecord = serde_json::from_str(json).unwrap();

        assert!(decoded.group_path.is_none());
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
