//! 与具体业务无关的 native IPC manifest。
//!
//! SQL 数据库的 `db::ipc::IpcDriverManifest` 保持向后兼容；新的 Redis、
//! MongoDB 和后续 native sidecar 使用这里定义的公共进程/transport/API
//! 描述，不需要依赖 `db`。

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{HostError, HostResult};

pub const NATIVE_DRIVER_MANIFEST_FILE: &str = "driver.json";

fn default_api() -> String {
    "native".to_string()
}

fn default_protocol_version() -> String {
    "1.0".to_string()
}

fn default_process_scope() -> NativeDriverProcessScope {
    NativeDriverProcessScope::Connection
}

fn default_true() -> bool {
    true
}

fn default_max_restart_attempts() -> u32 {
    3
}

/// 通用 native sidecar manifest。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeDriverManifest {
    pub id: String,
    pub name: String,
    #[serde(default = "default_api")]
    pub api: String,
    #[serde(default = "default_protocol_version")]
    pub protocol_version: String,
    pub entry: NativeDriverEntry,
    pub transport: NativeDriverTransport,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 领域驱动自定义的兼容信息；通用层只负责透传和版本存在性校验。
    #[serde(default)]
    pub compatibility: Value,
    #[serde(default)]
    pub process: NativeDriverProcessPolicy,
    #[serde(skip)]
    pub manifest_dir: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct NativeDriverRegistry {
    drivers: Vec<NativeDriverManifest>,
}

impl NativeDriverRegistry {
    pub fn from_drivers(mut drivers: Vec<NativeDriverManifest>) -> Self {
        drivers.sort_by(|left, right| {
            left.api
                .cmp(&right.api)
                .then_with(|| left.id.cmp(&right.id))
        });
        Self { drivers }
    }

    pub fn load_from_dir(root: &std::path::Path) -> HostResult<Self> {
        if !root.exists() {
            return Ok(Self::default());
        }
        let mut drivers = Vec::new();
        for entry in std::fs::read_dir(root).map_err(HostError::Io)? {
            let entry = entry.map_err(HostError::Io)?;
            if !entry.file_type().map_err(HostError::Io)?.is_dir() {
                continue;
            }
            let path = entry.path();
            match Self::load_driver_from_dir(&path) {
                Ok(Some(driver)) => drivers.push(driver),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %error,
                        "skipping invalid native driver manifest"
                    );
                }
            }
        }
        Ok(Self::from_drivers(drivers))
    }

    pub fn load_driver_from_dir(dir: &std::path::Path) -> HostResult<Option<NativeDriverManifest>> {
        let direct = dir.join(NATIVE_DRIVER_MANIFEST_FILE);
        let manifest_path = if direct.is_file() {
            Some(direct)
        } else {
            let mut nested = std::fs::read_dir(dir)
                .map_err(HostError::Io)?
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    entry
                        .file_type()
                        .ok()
                        .filter(|kind| kind.is_dir())
                        .map(|_| entry.path().join(NATIVE_DRIVER_MANIFEST_FILE))
                })
                .filter(|path| path.is_file());
            let first = nested.next();
            if nested.next().is_some() {
                return Err(HostError::Config(format!(
                    "multiple nested native driver manifests found in {}",
                    dir.display()
                )));
            }
            first
        };
        let Some(manifest_path) = manifest_path else {
            return Ok(None);
        };
        let content = std::fs::read_to_string(&manifest_path).map_err(HostError::Io)?;
        let mut manifest: NativeDriverManifest = serde_json::from_str(&content)?;
        manifest.manifest_dir = manifest_path.parent().unwrap_or(dir).to_path_buf();
        manifest.validate()?;
        Ok(Some(manifest))
    }

    pub fn drivers(&self) -> &[NativeDriverManifest] {
        &self.drivers
    }

    pub fn find(&self, api: &str, driver_id: &str) -> Option<NativeDriverManifest> {
        self.drivers
            .iter()
            .find(|driver| driver.api == api && driver.id == driver_id)
            .cloned()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeDriverEntry {
    pub command: String,
    #[serde(default)]
    pub commands: HashMap<String, String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeDriverTransport {
    pub name: String,
    #[serde(default)]
    pub connect_timeout_ms: Option<u64>,
}

impl NativeDriverTransport {
    pub const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 5_000;

    pub fn local_socket(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            connect_timeout_ms: None,
        }
    }

    pub fn connect_timeout_ms(&self) -> u64 {
        self.connect_timeout_ms
            .unwrap_or(Self::DEFAULT_CONNECT_TIMEOUT_MS)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeDriverProcessPolicy {
    #[serde(default = "default_process_scope")]
    pub scope: NativeDriverProcessScope,
    #[serde(default)]
    pub auto_restart: bool,
    #[serde(default = "default_max_restart_attempts")]
    pub max_restart_attempts: u32,
    #[serde(default = "default_true")]
    pub kill_on_drop: bool,
}

impl Default for NativeDriverProcessPolicy {
    fn default() -> Self {
        Self {
            scope: default_process_scope(),
            auto_restart: false,
            max_restart_attempts: default_max_restart_attempts(),
            kill_on_drop: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeDriverProcessScope {
    #[default]
    Connection,
    Shared,
}

impl NativeDriverManifest {
    /// Build the generic process/session configuration shared by all native
    /// sidecar APIs. Domain adapters only need to supply host/instance ids.
    pub fn process_session_config(
        &self,
        host_version: impl Into<String>,
        instance_id: impl Into<String>,
    ) -> crate::ProcessRpcSessionConfig {
        let cwd = self.command_working_dir();
        let command = self.command_for_platform(current_platform_key());
        let command_path = std::path::Path::new(command);
        let program =
            if command_path.is_absolute() || (!command.contains('/') && !command.contains('\\')) {
                command_path.to_path_buf()
            } else {
                cwd.join(command_path)
            };
        let mut spawn = crate::process::SpawnConfig::new(program)
            .with_args(self.entry.args.clone())
            .with_cwd(cwd)
            .with_transport(crate::process::SpawnTransport::LocalSocket {
                name: crate::process::default_socket_name(),
            })
            .with_ready_timeout(std::time::Duration::from_millis(
                self.transport.connect_timeout_ms().max(1_000),
            ));
        for (key, value) in &self.entry.env {
            spawn = spawn.with_env(key.clone(), value.clone());
        }
        let negotiation = crate::negotiation::NegotiationConfig::new(host_version, instance_id)
            .offer_api(self.api.clone(), self.protocol_version.clone())
            .with_handshake_timeout(std::time::Duration::from_millis(
                self.transport.connect_timeout_ms().max(5_000),
            ));
        crate::ProcessRpcSessionConfig::new(spawn, negotiation).with_label(self.id.clone())
    }

    pub fn validate(&self) -> HostResult<()> {
        if self.id.trim().is_empty() || self.name.trim().is_empty() {
            return Err(HostError::Config(
                "native driver id and name are required".to_string(),
            ));
        }
        if self.api.trim().is_empty() || self.protocol_version.trim().is_empty() {
            return Err(HostError::Config(format!(
                "native driver `{}` api and protocol_version are required",
                self.id
            )));
        }
        if self.protocol_version != extension_protocol::WIRE_PROTOCOL_VERSION {
            return Err(HostError::Incompatible(format!(
                "native driver `{}` uses unsupported protocol version `{}`",
                self.id, self.protocol_version
            )));
        }
        if self.entry.command.trim().is_empty() {
            return Err(HostError::Config(format!(
                "native driver `{}` command is required",
                self.id
            )));
        }
        if self.transport.name.trim().is_empty() {
            return Err(HostError::Config(format!(
                "native driver `{}` transport name is required",
                self.id
            )));
        }
        for method in &self.methods {
            if !crate::manifest::is_allowed_method(method) {
                return Err(HostError::Incompatible(format!(
                    "native driver `{}` declares unknown IPC method `{method}`",
                    self.id
                )));
            }
        }
        Ok(())
    }

    pub fn command_for_platform(&self, platform: &str) -> &str {
        self.entry
            .commands
            .get(platform)
            .or_else(|| self.entry.commands.get("default"))
            .map(String::as_str)
            .unwrap_or(self.entry.command.as_str())
    }

    pub fn command_working_dir(&self) -> PathBuf {
        match self.entry.working_dir.as_deref() {
            Some(path) if !path.trim().is_empty() => {
                let path = std::path::Path::new(path);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    self.manifest_dir.join(path)
                }
            }
            _ => self.manifest_dir.clone(),
        }
    }
}

fn current_platform_key() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "default"
    }
}

fn is_allowed_method(method: &str) -> bool {
    // 业务 namespace 的正式 method 会逐步加入 extension_protocol::method；
    // `x/` 保留给尚未进入公共协议的 sidecar 私有能力。
    extension_protocol::method::is_allowed_declaration(method)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_json() -> &'static str {
        r#"{
            "id": "redis",
            "name": "Redis",
            "api": "redis",
            "protocol_version": "1.0",
            "entry": {"command": "./redis-driver"},
            "transport": {"name": "redis.sock"},
            "methods": ["conn/open", "x/redis/command"],
            "compatibility": {"server": {"min": "5.0"}},
            "process": {"scope": "connection", "auto_restart": false}
        }"#
    }

    #[test]
    fn parses_explicit_api_and_driver_policy() {
        let manifest: NativeDriverManifest = serde_json::from_str(manifest_json()).unwrap();

        assert_eq!("redis", manifest.api);
        assert_eq!("1.0", manifest.protocol_version);
        assert_eq!(NativeDriverProcessScope::Connection, manifest.process.scope);
        assert!(!manifest.process.auto_restart);
        assert_eq!("./redis-driver", manifest.command_for_platform("default"));
        manifest.validate().unwrap();
    }

    #[test]
    fn missing_api_uses_generic_default_without_becoming_sql() {
        let value = serde_json::json!({
            "id": "demo",
            "name": "Demo",
            "entry": {"command": "demo"},
            "transport": {"name": "demo.sock"}
        });
        let manifest: NativeDriverManifest = serde_json::from_value(value).unwrap();

        assert_eq!("native", manifest.api);
        assert_eq!("1.0", manifest.protocol_version);
        assert_eq!(NativeDriverProcessScope::Connection, manifest.process.scope);
        manifest.validate().unwrap();
    }

    #[test]
    fn rejects_missing_required_fields_and_unknown_methods() {
        let mut manifest: NativeDriverManifest = serde_json::from_str(manifest_json()).unwrap();
        manifest.entry.command.clear();
        assert!(matches!(manifest.validate(), Err(HostError::Config(_))));

        let mut manifest: NativeDriverManifest = serde_json::from_str(manifest_json()).unwrap();
        manifest.methods = vec!["redis/unknown".into()];
        assert!(matches!(
            manifest.validate(),
            Err(HostError::Incompatible(_))
        ));

        let mut manifest: NativeDriverManifest = serde_json::from_str(manifest_json()).unwrap();
        manifest.protocol_version = "0.1".into();
        assert!(matches!(
            manifest.validate(),
            Err(HostError::Incompatible(_))
        ));
    }

    #[test]
    fn resolves_relative_working_directory_from_manifest_dir() {
        let mut manifest: NativeDriverManifest = serde_json::from_str(manifest_json()).unwrap();
        manifest.manifest_dir = PathBuf::from("/tmp/driver");
        manifest.entry.working_dir = Some("runtime".into());

        assert_eq!(
            PathBuf::from("/tmp/driver/runtime"),
            manifest.command_working_dir()
        );
    }
}
