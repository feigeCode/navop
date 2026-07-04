use connection_import_protocol::{ImportRecord, ImportRecordKind, SshImportAuthMethod};
use one_core::storage::{ConnectionType, SshAuthMethod, SshParams, StoredConnection};

use super::connection_import_database_conversion::{
    database_config_duplicate_identity, database_duplicate_identity, to_database_connection,
};
use super::connection_import_draft::EditableImportDraft;

#[derive(Clone, Copy)]
pub(super) enum ConversionMode {
    StrictSave,
    EditorPrefill,
}

pub(crate) fn import_draft_to_stored_connection(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<StoredConnection, String> {
    import_draft_to_connection(draft, record, ConversionMode::StrictSave)
}

pub(crate) fn import_draft_to_editor_connection(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<StoredConnection, String> {
    import_draft_to_connection(draft, record, ConversionMode::EditorPrefill)
}

fn import_draft_to_connection(
    draft: &EditableImportDraft,
    record: &ImportRecord,
    mode: ConversionMode,
) -> Result<StoredConnection, String> {
    match record.kind {
        ImportRecordKind::Database => to_database_connection(draft, record, mode),
        ImportRecordKind::Ssh => to_ssh_connection(draft, record),
        ImportRecordKind::PortForwarding => Err("暂不支持直接保存端口转发导入记录".to_string()),
    }
}

pub(crate) fn import_draft_duplicate_identity(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<String, String> {
    match record.kind {
        ImportRecordKind::Database => database_duplicate_identity(draft, record),
        ImportRecordKind::Ssh => ssh_duplicate_identity(draft),
        ImportRecordKind::PortForwarding => Err("暂不支持端口转发导入记录重复检测".to_string()),
    }
}

pub(crate) fn stored_connection_duplicate_identity(
    connection: &StoredConnection,
) -> Result<Option<String>, String> {
    match connection.connection_type {
        ConnectionType::SshSftp => connection
            .to_ssh_params()
            .map(|params| Some(ssh_identity(&params.host, params.port, &params.username)))
            .map_err(|error| error.to_string()),
        ConnectionType::Database => connection
            .to_db_connection()
            .map(|config| Some(database_config_duplicate_identity(&config)))
            .map_err(|error| error.to_string()),
        _ => Ok(None),
    }
}

pub(crate) fn ssh_auth_edit_values(auth_method: &SshImportAuthMethod) -> (String, String) {
    match auth_method {
        SshImportAuthMethod::Password { password } => {
            (password.clone().unwrap_or_default(), String::new())
        }
        SshImportAuthMethod::PrivateKey { key_path, .. } => (String::new(), key_path.clone()),
        SshImportAuthMethod::PrivateKeyMaterial { .. } => (String::new(), String::new()),
        SshImportAuthMethod::Agent | SshImportAuthMethod::AutoPublicKey => {
            (String::new(), String::new())
        }
    }
}

fn to_ssh_connection(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<StoredConnection, String> {
    let imported = record
        .ssh
        .as_ref()
        .ok_or_else(|| "SSH 导入记录缺少 SSH 配置".to_string())?;
    let name = required_text(&draft.name, "连接名称")?;
    let params = SshParams {
        host: required_text(&draft.host, "主机")?,
        port: required_port(&draft.port)?,
        username: draft.username.trim().to_string(),
        auth_method: edited_ssh_auth_method(draft, &imported.auth_method)?,
        connect_timeout: None,
        keepalive_interval: None,
        keepalive_max: None,
        default_directory: None,
        init_script: None,
        disable_shell_integration: None,
        jump_server: None,
        proxy: None,
    };
    Ok(StoredConnection::new_ssh(name, params, None))
}

fn ssh_duplicate_identity(draft: &EditableImportDraft) -> Result<String, String> {
    let port = required_port(&draft.port)?;
    Ok(ssh_identity(&draft.host, port, &draft.username))
}

fn edited_ssh_auth_method(
    draft: &EditableImportDraft,
    auth_method: &SshImportAuthMethod,
) -> Result<SshAuthMethod, String> {
    match auth_method {
        SshImportAuthMethod::Password { .. } => Ok(SshAuthMethod::Password {
            password: optional_text(&draft.password).unwrap_or_default(),
        }),
        SshImportAuthMethod::PrivateKey { passphrase, .. } => Ok(SshAuthMethod::PrivateKey {
            key_path: draft.private_key_path.trim().to_string(),
            passphrase: passphrase.clone(),
        }),
        SshImportAuthMethod::PrivateKeyMaterial { passphrase, .. } => {
            let key_path = draft.private_key_path.trim();
            if key_path.is_empty() {
                return Err("私钥内容导入需要先编辑为私钥路径".to_string());
            }
            Ok(SshAuthMethod::PrivateKey {
                key_path: key_path.to_string(),
                passphrase: passphrase.clone(),
            })
        }
        SshImportAuthMethod::Agent => Ok(SshAuthMethod::Agent),
        SshImportAuthMethod::AutoPublicKey => Ok(SshAuthMethod::AutoPublicKey),
    }
}

fn ssh_identity(host: &str, port: u16, username: &str) -> String {
    format!(
        "ssh:{}:{}:{}",
        normalize_identity_part(host),
        port,
        normalize_identity_part(username)
    )
}

pub(super) fn normalize_identity_part(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(super) fn required_text(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{}不能为空", label));
    }
    Ok(trimmed.to_string())
}

pub(super) fn optional_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(super) fn optional_port(value: &str) -> Result<Option<u16>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse::<u16>()
        .map(Some)
        .map_err(|_| "端口必须是 1-65535".to_string())
}

fn required_port(value: &str) -> Result<u16, String> {
    optional_port(value)?.ok_or_else(|| "端口不能为空".to_string())
}
