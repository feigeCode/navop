use one_core::storage::{
    ConnectionType, DatabaseType, DbConnectionConfig, MongoDBParams, RedisParams, SshParams,
    StoredConnection,
};

pub(super) fn connection_command(connection: &StoredConnection) -> Option<String> {
    match connection.connection_type {
        ConnectionType::Database => database_command(connection.to_db_connection().ok()?),
        ConnectionType::SshSftp => Some(ssh_command(connection.to_ssh_params().ok()?)),
        ConnectionType::Redis => Some(redis_command(connection.to_redis_params().ok()?)),
        ConnectionType::MongoDB => mongodb_command(connection.to_mongodb_params().ok()?),
        _ => None,
    }
}

fn database_command(params: DbConnectionConfig) -> Option<String> {
    let host = shell_quote(&params.host);
    let user = shell_quote(&params.username);
    let database = params.database.as_deref().map(shell_quote);
    match params.database_type {
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
        DatabaseType::Oracle | DatabaseType::External { .. } => None,
    }
}

fn ssh_command(params: SshParams) -> String {
    format!(
        "ssh -p {} {}@{}",
        params.port,
        shell_quote(&params.username),
        shell_quote(&params.host)
    )
}

fn redis_command(params: RedisParams) -> String {
    format!(
        "redis-cli -h {} -p {} -n {}{}",
        shell_quote(&params.host),
        params.port,
        params.db_index,
        if params.use_tls { " --tls" } else { "" }
    )
}

fn mongodb_command(params: MongoDBParams) -> Option<String> {
    let port = params.port?;
    let database = params.database.as_deref().unwrap_or("admin");
    Some(format!(
        "mongosh mongodb://{}:{port}/{}",
        shell_quote(&params.host),
        shell_quote(database)
    ))
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._-/:".contains(character))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use one_core::storage::{SshAuthMethod, SshParams};

    use super::*;

    #[test]
    fn ssh_command_omits_password_and_quotes_values() {
        let connection = StoredConnection::new_ssh(
            "prod".to_string(),
            SshParams {
                host: "ssh host.example".to_string(),
                port: 2222,
                username: "alice".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "secret".to_string(),
                },
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
            },
            None,
        );

        let command = connection_command(&connection).expect("SSH command should exist");
        assert_eq!("ssh -p 2222 alice@'ssh host.example'", command);
        assert!(!command.contains("secret"));
    }

    #[test]
    fn unsupported_connection_type_has_no_command() {
        let mut connection = StoredConnection::new_ssh(
            "rdp".to_string(),
            SshParams {
                host: "example.test".to_string(),
                port: 22,
                username: "alice".to_string(),
                auth_method: SshAuthMethod::Agent,
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
            },
            None,
        );
        connection.connection_type = ConnectionType::Rdp;
        assert_eq!(None, connection_command(&connection));
    }
}
