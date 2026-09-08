use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value};

use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionRepository, ConnectionType, CredentialReference, ExtensionConnectionParams,
    MongoDBParams, MqttParams, RedisParams, StoredConnection,
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
            ConnectionType::Extension => self.resolve_extension_tunnel(resolved),
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

    /// 解析 extension 连接 Auth 字段里的密码簿引用为运行时明文。
    ///
    /// 与数据库/中间件走同一凭据解析机制(driver.json 驱动同样以明文收到
    /// 账号密码):`config[field].credential_reference` 被替换为
    /// `config[field] = {"username": ..., "password": ...}`,随 open 载荷送达
    /// provider。仅用于内存中的连接尝试,禁止落盘/同步/日志。
    pub fn resolve_extension_runtime_config(&self, config: &mut Map<String, Value>) -> Result<()> {
        resolve_extension_auth_objects(config, |reference| {
            self.fetch_referenced_credential(reference)
        })
    }

    fn resolve_extension_tunnel(&self, connection: StoredConnection) -> Result<StoredConnection> {
        let params = connection.to_extension_params()?;
        let mut config = params.config;
        self.resolve_extension_runtime_config(&mut config)?;
        let params = ExtensionConnectionParams::new(
            params.extension_id,
            params.contribution_id,
            config,
            params.secrets,
        )?;
        let mut connection = connection;
        connection.params = serde_json::to_string(&params)?;
        Ok(connection)
    }

    fn fetch_referenced_credential(
        &self,
        reference: &CredentialReference,
    ) -> Result<(Option<String>, Option<String>)> {
        let credentials = self.credential_repository();
        let entry = match reference.credential_cloud_id.as_deref() {
            Some(cloud_id) => credentials.get_by_cloud_id(cloud_id)?,
            None => credentials.get_plaintext(reference.credential_id)?,
        }
        .ok_or_else(|| {
            let id = reference
                .credential_cloud_id
                .clone()
                .unwrap_or_else(|| reference.credential_id.to_string());
            anyhow!("referenced credential `{id}` not found")
        })?;
        Ok((entry.username, entry.password))
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

/// 把 config 中形如 `{"credential_reference": {...}}` 的 Auth 字段解析为运行时形态:
/// `config[field] = {"username": ..., "password": ...}`(与 driver.json 驱动收到
/// 明文凭据一致的统一机制)。`fetch` 由调用方提供真实密码簿查找,便于纯函数测试。
fn resolve_extension_auth_objects(
    config: &mut Map<String, Value>,
    fetch: impl Fn(&CredentialReference) -> Result<(Option<String>, Option<String>)>,
) -> Result<()> {
    let mut rewrite = Vec::new();
    for (field, value) in config.iter() {
        let Some(reference) = value
            .as_object()
            .and_then(|object| object.get("credential_reference"))
            .and_then(|value| serde_json::from_value::<CredentialReference>(value.clone()).ok())
        else {
            continue;
        };
        let (username, password) = fetch(&reference)?;
        let mut runtime = Map::new();
        if reference.username
            && let Some(username) = username.filter(|value| !value.is_empty())
        {
            runtime.insert("username".into(), Value::String(username));
        }
        if reference.password
            && let Some(password) = password.filter(|value| !value.is_empty())
        {
            runtime.insert("password".into(), Value::String(password));
        }
        rewrite.push((field.clone(), Value::Object(runtime)));
    }
    for (field, runtime) in rewrite {
        config.insert(field, runtime);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth_reference(credential_id: i64) -> Value {
        serde_json::json!({
            "credential_reference": {
                "credential_id": credential_id,
                "username": true,
                "password": true,
            }
        })
    }

    #[test]
    fn resolves_reference_into_username_and_password() {
        let mut config = Map::from_iter([("auth".into(), auth_reference(7))]);
        resolve_extension_auth_objects(&mut config, |reference| {
            assert_eq!(7, reference.credential_id);
            Ok((Some("root".into()), Some("s3cret".into())))
        })
        .unwrap();
        assert_eq!(config["auth"]["username"], "root");
        assert_eq!(config["auth"]["password"], "s3cret");
        assert!(config["auth"].get("credential_reference").is_none());
    }

    #[test]
    fn leaves_non_auth_and_manual_fields_untouched() {
        let mut config = Map::from_iter([
            ("host".into(), serde_json::json!("example.com")),
            ("auth".into(), serde_json::json!({ "username": "manual" })),
        ]);
        resolve_extension_auth_objects(&mut config, |_| unreachable!()).unwrap();
        assert_eq!(config["host"], "example.com");
        assert_eq!(config["auth"]["username"], "manual");
        assert!(config["auth"].get("password").is_none());
    }

    #[test]
    fn honors_reference_field_flags() {
        let mut reference = auth_reference(3);
        reference["credential_reference"]["password"] = serde_json::json!(false);
        let mut config = Map::from_iter([("auth".into(), reference)]);
        resolve_extension_auth_objects(&mut config, |_| {
            Ok((Some("user".into()), Some("pw".into())))
        })
        .unwrap();
        assert_eq!(config["auth"]["username"], "user");
        assert!(
            config["auth"].get("password").is_none(),
            "password 标志关闭时不注入密码"
        );
    }

    #[test]
    fn propagates_missing_credential_error() {
        let mut config = Map::from_iter([("auth".into(), auth_reference(99))]);
        let error = resolve_extension_auth_objects(&mut config, |_| {
            Err(anyhow!("referenced credential `99` not found"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("not found"));
    }
}
