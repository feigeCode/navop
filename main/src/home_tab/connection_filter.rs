use super::*;

/// 连接类型筛选目标：内置类型或某个具体扩展连接贡献。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ConnectionFilter {
    All,
    Builtin(ConnectionType),
    Extension(ExtensionFilterTarget),
}

/// 参与筛选的扩展连接贡献（按 extension_id + contribution_id 精确匹配连接；label 用于菜单展示）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExtensionFilterTarget {
    pub extension_id: String,
    pub contribution_id: String,
    pub label: String,
}

impl ConnectionFilter {
    /// 菜单与按钮共用的英文显示名（与核心 `ConnectionType::label()` 保持一致）。
    pub(crate) fn label(&self) -> String {
        match self {
            Self::All => ConnectionType::All.label().to_string(),
            Self::Builtin(kind) => kind.label().to_string(),
            Self::Extension(target) => target.label.clone(),
        }
    }

    pub(crate) fn is_all(&self) -> bool {
        matches!(self, Self::All)
    }
}

pub(crate) fn connection_matches_query(conn: &StoredConnection, query: &str) -> bool {
    let query = query.to_lowercase();

    if query.is_empty() {
        return true;
    }

    // 匹配连接名称
    if conn.name.to_lowercase().contains(&query) {
        return true;
    }

    // 根据连接类型解析对应参数进行匹配
    match conn.connection_type {
        ConnectionType::Database => {
            if let Ok(params) = conn.to_db_connection() {
                if params.host.to_lowercase().contains(&query) {
                    return true;
                }
                if params.port.to_string().contains(&query) {
                    return true;
                }
                if params.username.to_lowercase().contains(&query) {
                    return true;
                }
                if params
                    .database
                    .as_ref()
                    .map_or(false, |db| db.to_lowercase().contains(&query))
                {
                    return true;
                }
                let conn_str = format!("{}@{}:{}", params.username, params.host, params.port);
                if conn_str.to_lowercase().contains(&query) {
                    return true;
                }
            }
        }
        ConnectionType::SshSftp => {
            if let Ok(params) = conn.to_ssh_params() {
                if params.host.to_lowercase().contains(&query) {
                    return true;
                }
                if params.port.to_string().contains(&query) {
                    return true;
                }
                if params.username.to_lowercase().contains(&query) {
                    return true;
                }
                let conn_str = format!("{}@{}:{}", params.username, params.host, params.port);
                if conn_str.to_lowercase().contains(&query) {
                    return true;
                }
            }
        }
        ConnectionType::Telnet => {
            if let Ok(params) = conn.to_telnet_params() {
                if params.host.to_lowercase().contains(&query) {
                    return true;
                }
                if params.port.to_string().contains(&query) {
                    return true;
                }
                let conn_str = format!("{}:{}", params.host, params.port);
                if conn_str.to_lowercase().contains(&query) {
                    return true;
                }
            }
        }
        ConnectionType::Rdp | ConnectionType::Vnc => {
            if let Ok(params) = conn.to_remote_desktop_params() {
                if params.host.to_lowercase().contains(&query) {
                    return true;
                }
                if params.port.to_string().contains(&query) {
                    return true;
                }
                if params
                    .username
                    .as_ref()
                    .map_or(false, |username| username.to_lowercase().contains(&query))
                {
                    return true;
                }
                let conn_str = match params.username {
                    Some(username) => {
                        format!("{}@{}:{}", username, params.host, params.port)
                    }
                    None => format!("{}:{}", params.host, params.port),
                };
                if conn_str.to_lowercase().contains(&query) {
                    return true;
                }
            }
        }
        ConnectionType::Redis => {
            if let Ok(params) = conn.to_redis_params() {
                if params.host.to_lowercase().contains(&query) {
                    return true;
                }
                if params.port.to_string().contains(&query) {
                    return true;
                }
                if params
                    .username
                    .as_ref()
                    .map_or(false, |u| u.to_lowercase().contains(&query))
                {
                    return true;
                }
            }
        }
        ConnectionType::MongoDB => {
            if let Ok(params) = conn.to_mongodb_params() {
                if params.host.to_lowercase().contains(&query) {
                    return true;
                }
                if params
                    .port
                    .map_or(false, |p| p.to_string().contains(&query))
                {
                    return true;
                }
                if params
                    .username
                    .as_ref()
                    .map_or(false, |u| u.to_lowercase().contains(&query))
                {
                    return true;
                }
                if params
                    .database
                    .as_ref()
                    .map_or(false, |db| db.to_lowercase().contains(&query))
                {
                    return true;
                }
                if params.connection_string.to_lowercase().contains(&query) {
                    return true;
                }
            }
        }
        ConnectionType::PortForwarding => {
            if let Ok(params) = conn.to_port_forwarding_params() {
                if port_forwarding_connection_info(&params)
                    .to_lowercase()
                    .contains(&query)
                {
                    return true;
                }
            }
        }
        _ => {}
    }

    false
}

impl HomePage {
    pub(crate) fn set_selected_filter(&mut self, filter: ConnectionFilter, cx: &mut Context<Self>) {
        self.selected_filter = filter;
        cx.notify();
    }

    pub(crate) fn match_connection_type(&self, conn: &StoredConnection) -> bool {
        match_connection_type(&self.selected_filter, conn)
    }

    pub(crate) fn match_connection(&self, conn: &StoredConnection, query: &str) -> bool {
        connection_matches_query(conn, query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::models::TelnetParams;

    fn telnet_connection() -> StoredConnection {
        StoredConnection::new_telnet(
            "Lab Console".to_string(),
            TelnetParams {
                host: "Switch.EXAMPLE.com".to_string(),
                port: 2323,
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                backspace_code: Default::default(),
                login_script: Vec::new(),
            },
            None,
        )
    }

    #[test]
    fn telnet_connection_matches_host_port_and_endpoint() {
        let connection = telnet_connection();

        assert!(connection_matches_query(&connection, "switch.example"));
        assert!(connection_matches_query(&connection, "SWITCH.EXAMPLE.COM"));
        assert!(connection_matches_query(&connection, "2323"));
        assert!(connection_matches_query(
            &connection,
            "switch.example.com:2323"
        ));
        assert!(!connection_matches_query(&connection, "router.example.com"));
    }

    fn extension_connection(extension_id: &str, contribution_id: &str) -> StoredConnection {
        serde_json::from_value(serde_json::json!({
            "id": 1,
            "name": "Ext",
            "connection_type": "Extension",
            // `params` 在存储模型中是 JSON 字符串，与 to_extension_params 的解析方式一致。
            "params": serde_json::json!({
                "schema_version": 1,
                "extension_id": extension_id,
                "contribution_id": contribution_id,
                "config": {},
                "secrets": {}
            })
            .to_string(),
            "workspace_id": 1,
            "sync_enabled": true,
        }))
        .expect("extension connection fixture should deserialize")
    }

    fn mqtt_extension_target() -> ExtensionFilterTarget {
        ExtensionFilterTarget {
            extension_id: "com.navop.middleware.mqtt".into(),
            contribution_id: "mqtt".into(),
            label: "MQTT".into(),
        }
    }

    #[test]
    fn builtin_filter_matches_only_the_selected_builtin_type() {
        let telnet = telnet_connection();
        let extension = extension_connection("com.navop.middleware.mqtt", "mqtt");

        assert!(match_connection_type(&ConnectionFilter::All, &telnet));
        assert!(match_connection_type(&ConnectionFilter::All, &extension));
        assert!(match_connection_type(
            &ConnectionFilter::Builtin(ConnectionType::Telnet),
            &telnet
        ));
        assert!(!match_connection_type(
            &ConnectionFilter::Builtin(ConnectionType::Redis),
            &telnet
        ));
        assert!(!match_connection_type(
            &ConnectionFilter::Builtin(ConnectionType::Telnet),
            &extension
        ));
    }

    #[test]
    fn extension_filter_matches_only_the_selected_extension_contribution() {
        let mqtt = extension_connection("com.navop.middleware.mqtt", "mqtt");
        let rocketmq = extension_connection("com.navop.middleware.rocketmq", "rocketmq");

        assert!(match_connection_type(
            &ConnectionFilter::Extension(mqtt_extension_target()),
            &mqtt
        ));
        assert!(!match_connection_type(
            &ConnectionFilter::Extension(mqtt_extension_target()),
            &rocketmq
        ));

        // 旧内置 MQTT 连接也归入 MQTT 扩展筛选，保证合并菜单项筛选正常。
        let legacy_mqtt = serde_json::from_value(serde_json::json!({
            "id": 2,
            "name": "Legacy",
            "connection_type": "Mqtt",
            "params": "{}",
            "workspace_id": 1,
            "sync_enabled": true,
        }))
        .expect("legacy mqtt fixture should deserialize");
        assert!(match_connection_type(
            &ConnectionFilter::Extension(mqtt_extension_target()),
            &legacy_mqtt
        ));
    }
}

/// Stateless predicate: home and tree must each supply their own selection.
pub(crate) fn match_connection_type(filter: &ConnectionFilter, conn: &StoredConnection) -> bool {
    match filter {
        ConnectionFilter::All => true,
        ConnectionFilter::Builtin(kind) => {
            if *kind == ConnectionType::Extension {
                // 兼容旧状态：单一 Extension 项表示“所有扩展连接”。
                conn.connection_type == ConnectionType::Extension
            } else {
                conn.connection_type == *kind
            }
        }
        ConnectionFilter::Extension(target) => extension_connection_matches(target, conn),
    }
}

fn extension_connection_matches(target: &ExtensionFilterTarget, conn: &StoredConnection) -> bool {
    if conn.connection_type == ConnectionType::Extension {
        return conn
            .to_extension_params()
            .map(|params| {
                params
                    .extension_id
                    .eq_ignore_ascii_case(&target.extension_id)
                    && params
                        .contribution_id
                        .eq_ignore_ascii_case(&target.contribution_id)
            })
            .unwrap_or(false);
    }
    // 迁移前的旧内置 MQTT 连接归入 com.navop.middleware.mqtt 扩展筛选，
    // 保证合并菜单项也能命中历史数据（筛选正常）。
    conn.connection_type == ConnectionType::Mqtt
        && target
            .extension_id
            .eq_ignore_ascii_case("com.navop.middleware.mqtt")
}
