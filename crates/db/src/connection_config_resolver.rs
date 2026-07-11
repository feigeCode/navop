use std::sync::Arc;

use one_core::storage::{
    ConnectionRepository, ConnectionType, DbConnectionConfig, StoredConnection, traits::Repository,
};

use crate::connection::DbError;

#[derive(Clone, Default)]
pub struct ConnectionConfigResolver {
    connection_repo: Option<Arc<ConnectionRepository>>,
}

impl ConnectionConfigResolver {
    pub fn new(connection_repo: Option<Arc<ConnectionRepository>>) -> Self {
        Self { connection_repo }
    }

    pub fn resolve(&self, config: DbConnectionConfig) -> Result<DbConnectionConfig, DbError> {
        Self::resolve_with_loader(config, |ssh_id| {
            let repo = self.connection_repo.as_ref().ok_or_else(|| {
                DbError::connection(
                    "ssh_connection_id is set but ConnectionRepository is unavailable",
                )
            })?;
            repo.get(ssh_id).map_err(|error| {
                DbError::connection(format!(
                    "failed to load referenced ssh connection {ssh_id}: {error}"
                ))
            })
        })
    }

    fn resolve_with_loader<F>(
        mut config: DbConnectionConfig,
        load_ssh_connection: F,
    ) -> Result<DbConnectionConfig, DbError>
    where
        F: FnOnce(i64) -> Result<Option<StoredConnection>, DbError>,
    {
        let Some(ssh_id) = referenced_ssh_connection_id(&config)? else {
            return Ok(config);
        };
        let ssh_connection = load_ssh_connection(ssh_id)?.ok_or_else(|| {
            DbError::connection(format!("referenced ssh connection not found: {ssh_id}"))
        })?;

        if ssh_connection.connection_type != ConnectionType::SshSftp {
            return Err(DbError::connection(format!(
                "referenced connection {ssh_id} is not an SSH/SFTP connection"
            )));
        }

        config
            .apply_referenced_ssh_tunnel(&ssh_connection)
            .map_err(|error| DbError::connection(format!("failed to apply ssh tunnel: {error}")))?;
        Ok(config)
    }
}

fn referenced_ssh_connection_id(config: &DbConnectionConfig) -> Result<Option<i64>, DbError> {
    let Some(value) = config.extra_params.get("ssh_connection_id") else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }

    value
        .parse::<i64>()
        .map(Some)
        .map_err(|_| DbError::connection(format!("invalid ssh_connection_id: {value}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::{
        ConnectionType, DatabaseType, DbConnectionConfig, SshAuthMethod, SshParams,
        StoredConnection,
    };
    use std::collections::HashMap;

    fn database_config(extra_params: HashMap<String, String>) -> DbConnectionConfig {
        DbConnectionConfig {
            id: "db-1".to_string(),
            database_type: DatabaseType::MySQL,
            name: "prod mysql".to_string(),
            host: "mysql.internal".to_string(),
            port: 3306,
            username: "root".to_string(),
            password: "secret".to_string(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: Some(7),
            proxy: None,
            extra_params,
        }
    }

    fn database_config_with_ssh_ref(ssh_connection_id: &str) -> DbConnectionConfig {
        let mut extra_params = HashMap::new();
        extra_params.insert("ssh_tunnel_enabled".to_string(), "true".to_string());
        extra_params.insert(
            "ssh_connection_id".to_string(),
            ssh_connection_id.to_string(),
        );
        database_config(extra_params)
    }

    fn ssh_connection(id: i64, auth_method: SshAuthMethod) -> StoredConnection {
        let mut connection = StoredConnection::new_ssh(
            "prod-bastion".to_string(),
            SshParams {
                host: "bastion.example.com".to_string(),
                port: 2222,
                username: "deploy".to_string(),
                auth_method,
                connect_timeout: Some(15),
                keepalive_interval: Some(30),
                keepalive_max: Some(3),
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                jump_server: None,
                proxy: None,
            },
            Some(7),
        );
        connection.id = Some(id);
        connection
    }

    #[test]
    fn resolve_with_loader_returns_config_without_reference_unchanged() {
        let config = database_config(HashMap::new());

        let resolved = ConnectionConfigResolver::resolve_with_loader(config.clone(), |_| {
            panic!("loader should not be called without ssh_connection_id")
        })
        .expect("config without ssh reference should resolve");

        assert_eq!(config.id, resolved.id);
        assert_eq!(config.host, resolved.host);
        assert_eq!(config.port, resolved.port);
        assert_eq!(config.extra_params, resolved.extra_params);
    }

    #[test]
    fn resolve_with_loader_applies_referenced_auto_publickey_ssh_connection() {
        let config = database_config_with_ssh_ref("42");
        let ssh = ssh_connection(42, SshAuthMethod::AutoPublicKey);

        let resolved = ConnectionConfigResolver::resolve_with_loader(config, |id| {
            assert_eq!(42, id);
            Ok(Some(ssh.clone()))
        })
        .expect("referenced ssh connection should resolve");

        assert_eq!(
            Some(&"bastion.example.com".to_string()),
            resolved.extra_params.get("ssh_host")
        );
        assert_eq!(
            Some(&"auto_publickey".to_string()),
            resolved.extra_params.get("ssh_auth_type")
        );
        assert_eq!(None, resolved.extra_params.get("ssh_password"));
        assert_eq!(None, resolved.extra_params.get("ssh_private_key_path"));
    }

    #[test]
    fn resolve_with_loader_rejects_missing_referenced_ssh_connection() {
        let config = database_config_with_ssh_ref("42");

        let error = ConnectionConfigResolver::resolve_with_loader(config, |id| {
            assert_eq!(42, id);
            Ok(None)
        })
        .expect_err("missing ssh reference should fail");

        assert!(
            error
                .to_string()
                .contains("referenced ssh connection not found")
        );
    }

    #[test]
    fn resolve_with_loader_rejects_non_ssh_referenced_connection() {
        let config = database_config_with_ssh_ref("42");
        let mut referenced = ssh_connection(42, SshAuthMethod::Agent);
        referenced.connection_type = ConnectionType::Database;

        let error =
            ConnectionConfigResolver::resolve_with_loader(config, |_| Ok(Some(referenced.clone())))
                .expect_err("non-ssh reference should fail");

        assert!(error.to_string().contains("is not an SSH/SFTP connection"));
    }
}
