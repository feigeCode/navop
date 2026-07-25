//! Redis domain contracts and backend selection without GPUI dependencies.

use std::path::PathBuf;

rust_i18n::i18n!("../redis_view/locales", fallback = "zh-CN");

pub mod connection;
pub mod ipc;
pub mod pubsub;
pub mod types;

#[cfg(feature = "builtin-redis")]
mod builtin;
#[cfg(feature = "builtin-redis")]
mod builtin_pubsub;

#[cfg(feature = "builtin-redis")]
pub use builtin::RedisConnectionImpl as BuiltinRedisConnection;

#[doc(hidden)]
pub fn parse_command_args_for_test(command: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for ch in command.chars() {
        if escaped {
            current.push(if in_single || in_double {
                match ch {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                }
            } else {
                ch
            });
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}
pub use connection::RedisConnection;
pub use ipc::IpcRedisConnection;
pub use pubsub::{PubSubMessage, PubSubMessageKind, RedisPubSubHandle, SubscriptionCommand};
pub use types::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedisBackendKind {
    Ipc,
    Builtin,
}

pub const DEFAULT_REDIS_DRIVER_ID: &str = "redis";
pub const MINIMUM_REDIS_DRIVER_VERSION: &str = "0.1.2";

pub fn validate_native_driver_version(
    manifest: &extension_host::NativeDriverManifest,
) -> Result<(), RedisError> {
    let installed = semver::Version::parse(manifest.version.trim()).map_err(|_| {
        RedisError::connection(format!(
            "Redis native driver `{}` has invalid version `{}`; host requires version >= {}",
            manifest.id,
            display_native_driver_version(&manifest.version),
            MINIMUM_REDIS_DRIVER_VERSION
        ))
    })?;
    let required = semver::Version::parse(MINIMUM_REDIS_DRIVER_VERSION)
        .expect("minimum Redis native driver version must be valid semver");
    if installed < required {
        return Err(RedisError::connection(format!(
            "Redis native driver `{}` is incompatible: host requires version >= {}, installed {}",
            manifest.id, required, installed
        )));
    }
    Ok(())
}

fn display_native_driver_version(version: &str) -> &str {
    let version = version.trim();
    if version.is_empty() {
        "<empty>"
    } else {
        version
    }
}

pub const fn default_backend_kind() -> RedisBackendKind {
    #[cfg(feature = "builtin-redis")]
    {
        RedisBackendKind::Builtin
    }
    #[cfg(not(feature = "builtin-redis"))]
    {
        RedisBackendKind::Ipc
    }
}

#[derive(Clone)]
pub enum RedisConnectionFactory {
    Ipc(Box<extension_host::NativeDriverManifest>),
    InstalledRegistry(PathBuf),
    Unavailable,
    #[cfg(feature = "builtin-redis")]
    Builtin,
}

impl RedisConnectionFactory {
    pub fn from_installed_root(root: impl Into<PathBuf>) -> Self {
        Self::InstalledRegistry(root.into())
    }

    pub fn backend_kind(&self) -> RedisBackendKind {
        match self {
            Self::Ipc(_) | Self::InstalledRegistry(_) => RedisBackendKind::Ipc,
            Self::Unavailable => RedisBackendKind::Ipc,
            #[cfg(feature = "builtin-redis")]
            Self::Builtin => RedisBackendKind::Builtin,
        }
    }

    pub async fn create(
        &self,
        config: RedisConnectionConfig,
    ) -> Result<Box<dyn RedisConnection>, RedisError> {
        match self {
            Self::Ipc(manifest) => Ok(Box::new(IpcRedisConnection::start(manifest, config).await?)),
            Self::InstalledRegistry(root) => {
                let registry = extension_host::NativeDriverRegistry::load_from_dir(root)
                    .map_err(|error| RedisError::connection(error.to_string()))?;
                let manifest =
                    registry
                        .find("redis", DEFAULT_REDIS_DRIVER_ID)
                        .ok_or_else(|| {
                            RedisError::connection("Redis native driver is not installed")
                        })?;
                Ok(Box::new(
                    IpcRedisConnection::start(&manifest, config).await?,
                ))
            }
            Self::Unavailable => Err(RedisError::Connection {
                message: "Redis native driver is not installed".into(),
                source: None,
            }),
            #[cfg(feature = "builtin-redis")]
            Self::Builtin => {
                let mut connection = BuiltinRedisConnection::new(config);
                connection.connect().await?;
                Ok(Box::new(connection))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backend_matches_feature_contract() {
        #[cfg(feature = "builtin-redis")]
        assert_eq!(RedisBackendKind::Builtin, default_backend_kind());
        #[cfg(not(feature = "builtin-redis"))]
        assert_eq!(RedisBackendKind::Ipc, default_backend_kind());
    }

    #[test]
    fn installed_registry_factory_reports_ipc_backend() {
        assert_eq!(
            RedisBackendKind::Ipc,
            RedisConnectionFactory::from_installed_root("database_drivers").backend_kind()
        );
    }

    #[test]
    fn redis_native_driver_requires_the_scan_compatible_release() {
        assert_eq!("0.1.2", MINIMUM_REDIS_DRIVER_VERSION);

        let old = native_driver_manifest("0.1.1");
        let error = validate_native_driver_version(&old)
            .expect_err("the incompatible connection-manager release must be rejected");
        assert!(error.to_string().contains("requires version >= 0.1.2"));
        assert!(error.to_string().contains("installed 0.1.1"));

        validate_native_driver_version(&native_driver_manifest("0.1.2"))
            .expect("the fixed driver release must be accepted");
        validate_native_driver_version(&native_driver_manifest("0.2.0"))
            .expect("newer compatible driver releases must be accepted");
    }

    fn native_driver_manifest(version: &str) -> extension_host::NativeDriverManifest {
        serde_json::from_value(serde_json::json!({
            "id": "redis",
            "name": "Redis",
            "version": version,
            "api": "redis",
            "protocol_version": "1.0",
            "entry": { "command": "./redis-driver" },
            "transport": { "name": "redis.sock" }
        }))
        .expect("test manifest should deserialize")
    }

    #[cfg(feature = "builtin-redis")]
    #[test]
    fn builtin_factory_reports_builtin_backend() {
        assert_eq!(
            RedisBackendKind::Builtin,
            RedisConnectionFactory::Builtin.backend_kind()
        );
    }
}
