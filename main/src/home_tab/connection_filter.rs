use super::*;

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
    pub(crate) fn set_selected_filter(&mut self, filter: ConnectionType, cx: &mut Context<Self>) {
        self.selected_filter = filter;
        cx.notify();
    }

    pub(crate) fn match_connection_type(&self, conn: &StoredConnection) -> bool {
        match self.selected_filter {
            ConnectionType::All => true,
            filter_type => conn.connection_type == filter_type,
        }
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
}
