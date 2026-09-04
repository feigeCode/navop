use one_core::storage::{DatabaseType, DbConnectionConfig};

pub fn mysql_config() -> Option<DbConnectionConfig> {
    let password = std::env::var("ONETCLI_TEST_MYSQL_PASSWORD").ok()?;
    Some(base_config(
        "navop-real-mysql",
        DatabaseType::MySQL,
        env_or("ONETCLI_TEST_MYSQL_HOST", "127.0.0.1"),
        env_port("ONETCLI_TEST_MYSQL_PORT", 3306),
        env_or("ONETCLI_TEST_MYSQL_USER", "root"),
        password,
    ))
}

pub fn postgres_config() -> Option<DbConnectionConfig> {
    let password = std::env::var("ONETCLI_TEST_POSTGRES_PASSWORD").ok()?;
    Some(base_config(
        "navop-real-postgres",
        DatabaseType::PostgreSQL,
        env_or("ONETCLI_TEST_POSTGRES_HOST", "127.0.0.1"),
        env_port("ONETCLI_TEST_POSTGRES_PORT", 5432),
        env_or("ONETCLI_TEST_POSTGRES_USER", "postgres"),
        password,
    ))
}

pub fn optional_database(config: &DbConnectionConfig, fallback: &str) -> DbConnectionConfig {
    let mut config = config.clone();
    if config.database.as_deref().unwrap_or("").is_empty() {
        config.database = Some(fallback.to_string());
    }
    config
}

pub fn skip_database(label: &str, env_var: &str) {
    eprintln!("skipping real {label} tests: set {env_var} to run them");
}

fn base_config(
    id: &str,
    database_type: DatabaseType,
    host: String,
    port: u16,
    username: String,
    password: String,
) -> DbConnectionConfig {
    DbConnectionConfig {
        id: id.to_string(),
        database_type,
        name: id.to_string(),
        host,
        port,
        username,
        password,
        credential_reference: None,
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        extra_params: std::collections::HashMap::new(),
    }
}

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}

fn env_port(key: &str, fallback: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
}
