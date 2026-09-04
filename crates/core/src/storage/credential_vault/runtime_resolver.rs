use anyhow::{Context, Result, anyhow, bail};

use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionRepository, ConnectionType, MongoDBParams, MqttParams, RedisParams, StoredConnection,
};

impl ConnectionRepository {
    /// Resolves every credential needed by an actual connection attempt.
    ///
    /// This includes the primary connection and a referenced SSH tunnel
    /// connection. The returned clone can contain plaintext secrets and must
    /// never be persisted, synchronized, logged, shared, or exported.
    pub fn resolve_runtime_connection(
        &self,
        connection: &StoredConnection,
    ) -> Result<StoredConnection> {
        let credentials = self.credential_repository();
        let resolved = credentials.resolve_connection(connection)?;
        match resolved.connection_type {
            ConnectionType::Database => self.resolve_database_tunnel(resolved),
            ConnectionType::Redis => self.resolve_redis_tunnel(resolved),
            ConnectionType::MongoDB => self.resolve_mongodb_tunnel(resolved),
            ConnectionType::Mqtt => self.resolve_mqtt_tunnel(resolved),
            _ => Ok(resolved),
        }
    }

    fn resolve_database_tunnel(
        &self,
        mut connection: StoredConnection,
    ) -> Result<StoredConnection> {
        let mut params = connection.to_db_connection()?;
        if !params.get_param_bool("ssh_tunnel_enabled") {
            return Ok(connection);
        }
        let Some(id) = database_ssh_connection_id(&params)? else {
            return Ok(connection);
        };
        let ssh = self.resolve_referenced_ssh(id)?;
        params
            .apply_referenced_ssh_tunnel(&ssh)
            .context("failed to apply referenced SSH tunnel")?;
        connection.params = serde_json::to_string(&params)?;
        Ok(connection)
    }

    fn resolve_redis_tunnel(&self, mut connection: StoredConnection) -> Result<StoredConnection> {
        let mut params = connection.to_redis_params()?;
        let Some(id) = enabled_tunnel_id(&params) else {
            return Ok(connection);
        };
        let ssh = self.resolve_referenced_ssh(id)?;
        params
            .apply_referenced_ssh_tunnel(&ssh)
            .context("failed to apply referenced Redis SSH tunnel")?;
        connection.params = serde_json::to_string(&params)?;
        Ok(connection)
    }

    fn resolve_mongodb_tunnel(&self, mut connection: StoredConnection) -> Result<StoredConnection> {
        let mut params = connection.to_mongodb_params()?;
        let Some(id) = enabled_mongodb_tunnel_id(&params) else {
            return Ok(connection);
        };
        let ssh = self.resolve_referenced_ssh(id)?;
        params
            .apply_referenced_ssh_tunnel(&ssh)
            .context("failed to apply referenced MongoDB SSH tunnel")?;
        connection.params = serde_json::to_string(&params)?;
        Ok(connection)
    }

    fn resolve_mqtt_tunnel(&self, mut connection: StoredConnection) -> Result<StoredConnection> {
        let mut params = connection.to_mqtt_params()?;
        let Some(id) = enabled_mqtt_tunnel_id(&params) else {
            return Ok(connection);
        };
        let ssh = self.resolve_referenced_ssh(id)?;
        params
            .apply_referenced_ssh_tunnel(&ssh)
            .context("failed to apply referenced MQTT SSH tunnel")?;
        connection.params = serde_json::to_string(&params)?;
        Ok(connection)
    }

    fn resolve_referenced_ssh(&self, id: i64) -> Result<StoredConnection> {
        let connection = self
            .get(id)?
            .ok_or_else(|| anyhow!("referenced SSH connection not found: {id}"))?;
        if connection.connection_type != ConnectionType::SshSftp {
            bail!("referenced connection {id} is not an SSH/SFTP connection");
        }
        self.credential_repository().resolve_connection(&connection)
    }
}

fn database_ssh_connection_id(params: &crate::storage::DbConnectionConfig) -> Result<Option<i64>> {
    let Some(value) = params.get_param("ssh_connection_id") else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse::<i64>()
        .map(Some)
        .with_context(|| format!("invalid ssh_connection_id: {value}"))
}

fn enabled_tunnel_id(params: &RedisParams) -> Option<i64> {
    params
        .ssh_tunnel
        .as_ref()
        .filter(|tunnel| tunnel.enabled)
        .and_then(|tunnel| tunnel.connection_id)
}

fn enabled_mongodb_tunnel_id(params: &MongoDBParams) -> Option<i64> {
    params
        .ssh_tunnel
        .as_ref()
        .filter(|tunnel| tunnel.enabled)
        .and_then(|tunnel| tunnel.connection_id)
}

fn enabled_mqtt_tunnel_id(params: &MqttParams) -> Option<i64> {
    params
        .ssh_tunnel
        .as_ref()
        .filter(|tunnel| tunnel.enabled)
        .and_then(|tunnel| tunnel.connection_id)
}
