//! Composite extension manifest 与 native IPC host 的薄适配层。
//!
//! 本 crate 不实现 provider，也不负责进程重启或通用 capability 授权；
//! 它只把 catalog 中已经校验的 IPC binding 转为单次 session 配置，并在
//! spawn 边界复核 command 权限和路径 containment。

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use extension_host::{NegotiationConfig, ProcessRpcSessionConfig, SpawnConfig};
use extension_runtime::RegisteredIpcRuntimeBinding;
use thiserror::Error;

pub mod activation;

pub mod blob_store;
#[cfg(test)]
mod blob_store_tests;
pub mod event_activation;
pub mod event_supervisor;
#[cfg(test)]
mod event_supervisor_tests;
pub mod job_activation;
mod job_activation_state;
#[cfg(test)]
mod job_activation_tests;
pub mod provider_permissions;
pub mod universal_host;

pub use activation::{
    ActivationError, ActivationHandle, ActivationManager, HostApiFactory, ManagedRpcSession,
    ManagedUniversalPluginClient, RuntimeActivationState, RuntimeHealth, RuntimeMonitor,
    RuntimeMonitorConfig, RuntimeMonitorError, RuntimeMonitorEvent, SessionContext, SessionFactory,
    SupervisionPolicy, process_session_factory,
};

pub use blob_store::{
    BlobInfo, BlobOwner, BlobStore, BlobStoreError, BlobStoreLimits, DEFAULT_MAX_BLOB_BYTES,
    DEFAULT_MAX_TOTAL_BLOB_BYTES,
};
pub use event_activation::{
    DEFAULT_MAX_OPEN_EVENT_STREAMS, EventActivationError, EventActivationManager, EventStreamKey,
};
pub use event_supervisor::{
    DEFAULT_EVENT_BRIDGE_CAPACITY, EventStreamBatch, EventStreamSubscription,
    EventStreamSubscriptionConfig,
};
pub use job_activation::{
    JobActivationError, JobActivationHandle, JobActivationManager, RecoveredJob, RetiredJob,
};
pub use provider_permissions::{
    NetworkEndpoint, ProviderPermissionError, ProviderPermissionSet, ResourceOpenAuthorizer,
    SecretReference,
};
pub use universal_host::{MapSecretResolver, SecretResolver, UniversalProviderHost};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PluginAdapterError {
    #[error("unsupported IPC transport `{0}`")]
    UnsupportedTransport(String),
    #[error("IPC shutdown grace {0}ms exceeds the host limit")]
    ShutdownGraceOverflow(u64),
    #[error("IPC command is missing required permission `{0}`")]
    MissingSpawnPermission(String),
    #[error("IPC path `{path}` escapes allowed root `{allowed_root}`")]
    PathEscapesAllowedRoot {
        path: PathBuf,
        allowed_root: PathBuf,
    },
    #[error("failed to resolve IPC path `{path}`: {message}")]
    PathResolution { path: PathBuf, message: String },
}

/// 把静态 IPC binding 转为一次 native process session 的启动配置。
///
/// 本函数在真正生成 spawn 配置前再次校验 command 对应的精确 `spawn:` 权限和
/// symlink containment。其余 permissions 以及 `auto_restart`、
/// `max_restart_attempts` 属于更高层 activation manager 的策略。
pub fn process_session_config(
    binding: &RegisteredIpcRuntimeBinding,
    negotiation: NegotiationConfig,
) -> Result<ProcessRpcSessionConfig, PluginAdapterError> {
    if binding.transport_kind != "local_socket" {
        return Err(PluginAdapterError::UnsupportedTransport(
            binding.transport_kind.clone(),
        ));
    }
    validate_spawn_boundary(binding)?;
    let shutdown_grace_ms = u32::try_from(binding.shutdown_grace_ms)
        .map_err(|_| PluginAdapterError::ShutdownGraceOverflow(binding.shutdown_grace_ms))?;

    let mut spawn = SpawnConfig::new(binding.command.clone()).with_args(binding.args.clone());
    if let Some(working_dir) = &binding.working_dir {
        spawn = spawn
            .with_cwd(working_dir.clone())
            .with_cwd_root(binding.extension_root.clone());
    }
    if binding.required_spawn_permission.starts_with("spawn:./") {
        spawn = spawn.with_program_root(binding.extension_root.clone());
    } else {
        spawn = spawn.with_program_root("/usr/bin");
    }
    for (key, value) in &binding.env {
        spawn = spawn.with_env(key.clone(), value.clone());
    }
    if let Some(timeout_ms) = binding.connect_timeout_ms {
        spawn = spawn.with_ready_timeout(Duration::from_millis(timeout_ms));
    }

    Ok(ProcessRpcSessionConfig::new(spawn, negotiation)
        .with_shutdown_grace_ms(shutdown_grace_ms)
        .with_label(binding.runtime_key.clone()))
}

fn validate_spawn_boundary(
    binding: &RegisteredIpcRuntimeBinding,
) -> Result<(), PluginAdapterError> {
    if !binding
        .permissions
        .iter()
        .any(|permission| permission == &binding.required_spawn_permission)
    {
        return Err(PluginAdapterError::MissingSpawnPermission(
            binding.required_spawn_permission.clone(),
        ));
    }

    ensure_path_within(&binding.extension_root, &binding.extension_root)?;
    if let Some(working_dir) = &binding.working_dir {
        ensure_path_within(&binding.extension_root, working_dir)?;
    }
    if binding.required_spawn_permission.starts_with("spawn:./") {
        ensure_path_within(&binding.extension_root, &binding.command)?;
    } else {
        ensure_path_within(Path::new("/usr/bin"), &binding.command)?;
    }
    Ok(())
}

fn ensure_path_within(allowed_root: &Path, candidate: &Path) -> Result<(), PluginAdapterError> {
    let canonical_root =
        allowed_root
            .canonicalize()
            .map_err(|error| PluginAdapterError::PathResolution {
                path: allowed_root.to_path_buf(),
                message: error.to_string(),
            })?;
    let mut existing = candidate;
    while !existing.exists() {
        let Some(parent) = existing.parent() else {
            return Err(PluginAdapterError::PathResolution {
                path: candidate.to_path_buf(),
                message: "no existing ancestor".into(),
            });
        };
        existing = parent;
    }
    let canonical_existing =
        existing
            .canonicalize()
            .map_err(|error| PluginAdapterError::PathResolution {
                path: existing.to_path_buf(),
                message: error.to_string(),
            })?;
    if canonical_existing.starts_with(&canonical_root) {
        return Ok(());
    }
    Err(PluginAdapterError::PathEscapesAllowedRoot {
        path: candidate.to_path_buf(),
        allowed_root: allowed_root.to_path_buf(),
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
