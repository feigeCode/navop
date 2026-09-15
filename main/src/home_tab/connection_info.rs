use super::*;

pub(super) fn card_connection_info(conn: &StoredConnection) -> Option<String> {
    if cfg!(feature = "screenshot-safe") {
        return screenshot_safe_connection_info(conn.connection_type).map(str::to_owned);
    }

    match conn.connection_type {
        ConnectionType::Database => conn.to_db_connection().ok().map(database_connection_info),
        ConnectionType::SshSftp => conn
            .to_ssh_params()
            .ok()
            .map(|params| format!("{}@{}:{}", params.username, params.host, params.port)),
        ConnectionType::Redis => conn.to_redis_params().ok().map(redis_connection_info),
        ConnectionType::MongoDB => conn.to_mongodb_params().ok().map(mongodb_connection_info),
        ConnectionType::Mqtt => conn.to_mqtt_params().ok().map(mqtt_connection_info),
        ConnectionType::Serial => conn.to_serial_params().ok().map(serial_connection_info),
        ConnectionType::Telnet => conn
            .to_telnet_params()
            .ok()
            .map(|params| format!("{}:{}", params.host, params.port)),
        ConnectionType::Rdp | ConnectionType::Vnc => conn
            .to_remote_desktop_params()
            .ok()
            .map(|params| remote_desktop_connection_info(&params)),
        ConnectionType::PortForwarding => conn
            .to_port_forwarding_params()
            .ok()
            .map(|params| port_forwarding_connection_info(&params)),
        ConnectionType::Extension => conn
            .to_extension_params()
            .ok()
            .map(|params| format!("{} / {}", params.extension_id, params.contribution_id)),
        _ => None,
    }
}

pub(super) fn screenshot_safe_connection_info(
    connection_type: ConnectionType,
) -> Option<&'static str> {
    match connection_type {
        ConnectionType::Database => Some("user@localhost:5432/example"),
        ConnectionType::SshSftp => Some("user@localhost:22"),
        ConnectionType::Redis => Some("localhost:6379/0"),
        ConnectionType::MongoDB => Some("localhost:27017"),
        ConnectionType::Mqtt => Some("localhost:1883"),
        ConnectionType::Serial => Some("COM1 (115200, 8N1)"),
        ConnectionType::Telnet => Some("localhost:23"),
        ConnectionType::PortForwarding => Some("localhost:8080 -> localhost:80"),
        ConnectionType::Rdp => Some("user@localhost:3389"),
        ConnectionType::Vnc => Some("user@localhost:5900"),
        ConnectionType::Extension => Some("Local Extension"),
        ConnectionType::All => None,
    }
}

pub(super) fn connection_display_name(conn: &StoredConnection) -> String {
    if !cfg!(feature = "screenshot-safe") {
        return conn.name.clone();
    }

    match conn.connection_type {
        ConnectionType::Database => "Local Database",
        ConnectionType::SshSftp => "Local SSH",
        ConnectionType::Redis => "Local Redis",
        ConnectionType::MongoDB => "Local MongoDB",
        ConnectionType::Mqtt => "Local MQTT",
        ConnectionType::Serial => "Local Serial",
        ConnectionType::Telnet => "Local Telnet",
        ConnectionType::PortForwarding => "Local Port Forwarding",
        ConnectionType::Rdp => "Local RDP",
        ConnectionType::Vnc => "Local VNC",
        ConnectionType::Extension => "Local Extension",
        ConnectionType::All => "Local Connection",
    }
    .to_owned()
}

fn database_connection_info(params: one_core::storage::DbConnectionConfig) -> String {
    if matches!(
        params.database_type,
        DatabaseType::SQLite | DatabaseType::DuckDB
    ) {
        return params.host;
    }
    let database = params
        .database
        .map(|database| format!("/{database}"))
        .unwrap_or_default();
    format!(
        "{}@{}:{}{}",
        params.username, params.host, params.port, database
    )
}

fn redis_connection_info(params: one_core::storage::RedisParams) -> String {
    match params.mode {
        RedisMode::Standalone => format!("{}:{}/{}", params.host, params.port, params.db_index),
        RedisMode::Sentinel => {
            let (master_name, sentinel_count) = params
                .sentinel
                .as_ref()
                .map(|sentinel| (sentinel.master_name.as_str(), sentinel.sentinels.len()))
                .unwrap_or(("sentinel", 0));
            format!("{master_name} (sentinel:{sentinel_count})")
        }
        RedisMode::Cluster => {
            let node_count = params
                .cluster
                .as_ref()
                .map(|cluster| cluster.nodes.len())
                .unwrap_or(0);
            format!("cluster ({node_count} nodes)")
        }
    }
}

fn mongodb_connection_info(params: one_core::storage::MongoDBParams) -> String {
    if !params.host.is_empty() {
        return params
            .port
            .map(|port| format!("{}:{port}", params.host))
            .unwrap_or(params.host);
    }
    if !params.connection_string.is_empty() {
        return params.connection_string;
    }
    "MongoDB".to_string()
}

fn mqtt_connection_info(params: one_core::storage::MqttParams) -> String {
    format!("{}:{}", params.host, params.port)
}

fn serial_connection_info(params: one_core::storage::SerialParams) -> String {
    let parity = match params.parity {
        one_core::storage::models::SerialParity::None => 'N',
        one_core::storage::models::SerialParity::Odd => 'O',
        one_core::storage::models::SerialParity::Even => 'E',
    };
    format!(
        "{} ({}, {}{}{})",
        params.port_name, params.baud_rate, params.data_bits, parity, params.stop_bits
    )
}

pub(super) fn remote_desktop_connection_info(params: &RemoteDesktopParams) -> String {
    match params.username.as_deref() {
        Some(username) => format!("{}@{}:{}", username, params.host, params.port),
        None => format!("{}:{}", params.host, params.port),
    }
}

pub(super) fn port_forwarding_connection_info(
    params: &one_core::storage::PortForwardingParams,
) -> String {
    match params.kind {
        one_core::storage::PortForwardingKind::Local => format!(
            "{}:{} -> {}:{}",
            params.bind_host, params.bind_port, params.target_host, params.target_port
        ),
        one_core::storage::PortForwardingKind::Remote => format!(
            "{}:{} <- {}:{}",
            params.bind_host, params.bind_port, params.target_host, params.target_port
        ),
        one_core::storage::PortForwardingKind::Dynamic => {
            format!("SOCKS {}:{}", params.bind_host, params.bind_port)
        }
    }
}

/// 生成复制连接的唯一名称
pub(super) fn generate_duplicate_name(
    original_name: &str,
    existing_names: &HashSet<String>,
) -> String {
    let base_name = t!("Home.duplicate_name", name = original_name).to_string();

    if !existing_names.contains(&base_name) {
        return base_name;
    }

    // 如果基础名称已存在，添加数字序号
    for i in 2..100 {
        let name = t!(
            "Home.duplicate_name_numbered",
            name = original_name,
            index = i
        )
        .to_string();
        if !existing_names.contains(&name) {
            return name;
        }
    }

    base_name
}
