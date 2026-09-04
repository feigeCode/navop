use one_core::storage::{
    DatabaseType, DbConnectionConfig, MongoDBParams, PortForwardingKind, PortForwardingParams,
    RedisMode, RedisParams, SerialParams, SshAuthMethod, SshParams,
};
use url::Url;

pub(super) fn database_command(params: &DbConnectionConfig) -> Option<String> {
    let _ = non_empty(&params.host)?;
    let host = shell_quote(&params.host);
    let user = shell_quote(&params.username);
    let database = params.database.as_deref().map(shell_quote);
    match &params.database_type {
        DatabaseType::MySQL => Some(format!(
            "mysql -h {host} -P {} -u {user}{}",
            params.port,
            database
                .map(|value| format!(" {value}"))
                .unwrap_or_default()
        )),
        DatabaseType::PostgreSQL => Some(format!(
            "psql -h {host} -p {} -U {user}{}",
            params.port,
            database
                .map(|value| format!(" -d {value}"))
                .unwrap_or_default()
        )),
        DatabaseType::SQLite => Some(format!("sqlite3 {}", shell_quote(&params.host))),
        DatabaseType::DuckDB => Some(format!("duckdb {}", shell_quote(&params.host))),
        DatabaseType::MSSQL => Some(format!(
            "sqlcmd -S {host},{} -U {user}{}",
            params.port,
            database
                .map(|value| format!(" -d {value}"))
                .unwrap_or_default()
        )),
        DatabaseType::ClickHouse => Some(format!(
            "clickhouse-client --host {host} --port {} --user {user}{}",
            params.port,
            database
                .map(|value| format!(" --database {value}"))
                .unwrap_or_default()
        )),
        // TDengine 官方 CLI:taos(-h 主机 -P 端口 -u 用户 -d 库)。
        DatabaseType::TDengine => Some(format!(
            "taos -h {host} -P {} -u {user}{}",
            params.port,
            database
                .map(|value| format!(" -d {value}"))
                .unwrap_or_default()
        )),
        DatabaseType::Oracle | DatabaseType::External { .. } => None,
    }
}

pub(super) fn database_jdbc_url(params: &DbConnectionConfig) -> Option<String> {
    let _ = non_empty(&params.host)?;
    let database = params
        .database
        .as_deref()
        .filter(|database| !database.trim().is_empty());
    match &params.database_type {
        DatabaseType::MySQL => Some(network_jdbc_url("mysql", params, database)),
        DatabaseType::PostgreSQL => Some(network_jdbc_url("postgresql", params, database)),
        DatabaseType::SQLite => non_empty(&params.host).map(|path| format!("jdbc:sqlite:{path}")),
        DatabaseType::DuckDB => non_empty(&params.host).map(|path| format!("jdbc:duckdb:{path}")),
        DatabaseType::MSSQL => {
            let address = host_port(&params.host, params.port);
            Some(format!(
                "jdbc:sqlserver://{address}{}",
                database
                    .map(|database| format!(";databaseName={database}"))
                    .unwrap_or_default()
            ))
        }
        DatabaseType::Oracle => {
            let address = host_port(&params.host, params.port);
            params
                .service_name
                .as_deref()
                .and_then(non_empty)
                .map(|service_name| format!("jdbc:oracle:thin:@//{address}/{service_name}"))
                .or_else(|| {
                    params
                        .sid
                        .as_deref()
                        .and_then(non_empty)
                        .map(|sid| format!("jdbc:oracle:thin:@{address}:{sid}"))
                })
        }
        DatabaseType::ClickHouse => Some(network_jdbc_url("clickhouse", params, database)),
        // TDengine 官方 JDBC 驱动的 REST/WebSocket 连接形式。
        DatabaseType::TDengine => Some(network_jdbc_url("TAOS-RS", params, database)),
        DatabaseType::External { .. } => None,
    }
}

pub(super) fn ssh_command(params: &SshParams) -> String {
    ssh_like_command("ssh", params, true)
}

pub(super) fn sftp_command(params: &SshParams) -> String {
    ssh_like_command("sftp", params, false)
}

pub(super) fn redis_uri(params: &RedisParams) -> Option<String> {
    let _ = non_empty(&params.host)?;
    (params.mode == RedisMode::Standalone).then(|| {
        format!(
            "{}://{}/{}",
            if params.use_tls { "rediss" } else { "redis" },
            host_port(&params.host, params.port),
            params.db_index
        )
    })
}

pub(super) fn redis_command(params: &RedisParams) -> Option<String> {
    let _ = non_empty(&params.host)?;
    (params.mode == RedisMode::Standalone).then(|| {
        format!(
            "redis-cli -h {} -p {} -n {}{}",
            shell_quote(&params.host),
            params.port,
            params.db_index,
            if params.use_tls { " --tls" } else { "" }
        )
    })
}

pub(super) fn redis_sentinel_config(params: &RedisParams) -> Option<String> {
    if params.mode != RedisMode::Sentinel {
        return None;
    }
    let sentinel = params.sentinel.as_ref()?;
    let master_name = sentinel.master_name.trim();
    if master_name.is_empty() {
        return None;
    }
    let sentinels = sentinel
        .sentinels
        .iter()
        .map(|endpoint| endpoint.trim())
        .filter(|endpoint| !endpoint.is_empty())
        .collect::<Vec<_>>();
    if sentinels.is_empty() {
        return None;
    }
    let mut lines = vec![
        format!("master_name: {master_name}"),
        "sentinels:".to_string(),
    ];
    lines.extend(sentinels.into_iter().map(ToString::to_string));
    Some(lines.join("\n"))
}

pub(super) fn redis_cluster_nodes(params: &RedisParams) -> Option<String> {
    if params.mode != RedisMode::Cluster {
        return None;
    }
    let nodes = &params.cluster.as_ref()?.nodes;
    (!nodes.is_empty()).then(|| nodes.join("\n"))
}

pub(super) fn mongodb_safe_uri(params: &MongoDBParams) -> Option<String> {
    if !params.host.trim().is_empty() {
        return structured_mongodb_uri(params);
    }

    let mut uri = Url::parse(params.connection_string.trim()).ok()?;
    if !matches!(uri.scheme(), "mongodb" | "mongodb+srv") {
        return None;
    }
    uri.set_username("").ok()?;
    uri.set_password(None).ok()?;

    let safe_query = uri
        .query_pairs()
        .filter(|(key, _)| is_safe_mongodb_query_key(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    uri.set_query(None);
    if !safe_query.is_empty() {
        let mut query = uri.query_pairs_mut();
        for (key, value) in safe_query {
            query.append_pair(&key, &value);
        }
    }
    Some(uri.into())
}

pub(super) fn mongodb_command(params: &MongoDBParams) -> Option<String> {
    mongodb_safe_uri(params).map(|uri| format!("mongosh {}", shell_quote(&uri)))
}

pub(super) fn serial_config(params: &SerialParams) -> String {
    let parity = match params.parity {
        one_core::storage::SerialParity::None => "N",
        one_core::storage::SerialParity::Odd => "O",
        one_core::storage::SerialParity::Even => "E",
    };
    format!(
        "{}, {} baud, {}{}{}, {}",
        params.port_name,
        params.baud_rate,
        params.data_bits,
        parity,
        params.stop_bits,
        params.flow_control.label()
    )
}

pub(super) fn forwarding_command(
    forwarding: &PortForwardingParams,
    ssh: &SshParams,
) -> Option<String> {
    let bind_host = non_empty(&forwarding.bind_host)?;
    let _ = non_empty(&ssh.host)?;
    let mut parts = vec!["ssh".to_string(), "-N".to_string()];
    match forwarding.kind {
        PortForwardingKind::Local | PortForwardingKind::Remote => {
            let target_host = non_empty(&forwarding.target_host)?;
            let specification = format!(
                "{}:{}",
                host_port(bind_host, forwarding.bind_port),
                host_port(target_host, forwarding.target_port)
            );
            let flag = match forwarding.kind {
                PortForwardingKind::Local => "-L",
                PortForwardingKind::Remote => "-R",
                PortForwardingKind::Dynamic => unreachable!(),
            };
            parts.extend([flag.to_string(), shell_quote(&specification)]);
        }
        PortForwardingKind::Dynamic => {
            parts.extend([
                "-D".to_string(),
                shell_quote(&host_port(bind_host, forwarding.bind_port)),
            ]);
        }
    }
    append_safe_ssh_options(&mut parts, ssh, false);
    parts.extend([
        "-p".to_string(),
        ssh.port.to_string(),
        shell_quote(&ssh_destination(ssh)),
    ]);
    Some(parts.join(" "))
}

fn network_jdbc_url(scheme: &str, params: &DbConnectionConfig, database: Option<&str>) -> String {
    let address = host_port(&params.host, params.port);
    format!(
        "jdbc:{scheme}://{address}{}",
        database
            .map(|database| format!("/{database}"))
            .unwrap_or_default()
    )
}

fn ssh_like_command(program: &str, params: &SshParams, include_x11: bool) -> String {
    let mut parts = vec![program.to_string()];
    append_safe_ssh_options(&mut parts, params, include_x11);
    parts.extend([
        if program == "sftp" { "-P" } else { "-p" }.to_string(),
        params.port.to_string(),
        shell_quote(&ssh_destination(params)),
    ]);
    parts.join(" ")
}

fn append_safe_ssh_options(parts: &mut Vec<String>, params: &SshParams, include_x11: bool) {
    if include_x11 && params.x11_forwarding == Some(true) {
        parts.push("-X".to_string());
    }
    if let SshAuthMethod::PrivateKey { key_path, .. } = &params.auth_method
        && !key_path.trim().is_empty()
    {
        parts.extend(["-i".to_string(), shell_quote(key_path)]);
    }
    if let Some(jump) = &params.jump_server {
        let jump_host = format_host(&jump.host);
        let jump_destination = if jump.username.trim().is_empty() {
            format!("{jump_host}:{}", jump.port)
        } else {
            format!("{}@{jump_host}:{}", jump.username, jump.port)
        };
        parts.extend(["-J".to_string(), shell_quote(&jump_destination)]);
    }
}

fn ssh_destination(params: &SshParams) -> String {
    let host = format_host(&params.host);
    if params.username.trim().is_empty() {
        host
    } else {
        format!("{}@{host}", params.username)
    }
}

fn structured_mongodb_uri(params: &MongoDBParams) -> Option<String> {
    let scheme = if params.use_srv_record {
        "mongodb+srv"
    } else {
        "mongodb"
    };
    let host = format_host(params.host.trim());
    let mut uri = Url::parse(&format!("{scheme}://{host}")).ok()?;
    if !params.use_srv_record
        && let Some(port) = params.port
    {
        uri.set_port(Some(port)).ok()?;
    }
    if let Some(database) = params.database.as_deref().and_then(non_empty) {
        uri.set_path(&format!("/{database}"));
    }

    let mut query_pairs = Vec::new();
    push_query_option(
        &mut query_pairs,
        "authSource",
        params.auth_source.as_deref(),
    );
    push_query_option(
        &mut query_pairs,
        "replicaSet",
        params.replica_set.as_deref(),
    );
    push_query_option(
        &mut query_pairs,
        "readPreference",
        params.read_preference.as_deref(),
    );
    if params.use_tls {
        query_pairs.push(("tls".to_string(), "true".to_string()));
    }
    if params.direct_connection {
        query_pairs.push(("directConnection".to_string(), "true".to_string()));
    }
    if let Some(seconds) = params.connect_timeout_seconds {
        query_pairs.push((
            "connectTimeoutMS".to_string(),
            seconds.saturating_mul(1000).to_string(),
        ));
    }
    push_query_option(
        &mut query_pairs,
        "appName",
        params.application_name.as_deref(),
    );
    if !query_pairs.is_empty() {
        let mut query = uri.query_pairs_mut();
        for (key, value) in query_pairs {
            query.append_pair(&key, &value);
        }
    }
    Some(uri.into())
}

fn push_query_option(pairs: &mut Vec<(String, String)>, key: &str, value: Option<&str>) {
    if let Some(value) = value.and_then(non_empty) {
        pairs.push((key.to_string(), value.to_string()));
    }
}

fn is_safe_mongodb_query_key(key: &str) -> bool {
    matches!(
        key,
        "authSource"
            | "replicaSet"
            | "readPreference"
            | "tls"
            | "directConnection"
            | "connectTimeoutMS"
            | "appName"
    )
}

fn host_port(host: &str, port: u16) -> String {
    format!("{}:{port}", format_host(host))
}

fn format_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.trim().is_empty()).then_some(value)
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._-/@=:".contains(character))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use one_core::storage::{
        DatabaseType, DbConnectionConfig, JumpServerConfig, MongoDBParams, PortForwardingKind,
        PortForwardingParams, RedisClusterConfig, RedisMode, RedisParams, RedisSentinelConfig,
        SerialFlowControl, SerialParams, SshAuthMethod, SshParams,
    };

    use super::*;

    fn database_config(database_type: DatabaseType) -> DbConnectionConfig {
        DbConnectionConfig {
            id: String::new(),
            database_type,
            name: String::new(),
            host: "db.example.test".to_string(),
            port: 5432,
            username: "db-user".to_string(),
            password: "db-secret".to_string(),
            credential_reference: None,
            database: Some("app".to_string()),
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            extra_params: Default::default(),
        }
    }

    fn ssh_params(auth_method: SshAuthMethod) -> SshParams {
        SshParams {
            sftp_default_directory: None,
            disabled_jump_server: None,
            sftp_account: None,
            host: "2001:db8::1".to_string(),
            port: 2222,
            username: "alice doe".to_string(),
            auth_method,
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            keyboard_interactive: None,
            terminal_encoding: Default::default(),
            terminal_type: Default::default(),
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            x11_forwarding: Some(true),
            allow_legacy_algorithms: None,
            jump_server: Some(JumpServerConfig {
                host: "jump host".to_string(),
                port: 2200,
                username: "jump".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "jump-secret".to_string(),
                },
                credential_reference: None,
            }),
            proxy: None,
            os_id: None,
            icon: None,
            icon_file_path: None,
            account_expect: Default::default(),
        }
    }

    #[test]
    fn jdbc_urls_cover_builtin_database_types() {
        let cases = [
            (DatabaseType::MySQL, "jdbc:mysql://db.example.test:5432/app"),
            (
                DatabaseType::PostgreSQL,
                "jdbc:postgresql://db.example.test:5432/app",
            ),
            (
                DatabaseType::MSSQL,
                "jdbc:sqlserver://db.example.test:5432;databaseName=app",
            ),
            (
                DatabaseType::ClickHouse,
                "jdbc:clickhouse://db.example.test:5432/app",
            ),
        ];
        for (database_type, expected) in cases {
            assert_eq!(
                Some(expected.to_string()),
                database_jdbc_url(&database_config(database_type))
            );
        }

        let mut sqlite = database_config(DatabaseType::SQLite);
        sqlite.host = "/tmp/app.sqlite".to_string();
        assert_eq!(
            Some("jdbc:sqlite:/tmp/app.sqlite".to_string()),
            database_jdbc_url(&sqlite)
        );

        let mut duckdb = database_config(DatabaseType::DuckDB);
        duckdb.host = "/tmp/app.duckdb".to_string();
        assert_eq!(
            Some("jdbc:duckdb:/tmp/app.duckdb".to_string()),
            database_jdbc_url(&duckdb)
        );
    }

    #[test]
    fn jdbc_urls_handle_ipv6_oracle_service_and_sid_without_credentials() {
        let mut postgres = database_config(DatabaseType::PostgreSQL);
        postgres.host = "2001:db8::20".to_string();
        assert_eq!(
            Some("jdbc:postgresql://[2001:db8::20]:5432/app".to_string()),
            database_jdbc_url(&postgres)
        );

        let mut oracle = database_config(DatabaseType::Oracle);
        oracle.port = 1521;
        oracle.service_name = Some("ORCLPDB".to_string());
        oracle.sid = Some("ORCL".to_string());
        assert_eq!(
            Some("jdbc:oracle:thin:@//db.example.test:1521/ORCLPDB".to_string()),
            database_jdbc_url(&oracle)
        );
        oracle.service_name = None;
        assert_eq!(
            Some("jdbc:oracle:thin:@db.example.test:1521:ORCL".to_string()),
            database_jdbc_url(&oracle)
        );

        assert_eq!(
            None,
            database_jdbc_url(&database_config(DatabaseType::external("custom")))
        );
        assert!(!database_jdbc_url(&postgres).unwrap().contains("db-secret"));
    }

    #[test]
    fn ssh_and_sftp_commands_support_ipv6_identity_jump_host_and_quoting() {
        let params = ssh_params(SshAuthMethod::PrivateKey {
            key_path: "/tmp/key file".to_string(),
            passphrase: Some("key-secret".to_string()),
        });

        assert_eq!(
            "ssh -X -i '/tmp/key file' -J 'jump@jump host:2200' -p 2222 'alice doe@[2001:db8::1]'",
            ssh_command(&params)
        );
        assert_eq!(
            "sftp -i '/tmp/key file' -J 'jump@jump host:2200' -P 2222 'alice doe@[2001:db8::1]'",
            sftp_command(&params)
        );
        let joined = format!("{}\n{}", ssh_command(&params), sftp_command(&params));
        assert!(!joined.contains("key-secret"));
        assert!(!joined.contains("jump-secret"));
    }

    #[test]
    fn embedded_private_key_contents_are_never_written_to_commands() {
        let params = ssh_params(SshAuthMethod::PrivateKeyContent {
            private_key: "PRIVATE KEY BODY".to_string(),
            passphrase: Some("private-key-passphrase".to_string()),
        });
        let joined = format!("{}\n{}", ssh_command(&params), sftp_command(&params));
        assert!(!joined.contains("PRIVATE KEY BODY"));
        assert!(!joined.contains("private-key-passphrase"));
        assert!(!joined.contains(" -i "));
    }

    #[test]
    fn redis_uri_and_cli_are_available_only_for_standalone_and_omit_credentials() {
        let mut params = RedisParams {
            host: "2001:db8::30".to_string(),
            port: 6380,
            password: Some("redis-secret".to_string()),
            username: Some("redis-user".to_string()),
            credential_reference: None,
            db_index: 2,
            mode: RedisMode::Standalone,
            use_tls: true,
            connect_timeout: None,
            sentinel: None,
            cluster: None,
            ssh_tunnel: None,
        };
        assert_eq!(
            Some("rediss://[2001:db8::30]:6380/2".to_string()),
            redis_uri(&params)
        );
        assert_eq!(
            Some("redis-cli -h 2001:db8::30 -p 6380 -n 2 --tls".to_string()),
            redis_command(&params)
        );
        let copied = format!(
            "{}\n{}",
            redis_uri(&params).unwrap(),
            redis_command(&params).unwrap()
        );
        assert!(!copied.contains("redis-secret"));
        assert!(!copied.contains("redis-user"));

        params.mode = RedisMode::Sentinel;
        params.sentinel = Some(RedisSentinelConfig {
            master_name: "mymaster".to_string(),
            sentinels: vec!["redis-1:26379".to_string()],
            sentinel_password: Some("sentinel-secret".to_string()),
            credential_reference: None,
        });
        assert_eq!(None, redis_uri(&params));
        assert_eq!(None, redis_command(&params));
        assert!(
            !redis_sentinel_config(&params)
                .unwrap()
                .contains("sentinel-secret")
        );

        params.mode = RedisMode::Cluster;
        params.cluster = Some(RedisClusterConfig {
            nodes: vec!["redis-1:6379".to_string(), "redis-2:6379".to_string()],
        });
        assert_eq!(None, redis_uri(&params));
        assert_eq!(None, redis_command(&params));
        assert_eq!(
            Some("redis-1:6379\nredis-2:6379".to_string()),
            redis_cluster_nodes(&params)
        );
    }

    #[test]
    fn redis_sentinel_config_requires_a_master_and_at_least_one_endpoint() {
        let mut params = RedisParams {
            host: String::new(),
            port: 6379,
            password: Some("redis-secret".to_string()),
            username: Some("redis-user".to_string()),
            credential_reference: None,
            db_index: 0,
            mode: RedisMode::Sentinel,
            use_tls: false,
            connect_timeout: None,
            sentinel: Some(RedisSentinelConfig {
                master_name: String::new(),
                sentinels: vec!["redis-1:26379".to_string()],
                sentinel_password: Some("sentinel-secret".to_string()),
                credential_reference: None,
            }),
            cluster: None,
            ssh_tunnel: None,
        };
        assert_eq!(None, redis_sentinel_config(&params));

        let sentinel = params.sentinel.as_mut().expect("sentinel");
        sentinel.master_name = "mymaster".to_string();
        sentinel.sentinels.clear();
        assert_eq!(None, redis_sentinel_config(&params));

        let sentinel = params.sentinel.as_mut().expect("sentinel");
        sentinel.sentinels = vec!["  ".to_string(), " redis-1:26379 ".to_string()];
        assert_eq!(
            Some("master_name: mymaster\nsentinels:\nredis-1:26379".to_string()),
            redis_sentinel_config(&params)
        );
        assert!(
            !redis_sentinel_config(&params)
                .expect("valid sentinel config")
                .contains("sentinel-secret")
        );
    }

    #[test]
    fn mongodb_safe_uri_filters_userinfo_passwords_and_unknown_query_parameters() {
        let params = MongoDBParams {
            driver_variant: Default::default(),
            connection_string:
                "mongodb://admin:raw-secret@[2001:db8::40]:27017/app?authSource=admin&password=query-secret&tls=true&unknown=value"
                    .to_string(),
            host: String::new(),
            port: None,
            database: None,
            username: Some("admin".to_string()),
            password: Some("structured-secret".to_string()),
            credential_reference: None,
            auth_source: None,
            replica_set: None,
            read_preference: None,
            use_srv_record: false,
            direct_connection: false,
            use_tls: false,
            connect_timeout_seconds: None,
            application_name: None,
            ssh_tunnel: None,
        };

        let uri = mongodb_safe_uri(&params).expect("sanitized URI");
        assert!(uri.starts_with("mongodb://[2001:db8::40]:27017/app"));
        assert!(uri.contains("authSource=admin"));
        assert!(uri.contains("tls=true"));
        assert!(!uri.contains("admin:"));
        assert!(!uri.contains("raw-secret"));
        assert!(!uri.contains("query-secret"));
        assert!(!uri.contains("structured-secret"));
        assert!(!uri.contains("unknown"));
        assert_eq!(
            Some(format!("mongosh {}", shell_quote(&uri))),
            mongodb_command(&params)
        );
    }

    #[test]
    fn mongodb_structured_uri_preserves_safe_options_and_encodes_values() {
        let params = MongoDBParams {
            driver_variant: Default::default(),
            connection_string: String::new(),
            host: "cluster.example.test".to_string(),
            port: Some(27017),
            database: Some("app data".to_string()),
            username: Some("admin".to_string()),
            password: Some("secret".to_string()),
            credential_reference: None,
            auth_source: Some("admin".to_string()),
            replica_set: Some("rs 0".to_string()),
            read_preference: Some("secondaryPreferred".to_string()),
            use_srv_record: true,
            direct_connection: true,
            use_tls: true,
            connect_timeout_seconds: Some(5),
            application_name: Some("Navop Desktop".to_string()),
            ssh_tunnel: None,
        };

        let uri = mongodb_safe_uri(&params).expect("structured URI");
        assert!(uri.starts_with("mongodb+srv://cluster.example.test/app%20data?"));
        assert!(uri.contains("authSource=admin"));
        assert!(uri.contains("replicaSet=rs+0"));
        assert!(uri.contains("readPreference=secondaryPreferred"));
        assert!(uri.contains("tls=true"));
        assert!(uri.contains("directConnection=true"));
        assert!(uri.contains("connectTimeoutMS=5000"));
        assert!(uri.contains("appName=Navop+Desktop"));
        assert!(!uri.contains("secret"));
        assert!(!uri.contains("admin@"));
    }

    #[test]
    fn serial_configuration_is_human_readable() {
        let params = SerialParams {
            port_name: "/dev/ttyUSB0".to_string(),
            baud_rate: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: Default::default(),
            flow_control: SerialFlowControl::Hardware,
        };
        assert_eq!(
            "/dev/ttyUSB0, 115200 baud, 8N1, RTS/CTS",
            serial_config(&params)
        );
    }

    #[test]
    fn forwarding_commands_cover_local_remote_and_dynamic_modes() {
        let mut ssh = ssh_params(SshAuthMethod::Agent);
        ssh.host = "bastion".to_string();
        ssh.port = 22;
        ssh.username = "alice".to_string();
        ssh.x11_forwarding = None;
        ssh.jump_server = None;

        let mut forwarding = PortForwardingParams {
            ssh_connection_id: 42,
            kind: PortForwardingKind::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 3307,
            target_host: "db internal".to_string(),
            target_port: 3306,
        };
        assert_eq!(
            Some("ssh -N -L '127.0.0.1:3307:db internal:3306' -p 22 alice@bastion".to_string()),
            forwarding_command(&forwarding, &ssh)
        );

        forwarding.kind = PortForwardingKind::Remote;
        assert_eq!(
            Some("ssh -N -R '127.0.0.1:3307:db internal:3306' -p 22 alice@bastion".to_string()),
            forwarding_command(&forwarding, &ssh)
        );

        forwarding.kind = PortForwardingKind::Dynamic;
        forwarding.bind_host = "2001:db8::50".to_string();
        forwarding.bind_port = 1080;
        assert_eq!(
            Some("ssh -N -D '[2001:db8::50]:1080' -p 22 alice@bastion".to_string()),
            forwarding_command(&forwarding, &ssh)
        );
    }

    #[test]
    fn forwarding_commands_reject_incomplete_hosts() {
        let mut ssh = ssh_params(SshAuthMethod::Agent);
        ssh.host = "bastion".to_string();
        ssh.jump_server = None;
        let mut forwarding = PortForwardingParams {
            ssh_connection_id: 42,
            kind: PortForwardingKind::Local,
            bind_host: String::new(),
            bind_port: 3307,
            target_host: "db.internal".to_string(),
            target_port: 3306,
        };

        assert_eq!(None, forwarding_command(&forwarding, &ssh));

        forwarding.bind_host = "127.0.0.1".to_string();
        forwarding.target_host.clear();
        assert_eq!(None, forwarding_command(&forwarding, &ssh));

        forwarding.kind = PortForwardingKind::Dynamic;
        assert!(forwarding_command(&forwarding, &ssh).is_some());

        ssh.host.clear();
        assert_eq!(None, forwarding_command(&forwarding, &ssh));
    }
}
