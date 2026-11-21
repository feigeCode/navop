use std::path::PathBuf;
use std::process::Stdio;
use std::sync::OnceLock;

use process_util::configure_tokio_background_child;
use tokio::process::{Child, Command};
use x11_forwarding::{MagicCookie, ServerEndpoint};

use crate::X11ComponentError;
use crate::command::{CommandSpec, EndpointPlan};
use crate::installer::InstalledComponent;
use crate::lock::ComponentLock;
use crate::paths::component_root;
use crate::process::{
    PidFile, cleanup_orphan, install_exit_handler, instance_path, register_exit_cleanup,
};
use crate::runtime::{PersistentEndpoint, RuntimeIdentity};
use crate::xauth::write_xauth;

#[derive(Clone, Debug)]
pub struct LaunchSpec {
    pub root: PathBuf,
    pub executable: PathBuf,
    pub version: String,
    pub min_vulkan: String,
    pub identity: RuntimeIdentity,
}

struct LaunchInput {
    data_dir: PathBuf,
    executable: PathBuf,
    version: String,
    min_vulkan: String,
    manual: bool,
}

impl LaunchSpec {
    pub fn installed(
        data_dir: PathBuf,
        component: InstalledComponent,
    ) -> Result<Self, X11ComponentError> {
        Self::build(LaunchInput {
            data_dir,
            executable: component.executable,
            version: component.version,
            min_vulkan: component.min_vulkan,
            manual: false,
        })
    }

    pub fn manual(data_dir: PathBuf, executable: PathBuf) -> Result<Self, X11ComponentError> {
        Self::build(LaunchInput {
            data_dir,
            executable,
            version: "manual".into(),
            min_vulkan: "1.3.0".into(),
            manual: true,
        })
    }

    fn build(input: LaunchInput) -> Result<Self, X11ComponentError> {
        let root = component_root(&input.data_dir);
        let identity = load_or_generate_identity(&root, input.manual)?;
        Ok(Self {
            root,
            executable: input.executable,
            version: input.version,
            min_vulkan: input.min_vulkan,
            identity,
        })
    }

    pub fn endpoint(&self) -> ServerEndpoint {
        self.identity.endpoint.to_endpoint()
    }

    pub fn cookie(&self) -> Result<MagicCookie, X11ComponentError> {
        self.identity.cookie()
    }

    /// spawn 前必须在组件锁内统一身份；首次生成的身份也在此落盘。
    pub fn reload_identity_for_spawn(&mut self) -> Result<(), X11ComponentError> {
        let path = self.root.join("runtime.json");
        if path.exists() {
            self.identity = RuntimeIdentity::read(&path)?;
        } else {
            self.identity.write(&path)?;
        }
        Ok(())
    }

    /// 测试及不持锁调用方使用的包装；启动重试必须调用 locked 版本。
    #[cfg(test)]
    pub fn refresh_endpoint_after_conflict(&mut self) -> Result<(), X11ComponentError> {
        let lock = ComponentLock::acquire(&self.root).map_err(process_error)?;
        self.refresh_endpoint_after_conflict_locked(&lock)
    }

    /// 调用者必须持有当前组件锁，失败记录清理与 identity 更新不得插入其他启动。
    pub fn refresh_endpoint_after_conflict_locked(
        &mut self,
        _lock: &ComponentLock,
    ) -> Result<(), X11ComponentError> {
        let plan = match self.endpoint() {
            ServerEndpoint::Inet { .. } => EndpointPlan::random_tcp()?,
            ServerEndpoint::Unix(_) => EndpointPlan::random(true)?,
        };
        self.identity.endpoint = PersistentEndpoint::from_endpoint(&plan.endpoint());
        self.identity.write(&self.root.join("runtime.json"))
    }

    fn command_spec(&self) -> CommandSpec {
        let xauth = self.root.join("xauth");
        match self.endpoint() {
            ServerEndpoint::Unix(_) => {
                CommandSpec::unix(self.executable.clone(), &self.endpoint(), xauth)
            }
            endpoint @ ServerEndpoint::Inet { .. } if self.version == "manual" => {
                CommandSpec::manual_upstream_tcp(self.executable.clone(), &endpoint, xauth)
            }
            endpoint @ ServerEndpoint::Inet { .. } => {
                CommandSpec::installed_tcp(self.executable.clone(), &endpoint, xauth)
            }
        }
    }
}

pub struct LaunchedChild {
    pub child: Child,
    pub pid: PidFile,
    pub spec: LaunchSpec,
}

pub async fn spawn_once(spec: &LaunchSpec) -> Result<LaunchedChild, X11ComponentError> {
    ensure_exit_handler()?;
    cleanup_orphan(&spec.root.join("yserver.pid"))?;
    let mut command = tokio_command(spec)?;
    let mut child = command.spawn().map_err(process_error)?;
    let child_id = child
        .id()
        .ok_or_else(|| X11ComponentError::Process("yserver 缺少 pid".into()))?;
    let pid = match PidFile::capture(child_id, &spec.executable, &spec.version) {
        Ok(pid) => pid,
        Err(error) => {
            let _ = child.kill().await;
            return Err(error);
        }
    };
    if let Err(error) = write_startup_records(spec, &pid) {
        let _ = child.kill().await;
        return Err(error);
    }
    register_exit_cleanup(pid.clone());
    Ok(LaunchedChild {
        child,
        pid,
        spec: spec.clone(),
    })
}

fn write_startup_records(spec: &LaunchSpec, pid: &PidFile) -> Result<(), X11ComponentError> {
    pid.write(&instance_path(&spec.root, pid))?;
    if let Err(error) = pid.write(&spec.root.join("yserver.pid")) {
        let _ = std::fs::remove_file(instance_path(&spec.root, pid));
        return Err(error);
    }
    Ok(())
}

fn tokio_command(spec: &LaunchSpec) -> Result<Command, X11ComponentError> {
    let command_spec = spec.command_spec();
    command_spec.validate_secure_endpoint()?;
    let cookie = spec.cookie()?;
    let xauth = spec.root.join("xauth");
    write_xauth(&xauth, &spec.endpoint(), &cookie)?;
    let mut command = Command::new(command_spec.executable());
    command
        .args(command_spec.arguments())
        .env("XAUTHORITY", &xauth)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    configure_tokio_background_child(&mut command);
    Ok(command)
}

fn load_or_generate_identity(
    root: &PathBuf,
    manual: bool,
) -> Result<RuntimeIdentity, X11ComponentError> {
    let path = root.join("runtime.json");
    if path.exists() {
        return RuntimeIdentity::read(&path);
    }
    let plan = EndpointPlan::random(manual)?;
    Ok(RuntimeIdentity::new(
        &plan.endpoint(),
        &MagicCookie::generate(),
    ))
}

fn ensure_exit_handler() -> Result<(), X11ComponentError> {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    if INSTALLED.get().is_none() {
        install_exit_handler()?;
        let _ = INSTALLED.set(());
    }
    Ok(())
}

#[cfg(test)]
mod tests;

fn process_error(error: std::io::Error) -> X11ComponentError {
    X11ComponentError::Process(error.to_string())
}
