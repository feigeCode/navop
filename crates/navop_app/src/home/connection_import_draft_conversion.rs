use connection_import_protocol::{
    ImportRecord, ImportRecordKind, SshImportAuthMethod, SshProxyImportKind,
};
use one_core::storage::{
    ConnectionType, JumpServerConfig, ProxyConfig, ProxyType, QuickCommand, SshAuthMethod,
    SshParams, StoredConnection, TelnetLoginStep, TelnetParams,
};
use rust_i18n::t;

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

pub(crate) fn import_draft_to_quick_command(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<QuickCommand, String> {
    if record.kind != ImportRecordKind::QuickCommand {
        return Err(t!("Home.ConnectionImport.quick_command_config_missing").to_string());
    }
    let name = required_text(
        &draft.name,
        t!("Home.ConnectionImport.field_connection_name").as_ref(),
    )?;
    let command = required_text(
        &draft.quick_command_command,
        t!("Home.ConnectionImport.field_quick_command").as_ref(),
    )?;
    let mut quick_command = QuickCommand::new(command);
    quick_command.name = Some(name);
    quick_command.group_name = optional_text(&draft.quick_command_group_name);
    quick_command.shortcut = optional_text(&draft.quick_command_shortcut);
    quick_command.description = optional_text(&draft.quick_command_description);
    quick_command.sort_order = record
        .quick_command
        .as_ref()
        .map(|imported| imported.sort_order)
        .unwrap_or_default();
    Ok(quick_command)
}

fn import_draft_to_connection(
    draft: &EditableImportDraft,
    record: &ImportRecord,
    mode: ConversionMode,
) -> Result<StoredConnection, String> {
    match record.kind {
        ImportRecordKind::Database => to_database_connection(draft, record, mode),
        ImportRecordKind::Ssh => to_ssh_connection(draft, record),
        ImportRecordKind::Telnet => to_telnet_connection(draft, record),
        ImportRecordKind::QuickCommand => {
            Err(t!("Home.ConnectionImport.quick_command_editor_unsupported").to_string())
        }
        ImportRecordKind::PortForwarding => {
            Err(t!("Home.ConnectionImport.port_forwarding_save_unsupported").to_string())
        }
        ImportRecordKind::Workspace => {
            Err(t!("Home.ConnectionImport.workspace_save_unsupported").to_string())
        }
    }
}

pub(crate) fn import_draft_duplicate_identity(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<String, String> {
    match record.kind {
        ImportRecordKind::Database => database_duplicate_identity(draft, record),
        ImportRecordKind::Ssh => ssh_duplicate_identity(draft),
        ImportRecordKind::Telnet => telnet_duplicate_identity(draft),
        ImportRecordKind::QuickCommand => quick_command_duplicate_identity(draft),
        ImportRecordKind::PortForwarding => {
            Err(t!("Home.ConnectionImport.port_forwarding_duplicate_unsupported").to_string())
        }
        ImportRecordKind::Workspace => {
            Err(t!("Home.ConnectionImport.workspace_save_unsupported").to_string())
        }
    }
}

pub(crate) fn quick_command_duplicate_identity(
    draft: &EditableImportDraft,
) -> Result<String, String> {
    let name = required_text(
        &draft.name,
        t!("Home.ConnectionImport.field_connection_name").as_ref(),
    )?;
    let command = required_text(
        &draft.quick_command_command,
        t!("Home.ConnectionImport.field_quick_command").as_ref(),
    )?;
    Ok(format!(
        "quick-command:{}:{}:{}",
        normalize_identity_part(&draft.quick_command_group_name),
        normalize_identity_part(&name),
        command.trim()
    ))
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
        ConnectionType::Telnet => connection
            .to_telnet_params()
            .map(|params| Some(telnet_identity(&params.host, params.port)))
            .map_err(|error| error.to_string()),
        ConnectionType::Redis => connection
            .to_redis_params()
            .map(|params| {
                Some(redis_identity(
                    &params.host,
                    params.port,
                    params.username.as_deref().unwrap_or_default(),
                    params.db_index,
                ))
            })
            .map_err(|error| error.to_string()),
        ConnectionType::MongoDB => connection
            .to_mongodb_params()
            .map(|params| {
                Some(mongodb_identity(
                    &params.host,
                    params.port.unwrap_or(27017),
                    params.username.as_deref().unwrap_or_default(),
                    params.database.as_deref().unwrap_or_default(),
                ))
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

fn to_ssh_connection(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<StoredConnection, String> {
    let imported = record
        .ssh
        .as_ref()
        .ok_or_else(|| t!("Home.ConnectionImport.ssh_config_missing").to_string())?;
    let name = required_text(
        &draft.name,
        t!("Home.ConnectionImport.field_connection_name").as_ref(),
    )?;
    let params = SshParams { otp: None,
        remote_file: None,
        sftp_default_directory: None,
        disabled_jump_server: None,
        sftp_account: None,
        host: required_text(&draft.host, t!("Home.ConnectionImport.field_host").as_ref())?,
        port: required_port(&draft.port)?,
        username: draft.username.trim().to_string(),
        auth_method: edited_ssh_auth_method(draft, &imported.auth_method)?,
        credential_reference: None,
        prompt_username: None,
        prompt_password: None,
        // 导入协议不携带 keyboard-interactive 信息：迁移源里的「支持多种认证方式」只是服务器
        // 返回的方法列表，不等于需要二次认证。此字段留空会落到全局默认 `true`，使每个迁移进来的
        // 连接都走 keyboard-interactive 优先；而受限设备（交换机/防火墙）上这条路径可能直接被
        // 设备捆断。因此迁移时显式关闭，需要二次认证的设备由用户在连接设置里手动开启。
        keyboard_interactive: Some(false),
        terminal_encoding: Default::default(),
        terminal_type: Default::default(),
        connect_timeout: None,
        keepalive_interval: None,
        keepalive_max: None,
        default_directory: None,
        init_script: imported.init_script.clone(),
        disable_shell_integration: None,
        x11_forwarding: None,
        allow_legacy_algorithms: None,
        forward_agent: None,
        agent_forward_key: None,
        jump_server: imported
            .jump_server
            .as_ref()
            .map(|jump| -> Result<JumpServerConfig, String> {
                Ok(JumpServerConfig {
                    host: jump.host.clone(),
                    port: jump.port,
                    username: jump.username.clone(),
                    auth_method: imported_ssh_auth_method(&jump.auth_method)?,
                    credential_reference: None,
                })
            })
            .transpose()?,
        proxy: imported.proxy.as_ref().map(|proxy| ProxyConfig {
            proxy_type: match proxy.kind {
                SshProxyImportKind::Socks5 => ProxyType::Socks5,
                SshProxyImportKind::Http => ProxyType::Http,
            },
            host: proxy.host.clone(),
            port: proxy.port,
            username: proxy.username.clone(),
            password: proxy.password.clone(),
            credential_reference: None,
        }),
        os_id: None,
        icon: None,
        icon_file_path: None,
        account_expect: Default::default(),
    };
    Ok(StoredConnection::new_ssh(name, params, None))
}

/// Telnet 默认端口。
const DEFAULT_TELNET_PORT: u16 = 23;

/// 与产品内置的 Telnet 登录提示保持一致：账号与口令两步 expect 都来自
/// `home_tab::connection_open` 里临时凭据流程使用的规范正则。
const TELNET_USERNAME_PROMPT: &str = r"(?i)(?:login|username|user\s*name|account)\s*[:>]\s*$";
const TELNET_PASSWORD_PROMPT: &str = r"(?i)(?:password|passwd|passcode)\s*[:>]\s*$";

fn to_telnet_connection(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<StoredConnection, String> {
    if record.telnet.is_none() {
        return Err(t!("Home.ConnectionImport.telnet_config_missing").to_string());
    }
    let name = required_text(
        &draft.name,
        t!("Home.ConnectionImport.field_connection_name").as_ref(),
    )?;
    let mut params = TelnetParams {
        host: required_text(&draft.host, t!("Home.ConnectionImport.field_host").as_ref())?,
        port: optional_port(&draft.port)?.unwrap_or(DEFAULT_TELNET_PORT),
        credential_reference: None,
        prompt_username: None,
        prompt_password: None,
        backspace_code: Default::default(),
        // 登录脚本直接带上迁移来的账号与口令：`login_script.send` 和密码一样按敏感字段
        // 落盘加密。send 为空时运行时会弹出临时凭据输入，不会把空值发到设备上。
        login_script: telnet_login_script(
            optional_text(&draft.username).as_deref(),
            optional_text(&draft.password).as_deref(),
        ),
    };
    let (prompt_username, prompt_password) = params.login_credential_prompt_fields();
    params.prompt_username = prompt_username.then_some(true);
    params.prompt_password = prompt_password.then_some(true);
    Ok(StoredConnection::new_telnet(name, params, None))
}

fn telnet_login_script(username: Option<&str>, password: Option<&str>) -> Vec<TelnetLoginStep> {
    vec![
        TelnetLoginStep {
            expect: TELNET_USERNAME_PROMPT.to_string(),
            send: username.unwrap_or_default().to_string(),
        },
        TelnetLoginStep {
            expect: TELNET_PASSWORD_PROMPT.to_string(),
            send: password.unwrap_or_default().to_string(),
        },
    ]
}

fn telnet_duplicate_identity(draft: &EditableImportDraft) -> Result<String, String> {
    let port = optional_port(&draft.port)?.unwrap_or(DEFAULT_TELNET_PORT);
    Ok(telnet_identity(&draft.host, port))
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
        SshImportAuthMethod::PrivateKeyMaterial {
            private_key,
            passphrase,
            ..
        } => {
            let private_key = private_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| t!("Home.ConnectionImport.private_key_missing").to_string())?;
            Ok(SshAuthMethod::PrivateKeyContent {
                private_key: private_key.to_string(),
                passphrase: passphrase.clone(),
            })
        }
        SshImportAuthMethod::Agent => Ok(SshAuthMethod::Agent),
        SshImportAuthMethod::AutoPublicKey => Ok(SshAuthMethod::AutoPublicKey),
    }
}

fn imported_ssh_auth_method(auth_method: &SshImportAuthMethod) -> Result<SshAuthMethod, String> {
    match auth_method {
        SshImportAuthMethod::Password { password } => Ok(SshAuthMethod::Password {
            password: password.clone().unwrap_or_default(),
        }),
        SshImportAuthMethod::PrivateKey {
            key_path,
            passphrase,
        } => Ok(SshAuthMethod::PrivateKey {
            key_path: key_path.clone(),
            passphrase: passphrase.clone(),
        }),
        SshImportAuthMethod::PrivateKeyMaterial {
            private_key,
            passphrase,
            ..
        } => {
            let private_key = private_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| t!("Home.ConnectionImport.private_key_missing").to_string())?;
            Ok(SshAuthMethod::PrivateKeyContent {
                private_key: private_key.to_string(),
                passphrase: passphrase.clone(),
            })
        }
        SshImportAuthMethod::Agent => Ok(SshAuthMethod::Agent),
        SshImportAuthMethod::AutoPublicKey => Ok(SshAuthMethod::AutoPublicKey),
    }
}

/// Telnet 连接参数没有独立用户名字段（登录名在 `login_script.send` 里，口令缺失时还会为空），
/// 因此去重标识只按主机与端口计算，避免导入结果与已存连接永远对不上。
pub(super) fn telnet_identity(host: &str, port: u16) -> String {
    format!("telnet:{}:{}", normalize_identity_part(host), port)
}

fn ssh_identity(host: &str, port: u16, username: &str) -> String {
    format!(
        "ssh:{}:{}:{}",
        normalize_identity_part(host),
        port,
        normalize_identity_part(username)
    )
}

pub(super) fn redis_identity(host: &str, port: u16, username: &str, db_index: u8) -> String {
    format!(
        "redis:{}:{}:{}:{}",
        normalize_identity_part(host),
        port,
        normalize_identity_part(username),
        db_index
    )
}

pub(super) fn mongodb_identity(host: &str, port: u16, username: &str, database: &str) -> String {
    format!(
        "mongodb:{}:{}:{}:{}",
        normalize_identity_part(host),
        port,
        normalize_identity_part(username),
        normalize_identity_part(database)
    )
}

pub(super) fn normalize_identity_part(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(super) fn required_text(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(t!("Home.ConnectionImport.field_required", field = label).to_string());
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
        .map_err(|_| t!("Home.ConnectionImport.invalid_port").to_string())
}

fn required_port(value: &str) -> Result<u16, String> {
    optional_port(value)?.ok_or_else(|| t!("Home.ConnectionImport.port_required").to_string())
}
