use one_core::storage::{
    ConnectionType, DbConnectionConfig, MongoDBParams, PortForwardingKind, PortForwardingParams,
    RedisMode, RedisParams, RemoteDesktopParams, SerialParams, SshAuthMethod, SshParams,
    StoredConnection,
};
use serde_json::Value;

use crate::_rust_i18n_translate;

pub(super) fn connection_share_text(connection: &StoredConnection) -> Option<String> {
    connection_share_text_for_locale(connection, rust_i18n::locale().as_ref())
}

pub(super) fn connection_share_text_for_locale(
    connection: &StoredConnection,
    locale: &str,
) -> Option<String> {
    let fields = match connection.connection_type {
        ConnectionType::Database => database_fields(locale, connection.to_db_connection().ok()?),
        ConnectionType::SshSftp => ssh_fields(locale, connection.to_ssh_params().ok()?),
        ConnectionType::Redis => redis_fields(locale, connection.to_redis_params().ok()?),
        ConnectionType::MongoDB => mongodb_fields(locale, connection.to_mongodb_params().ok()?),
        ConnectionType::Mqtt => mqtt_fields(locale, connection.to_mqtt_params().ok()?),
        ConnectionType::Serial => serial_fields(locale, connection.to_serial_params().ok()?),
        ConnectionType::Telnet => telnet_fields(locale, connection.to_telnet_params().ok()?),
        ConnectionType::PortForwarding => {
            forwarding_fields(locale, connection.to_port_forwarding_params().ok()?)
        }
        ConnectionType::Rdp | ConnectionType::Vnc => {
            remote_desktop_fields(locale, connection.to_remote_desktop_params().ok()?)
        }
        ConnectionType::Extension => extension_fields(connection),
        ConnectionType::All => return None,
    };
    Some(render_share_template(connection, fields, locale))
}

pub(crate) fn connection_full_info_text(connection: &StoredConnection) -> Option<String> {
    connection_full_info_text_for_locale(connection, rust_i18n::locale().as_ref())
}

pub(crate) fn connection_full_info_text_for_locale(
    connection: &StoredConnection,
    locale: &str,
) -> Option<String> {
    let mut params = serde_json::from_str::<Value>(&connection.params_for_storage()).ok()?;
    redact_embedded_private_keys(&mut params, locale);
    redact_telnet_login_script_sends(&mut params, locale);
    let params = serde_json::to_string_pretty(&params).ok()?;
    let separator = tr(locale, "Connection.Share.separator");
    let mut lines = vec![
        tr(locale, "Connection.FullInfo.title"),
        full_info_line(locale, "name", &connection.name, &separator),
        full_info_line(
            locale,
            "type",
            &tr(locale, connection_type_key(connection.connection_type)),
            &separator,
        ),
    ];
    if let Some(remark) = connection
        .remark
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(full_info_line(locale, "remark", remark, &separator));
    }
    lines.push(format!(
        "{}{}\n{}",
        tr(locale, "Connection.FullInfo.parameters"),
        separator.trim_end(),
        params
    ));
    Some(lines.join("\n"))
}

fn render_share_template(
    connection: &StoredConnection,
    fields: Vec<(&str, String)>,
    locale: &str,
) -> String {
    let separator = tr(locale, "Connection.Share.separator");
    let mut lines = vec![
        tr(locale, "Connection.Share.title"),
        share_line(locale, "name", &connection.name, &separator),
        share_line(
            locale,
            "type",
            &tr(locale, connection_type_key(connection.connection_type)),
            &separator,
        ),
    ];
    lines.extend(
        fields
            .into_iter()
            .filter(|(_, value)| !value.trim().is_empty())
            .map(|(key, value)| share_line(locale, key, &value, &separator)),
    );
    if let Some(remark) = connection
        .remark
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(share_line(locale, "remark", remark, &separator));
    }
    lines.push(share_line(
        locale,
        "credentials",
        &tr(locale, "Connection.Share.credentials_hint"),
        &separator,
    ));
    lines.join("\n")
}

fn database_fields(locale: &str, params: DbConnectionConfig) -> Vec<(&'static str, String)> {
    vec![
        (
            "database_type",
            database_type_label(locale, &params.database_type),
        ),
        ("host", params.host),
        ("port", params.port.to_string()),
        ("username", params.username),
        ("database", params.database.unwrap_or_default()),
        ("service_name", params.service_name.unwrap_or_default()),
        ("sid", params.sid.unwrap_or_default()),
    ]
}

fn ssh_fields(locale: &str, params: SshParams) -> Vec<(&'static str, String)> {
    vec![
        ("host", params.host),
        ("port", params.port.to_string()),
        ("username", params.username),
        ("auth_method", tr(locale, ssh_auth_key(&params.auth_method))),
        (
            "default_directory",
            params.default_directory.unwrap_or_default(),
        ),
    ]
}

fn redis_fields(locale: &str, params: RedisParams) -> Vec<(&'static str, String)> {
    vec![
        ("mode", tr(locale, redis_mode_key(&params.mode))),
        ("host", params.host),
        ("port", params.port.to_string()),
        ("username", params.username.unwrap_or_default()),
        ("database_index", params.db_index.to_string()),
        ("tls", tr(locale, yes_no_key(params.use_tls))),
    ]
}

fn mongodb_fields(locale: &str, params: MongoDBParams) -> Vec<(&'static str, String)> {
    vec![
        ("host", params.host),
        (
            "port",
            params.port.map(|port| port.to_string()).unwrap_or_default(),
        ),
        ("database", params.database.unwrap_or_default()),
        ("username", params.username.unwrap_or_default()),
        ("auth_database", params.auth_source.unwrap_or_default()),
        ("replica_set", params.replica_set.unwrap_or_default()),
        ("tls", tr(locale, yes_no_key(params.use_tls))),
    ]
}

fn mqtt_fields(locale: &str, params: one_core::storage::MqttParams) -> Vec<(&'static str, String)> {
    vec![
        ("host", params.host),
        ("port", params.port.to_string()),
        ("client_id", params.client_id),
        ("username", params.username.unwrap_or_default()),
        ("tls", tr(locale, yes_no_key(params.use_tls))),
        (
            "keep_alive",
            params
                .keep_alive
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
    ]
}

fn serial_fields(locale: &str, params: SerialParams) -> Vec<(&'static str, String)> {
    vec![
        ("serial_port", params.port_name),
        ("baud_rate", params.baud_rate.to_string()),
        ("data_bits", params.data_bits.to_string()),
        ("stop_bits", params.stop_bits.to_string()),
        (
            "parity",
            tr(
                locale,
                &format!(
                    "Connection.Share.parity_{}",
                    params.parity.label().to_lowercase()
                ),
            ),
        ),
        (
            "flow_control",
            tr(locale, serial_flow_control_key(params.flow_control.label())),
        ),
    ]
}

fn telnet_fields(
    _locale: &str,
    params: one_core::storage::TelnetParams,
) -> Vec<(&'static str, String)> {
    vec![("host", params.host), ("port", params.port.to_string())]
}

fn forwarding_fields(locale: &str, params: PortForwardingParams) -> Vec<(&'static str, String)> {
    let mut fields = vec![
        ("mode", tr(locale, forwarding_mode_key(params.kind))),
        (
            "listen_address",
            format!("{}:{}", params.bind_host, params.bind_port),
        ),
    ];
    if params.kind != PortForwardingKind::Dynamic {
        fields.push((
            "target_address",
            format!("{}:{}", params.target_host, params.target_port),
        ));
    }
    fields
}

fn remote_desktop_fields(locale: &str, params: RemoteDesktopParams) -> Vec<(&'static str, String)> {
    vec![
        ("protocol", params.protocol.label().to_string()),
        ("host", params.host),
        ("port", params.port.to_string()),
        ("username", params.username.unwrap_or_default()),
        ("domain", params.domain.unwrap_or_default()),
        ("read_only", tr(locale, yes_no_key(params.read_only))),
    ]
}

fn share_line(locale: &str, field: &str, value: &str, separator: &str) -> String {
    format!(
        "{}{separator}{value}",
        tr(locale, &format!("Connection.Share.{field}"))
    )
}

fn full_info_line(locale: &str, field: &str, value: &str, separator: &str) -> String {
    format!(
        "{}{separator}{value}",
        tr(locale, &format!("Connection.FullInfo.{field}"))
    )
}

/// “完整信息”复制/导出默认隐藏 Telnet 登录脚本的 send 值。
///
/// 这些值通常包含密码、enable 密码或 token；即使本地数据库中已经加密，
/// 也不应在用户复制“全部连接信息”时默认明文输出。
fn redact_telnet_login_script_sends(value: &mut Value, locale: &str) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if key == "login_script" {
                    if let Some(steps) = value.as_array_mut() {
                        for step in steps {
                            if let Some(send) = step
                                .as_object_mut()
                                .and_then(|fields| fields.get_mut("send"))
                            {
                                *send = Value::String(tr(
                                    locale,
                                    "Connection.FullInfo.login_script_send_redacted",
                                ));
                            }
                        }
                    }
                } else {
                    redact_telnet_login_script_sends(value, locale);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_telnet_login_script_sends(value, locale);
            }
        }
        _ => {}
    }
}

fn redact_embedded_private_keys(value: &mut Value, locale: &str) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if is_private_key_content_variant(key, value) {
                    redact_embedded_private_keys(value, locale);
                } else if is_embedded_private_key_field(key) {
                    *value = Value::String(tr(
                        locale,
                        "Connection.FullInfo.embedded_private_key_redacted",
                    ));
                } else {
                    redact_embedded_private_keys(value, locale);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_embedded_private_keys(value, locale);
            }
        }
        _ => {}
    }
}

fn is_private_key_content_variant(key: &str, value: &Value) -> bool {
    key == "PrivateKeyContent"
        && value
            .as_object()
            .is_some_and(|fields| fields.contains_key("private_key"))
}

fn is_embedded_private_key_field(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    normalized.ends_with("privatekey") || normalized.ends_with("privatekeycontent")
}

fn tr(locale: &str, key: &str) -> String {
    _rust_i18n_translate(locale, key).into_owned()
}

fn ssh_auth_key(auth: &SshAuthMethod) -> &'static str {
    match auth {
        SshAuthMethod::Password { .. } => "Connection.Share.auth_password",
        SshAuthMethod::PrivateKey { .. } | SshAuthMethod::PrivateKeyContent { .. } => {
            "Connection.Share.auth_private_key"
        }
        SshAuthMethod::Agent => "Connection.Share.auth_agent",
        SshAuthMethod::Pageant => "Connection.Share.auth_pageant",
        SshAuthMethod::AutoPublicKey => "Connection.Share.auth_auto_public_key",
    }
}

fn redis_mode_key(mode: &RedisMode) -> &'static str {
    match mode {
        RedisMode::Standalone => "Connection.Share.mode_standalone",
        RedisMode::Sentinel => "Connection.Share.mode_sentinel",
        RedisMode::Cluster => "Connection.Share.mode_cluster",
    }
}

fn yes_no_key(value: bool) -> &'static str {
    if value {
        "Connection.Share.yes"
    } else {
        "Connection.Share.no"
    }
}

fn connection_type_key(connection_type: ConnectionType) -> &'static str {
    match connection_type {
        ConnectionType::All => "Connection.Share.type_all",
        ConnectionType::Database => "Connection.Share.type_database",
        ConnectionType::SshSftp => "Connection.Share.type_ssh_sftp",
        ConnectionType::Redis => "Connection.Share.type_redis",
        ConnectionType::MongoDB => "Connection.Share.type_mongodb",
        ConnectionType::Mqtt => "Connection.Share.type_mqtt",
        ConnectionType::Serial => "Connection.Share.type_serial",
        ConnectionType::Telnet => "Connection.Share.type_telnet",
        ConnectionType::PortForwarding => "Connection.Share.type_port_forwarding",
        ConnectionType::Rdp => "Connection.Share.type_rdp",
        ConnectionType::Vnc => "Connection.Share.type_vnc",
        ConnectionType::Extension => "Connection.Share.type_extension",
    }
}

fn extension_fields(connection: &StoredConnection) -> Vec<(&'static str, String)> {
    let Ok(params) = connection.to_extension_params() else {
        return Vec::new();
    };
    vec![
        ("Extension", params.extension_id),
        ("Connection Type", params.contribution_id),
        ("Configuration", Value::Object(params.config).to_string()),
    ]
}

fn database_type_label(locale: &str, database_type: &one_core::storage::DatabaseType) -> String {
    match database_type {
        one_core::storage::DatabaseType::External { .. } => {
            tr(locale, "Connection.Share.database_type_external")
        }
        _ => database_type.as_str().to_string(),
    }
}

fn forwarding_mode_key(kind: PortForwardingKind) -> &'static str {
    match kind {
        PortForwardingKind::Local => "Connection.Share.forwarding_local",
        PortForwardingKind::Remote => "Connection.Share.forwarding_remote",
        PortForwardingKind::Dynamic => "Connection.Share.forwarding_dynamic",
    }
}

fn serial_flow_control_key(label: &str) -> &'static str {
    match label {
        "XON/XOFF" => "Connection.Share.flow_software",
        "RTS/CTS" => "Connection.Share.flow_hardware",
        _ => "Connection.Share.flow_none",
    }
}

#[cfg(test)]
#[path = "connection_share_tests.rs"]
mod tests;
