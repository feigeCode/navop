use super::*;

impl HomePage {
    pub(super) fn connection_info_text(&self, conn: &StoredConnection) -> String {
        if cfg!(feature = "screenshot-safe") {
            return connection_info::screenshot_safe_connection_info(conn.connection_type)
                .unwrap_or_default()
                .to_owned();
        }

        match conn.connection_type {
            ConnectionType::Database => conn
                .to_db_connection()
                .map(|params| {
                    if matches!(
                        params.database_type,
                        DatabaseType::SQLite | DatabaseType::DuckDB
                    ) {
                        params.host.clone()
                    } else {
                        let database = params
                            .database
                            .map(|db| format!("/{}", db))
                            .unwrap_or_default();
                        format!(
                            "{}@{}:{}{}",
                            params.username, params.host, params.port, database
                        )
                    }
                })
                .unwrap_or_default(),
            ConnectionType::SshSftp => conn
                .to_ssh_params()
                .map(|params| format!("{}@{}:{}", params.username, params.host, params.port))
                .unwrap_or_default(),
            ConnectionType::Redis => conn
                .to_redis_params()
                .map(|params| format!("{}:{}/{}", params.host, params.port, params.db_index))
                .unwrap_or_default(),
            ConnectionType::MongoDB => conn
                .to_mongodb_params()
                .map(|params| {
                    if !params.host.is_empty() {
                        if let Some(port) = params.port {
                            format!("{}:{}", params.host, port)
                        } else {
                            params.host
                        }
                    } else if !params.connection_string.is_empty() {
                        params.connection_string
                    } else {
                        "MongoDB".to_string()
                    }
                })
                .unwrap_or_default(),
            ConnectionType::Mqtt => conn
                .to_mqtt_params()
                .map(|params| format!("{}:{}", params.host, params.port))
                .unwrap_or_default(),
            ConnectionType::Serial => conn
                .to_serial_params()
                .map(|params| {
                    let parity_char = match params.parity {
                        one_core::storage::models::SerialParity::None => 'N',
                        one_core::storage::models::SerialParity::Odd => 'O',
                        one_core::storage::models::SerialParity::Even => 'E',
                    };
                    format!(
                        "{} ({}, {}{}{})",
                        params.port_name,
                        params.baud_rate,
                        params.data_bits,
                        parity_char,
                        params.stop_bits
                    )
                })
                .unwrap_or_default(),
            ConnectionType::Telnet => conn
                .to_telnet_params()
                .map(|params| format!("{}:{}", params.host, params.port))
                .unwrap_or_default(),
            ConnectionType::Rdp | ConnectionType::Vnc => conn
                .to_remote_desktop_params()
                .map(|params| match params.username.as_deref() {
                    Some(username) => format!("{}@{}:{}", username, params.host, params.port),
                    None => format!("{}:{}", params.host, params.port),
                })
                .unwrap_or_default(),
            ConnectionType::PortForwarding => conn
                .to_port_forwarding_params()
                .map(|params| match params.kind {
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
                })
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
}
