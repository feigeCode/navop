use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::Duration;

use tokio::sync::Mutex;
use x11_forwarding::{X11Proxy, probe_endpoint};

use crate::launch::{LaunchSpec, LaunchedChild, spawn_once};
use crate::lock::ComponentLock;
use crate::metrics::{MetricEvent, record};
use crate::process::{kill_owned, remove_owned_files, remove_pidfile_if_owned};
use crate::restart::RestartBudget;
use crate::state::{disable_embedded, note_restart, set_active_version, update_status};
use crate::vulkan::preflight;
use crate::{X11ComponentError, X11ForwardingStatus, X11Resolution};

struct RunningServer {
    launched: LaunchedChild,
}

struct RecoveryContext<'a> {
    budget: &'a StdMutex<RestartBudget>,
    running: &'a mut Option<RunningServer>,
}

impl RunningServer {
    fn resolution(&self) -> Result<X11Resolution, X11ComponentError> {
        let spec = &self.launched.spec;
        Ok(X11Resolution {
            proxy: Some(X11Proxy::for_endpoint(spec.endpoint(), spec.cookie()?)),
            status: X11ForwardingStatus::Embedded {
                version: spec.version.clone(),
            },
        })
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        let _ = self.launched.child.start_kill();
        let _ = kill_owned(&self.launched.pid);
        let _ = remove_pidfile_if_owned(&self.launched.spec.root, &self.launched.pid);
    }
}

pub struct EmbeddedSupervisor {
    running: Mutex<Option<RunningServer>>,
    restart_budget: StdMutex<RestartBudget>,
    monitor_started: AtomicBool,
}

impl EmbeddedSupervisor {
    fn new() -> Self {
        Self {
            running: Mutex::new(None),
            restart_budget: StdMutex::new(RestartBudget::default()),
            monitor_started: AtomicBool::new(false),
        }
    }

    async fn ensure(&self, spec: LaunchSpec) -> Result<X11Resolution, X11ComponentError> {
        let mut running = self.running.lock().await;
        let crash = match running.as_mut() {
            Some(active) => match active.launched.child.try_wait() {
                Ok(None) => return active.resolution(),
                Ok(Some(status)) => Some(format!("yserver 已退出：{status}")),
                Err(error) => Some(format!("无法读取 yserver 状态：{error}")),
            },
            None => None,
        };
        if crash.is_some() {
            *running = None;
        }
        let context = RecoveryContext {
            budget: &self.restart_budget,
            running: &mut running,
        };
        let resolution = recover(context, spec, crash.clone()).await?;
        if crash.is_some() {
            note_restart(format!("{}；恢复结果：成功", crash.unwrap_or_default()));
        }
        Ok(resolution)
    }

    fn ensure_monitor(self: &Arc<Self>) {
        if self.monitor_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move { monitor_loop(weak).await });
    }

    async fn maintain(&self) {
        let mut running = self.running.lock().await;
        let Some(active) = running.as_mut() else {
            return;
        };
        let crash = match active.launched.child.try_wait() {
            Ok(None) => return,
            Ok(Some(status)) => format!("yserver 已退出：{status}"),
            Err(error) => format!("无法读取 yserver 状态：{error}"),
        };
        let spec = active.launched.spec.clone();
        *running = None;
        let context = RecoveryContext {
            budget: &self.restart_budget,
            running: &mut running,
        };
        let recovered = recover(context, spec, Some(crash.clone())).await;
        if recovered.is_ok() {
            note_restart(format!("{crash}；恢复结果：成功"));
        } else {
            record(MetricEvent::EmbeddedFailure);
        }
    }
}

pub async fn start(spec: LaunchSpec) -> Result<X11Resolution, X11ComponentError> {
    preflight(&spec.min_vulkan)?;
    let supervisor = global_supervisor();
    let resolution = supervisor.ensure(spec).await;
    if resolution.is_ok() {
        supervisor.ensure_monitor();
    }
    resolution
}

fn global_supervisor() -> &'static Arc<EmbeddedSupervisor> {
    static GLOBAL: OnceLock<Arc<EmbeddedSupervisor>> = OnceLock::new();
    GLOBAL.get_or_init(|| Arc::new(EmbeddedSupervisor::new()))
}

async fn recover(
    context: RecoveryContext<'_>,
    spec: LaunchSpec,
    crash: Option<String>,
) -> Result<X11Resolution, X11ComponentError> {
    let RecoveryContext { budget, running } = context;
    if crash.is_some() {
        record(MetricEvent::Crash);
        if !restart_allowed(budget) {
            let reason = "yserver 在 10 分钟内崩溃超过 3 次，本次运行禁用".to_string();
            let reason = disable_embedded(reason);
            set_active_version(None);
            let error = X11ComponentError::EmbeddedDisabled(reason.clone());
            update_status(X11ForwardingStatus::Disabled {
                reason: error.to_string(),
            });
            if let Some(crash) = crash {
                note_restart(format!("{crash}；恢复结果：永久禁用；{reason}"));
            }
            return Err(error);
        }
    }
    let mut launch_spec = spec.clone();
    match launch_with_retry(&mut launch_spec, crash.as_deref()).await {
        Ok(launched) => {
            if crash.is_some() {
                record(MetricEvent::Restart);
            } else {
                record(MetricEvent::EmbeddedStart);
            }
            set_active_version(Some(spec.version.clone()));
            let active = RunningServer { launched };
            let resolution = active.resolution()?;
            update_status(resolution.status.clone());
            *running = Some(active);
            Ok(resolution)
        }
        Err(error) => {
            set_active_version(None);
            update_status(X11ForwardingStatus::Disabled {
                reason: error.to_string(),
            });
            if let Some(crash) = crash {
                note_restart(format!("{crash}；恢复结果：失败；{error}"));
            }
            Err(error)
        }
    }
}

fn restart_allowed(budget: &StdMutex<RestartBudget>) -> bool {
    budget
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .record_crash(std::time::Instant::now())
}

#[derive(Debug, Eq, PartialEq)]
enum RetryDecision {
    Rotate,
    Stop,
}

fn spawn_retry_decision(error: &X11ComponentError, allow_refresh: bool) -> RetryDecision {
    if allow_refresh && matches!(error, X11ComponentError::Process(_)) {
        RetryDecision::Rotate
    } else {
        RetryDecision::Stop
    }
}

fn retry_decision(
    _spec: &LaunchSpec,
    crash: Option<&str>,
    last_attempt: bool,
    probe_failed: bool,
) -> RetryDecision {
    if crash.is_none() && !last_attempt && probe_failed {
        RetryDecision::Rotate
    } else {
        RetryDecision::Stop
    }
}

async fn launch_with_retry(
    spec: &mut LaunchSpec,
    crash: Option<&str>,
) -> Result<LaunchedChild, X11ComponentError> {
    let mut last_error = None;
    for attempt in 0..2 {
        let lock = ComponentLock::acquire_async(spec.root.clone())
            .await
            .map_err(|error| X11ComponentError::Process(error.to_string()))?;
        spec.reload_identity_for_spawn()?;
        let launched = match spawn_once(spec).await {
            Ok(launched) => launched,
            Err(error) => {
                let rotate = spawn_retry_decision(&error, crash.is_none() && attempt == 0)
                    == RetryDecision::Rotate;
                if rotate {
                    rotate_endpoint_locked(spec, &lock)?;
                    drop(lock);
                    continue;
                }
                return Err(error);
            }
        };
        match probe_launched(spec, launched, crash, attempt == 1).await {
            Ok(launched) => return Ok(launched),
            Err((error, rotate)) => {
                last_error = Some(error);
                if rotate {
                    rotate_endpoint_locked(spec, &lock)?;
                }
                drop(lock);
            }
        }
    }
    Err(X11ComponentError::Handshake(format!(
        "启动后握手重试一次仍失败：{}",
        last_error.unwrap_or_else(|| "未知错误".into())
    )))
}

fn rotate_endpoint_locked(
    spec: &mut LaunchSpec,
    lock: &ComponentLock,
) -> Result<(), X11ComponentError> {
    spec.refresh_endpoint_after_conflict_locked(lock)
}

async fn probe_launched(
    spec: &LaunchSpec,
    mut launched: LaunchedChild,
    crash: Option<&str>,
    last_attempt: bool,
) -> Result<LaunchedChild, (String, bool)> {
    let cookie = spec.cookie().map_err(|error| (error.to_string(), false))?;
    match probe_endpoint(&spec.endpoint(), &cookie, Duration::from_secs(2)).await {
        Ok(()) => Ok(launched),
        Err(error) => {
            let rotate = retry_decision(spec, crash, last_attempt, true) == RetryDecision::Rotate;
            let _ = launched.child.kill().await;
            let _ = launched.child.wait().await;
            let _ = kill_owned(&launched.pid);
            let _ = remove_owned_files(&spec.root, &launched.pid);
            Err((error.to_string(), rotate))
        }
    }
}

#[cfg(test)]
mod tests;

async fn monitor_loop(supervisor: Weak<EmbeddedSupervisor>) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let Some(supervisor) = supervisor.upgrade() else {
            return;
        };
        supervisor.maintain().await;
    }
}
