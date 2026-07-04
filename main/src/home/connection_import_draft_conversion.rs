use std::collections::HashMap;

use connection_import_protocol::{
    DatabaseImportRecord, ImportDatabaseType, ImportRecord, ImportRecordKind, SshImportAuthMethod,
};
use one_core::storage::{
    ConnectionType, DatabaseType, DbConnectionConfig, SshAuthMethod, SshParams, StoredConnection,
};

use super::connection_import_draft::EditableImportDraft;

struct DatabaseIdentity<'a> {
    database_type: &'a DatabaseType,
    host: &'a str,
    port: u16,
    username: &'a str,
    database: &'a str,
}

pub(crate) fn import_draft_to_stored_connection(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<StoredConnection, String> {
    match record.kind {
        ImportRecordKind::Database => to_database_connection(draft, record),
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
            .map(|config| {
                Some(database_identity(DatabaseIdentity {
                    database_type: &config.database_type,
                    host: &config.host,
                    port: config.port,
                    username: &config.username,
                    database: config.database.as_deref().unwrap_or_default(),
                }))
            })
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

fn to_database_connection(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<StoredConnection, String> {
    let imported = record
        .database
        .as_ref()
        .ok_or_else(|| "数据库导入记录缺少数据库配置".to_string())?;
    let name = required_text(&draft.name, "连接名称")?;
    let database_type = storage_database_type(&imported.database_type);
    let port = optional_port(&draft.port)?
        .or_else(|| default_database_port(&database_type))
        .unwrap_or_default();
    let config = DbConnectionConfig {
        id: String::new(),
        database_type,
        name: name.clone(),
        host: required_text(&draft.host, "主机")?,
        port,
        username: draft.username.trim().to_string(),
        password: optional_text(&draft.password).unwrap_or_default(),
        database: optional_text(&draft.database),
        service_name: None,
        sid: None,
        workspace_id: None,
        extra_params: extra_params(imported),
    };
    Ok(StoredConnection::from_db_connection(config))
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

fn database_duplicate_identity(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<String, String> {
    let imported = record
        .database
        .as_ref()
        .ok_or_else(|| "数据库导入记录缺少数据库配置".to_string())?;
    let database_type = storage_database_type(&imported.database_type);
    let port = optional_port(&draft.port)?
        .or_else(|| default_database_port(&database_type))
        .unwrap_or_default();
    Ok(database_identity(DatabaseIdentity {
        database_type: &database_type,
        host: &draft.host,
        port,
        username: &draft.username,
        database: draft.database.as_str(),
    }))
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

fn storage_database_type(database_type: &ImportDatabaseType) -> DatabaseType {
    match database_type {
        ImportDatabaseType::MySql => DatabaseType::MySQL,
        ImportDatabaseType::PostgreSql => DatabaseType::PostgreSQL,
        ImportDatabaseType::Sqlite => DatabaseType::SQLite,
        ImportDatabaseType::DuckDb => DatabaseType::DuckDB,
        ImportDatabaseType::SqlServer => DatabaseType::MSSQL,
        ImportDatabaseType::Oracle => DatabaseType::Oracle,
        ImportDatabaseType::ClickHouse => DatabaseType::ClickHouse,
        ImportDatabaseType::External { id } => DatabaseType::External {
            driver_id: id.clone(),
        },
    }
}

fn database_identity(identity: DatabaseIdentity<'_>) -> String {
    format!(
        "db:{}:{}:{}:{}:{}",
        database_type_identity(identity.database_type),
        normalize_identity_part(identity.host),
        identity.port,
        normalize_identity_part(identity.username),
        normalize_identity_part(identity.database)
    )
}

fn database_type_identity(database_type: &DatabaseType) -> String {
    match database_type {
        DatabaseType::MySQL => "mysql".to_string(),
        DatabaseType::PostgreSQL => "postgresql".to_string(),
        DatabaseType::SQLite => "sqlite".to_string(),
        DatabaseType::DuckDB => "duckdb".to_string(),
        DatabaseType::MSSQL => "sqlserver".to_string(),
        DatabaseType::Oracle => "oracle".to_string(),
        DatabaseType::ClickHouse => "clickhouse".to_string(),
        DatabaseType::External { driver_id } => format!("external:{driver_id}"),
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

fn normalize_identity_part(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn default_database_port(database_type: &DatabaseType) -> Option<u16> {
    match database_type {
        DatabaseType::MySQL => Some(3306),
        DatabaseType::PostgreSQL => Some(5432),
        DatabaseType::MSSQL => Some(1433),
        DatabaseType::Oracle => Some(1521),
        DatabaseType::ClickHouse => Some(8123),
        DatabaseType::SQLite | DatabaseType::DuckDB | DatabaseType::External { .. } => None,
    }
}

fn extra_params(imported: &DatabaseImportRecord) -> HashMap<String, String> {
    imported
        .extra_params
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn required_text(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{}不能为空", label));
    }
    Ok(trimmed.to_string())
}

fn optional_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn optional_port(value: &str) -> Result<Option<u16>, String> {
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
