use gpui::{App, Global};
use public_mcp::command_store::{CommandEntry, RemoteCommandStore};
use public_mcp::registry::{
    ConnectionState as McpConnectionState, PublicMcpRegistry, TerminalConnectionKind as McpKind,
    TerminalControlCancellation, TerminalControlFuture, TerminalControlSessionHandle,
    TerminalExecCancellation, TerminalExecFuture, TerminalExecSessionHandle, TerminalSessionHandle,
    TerminalSessionSnapshot,
};
use public_mcp::remote_ops::RemoteCommandStatus;
use public_mcp::terminal_control::{
    TerminalControlAction, TerminalControlReadiness, TerminalControlRequest, TerminalControlResult,
};
use public_mcp::terminal_exec::{TerminalExecCompletion, TerminalExecRequest, TerminalExecResult};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use terminal::terminal::{ConnectionState, Terminal, TerminalConnectionKind};
use terminal::{
    TerminalControlAction as CoreTerminalControlAction, TerminalControlHandle,
    TerminalControlReadiness as CoreTerminalControlReadiness,
    TerminalControlRequest as CoreTerminalControlRequest,
    TerminalExecCompletion as CoreTerminalExecCompletion, TerminalExecHandle, TerminalExecObserver,
    TerminalExecProgress, TerminalExecRequest as CoreTerminalExecRequest,
};
use uuid::Uuid;

const DEFAULT_OUTPUT_TIMEOUT_MS: u64 = 60_000;

pub struct GlobalPublicMcpRegistry(pub PublicMcpRegistry);

impl Global for GlobalPublicMcpRegistry {}

pub fn init(cx: &mut App) {
    if cx.try_global::<GlobalPublicMcpRegistry>().is_none() {
        cx.set_global(GlobalPublicMcpRegistry(PublicMcpRegistry::default()));
    }
}

pub fn registry(cx: &App) -> Option<PublicMcpRegistry> {
    cx.try_global::<GlobalPublicMcpRegistry>()
        .map(|global| global.0.clone())
}

pub struct TerminalPublicMcpRegistration {
    session_id: String,
    state: Arc<Mutex<TerminalSessionSnapshot>>,
    registry: PublicMcpRegistry,
    exec: Arc<Mutex<Option<TerminalExecHandle>>>,
    control: Arc<Mutex<Option<TerminalControlHandle>>>,
    terminal_exec_registered: AtomicBool,
    terminal_control_registered: AtomicBool,
}

impl TerminalPublicMcpRegistration {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn refresh(&self, terminal: &Terminal) {
        self.refresh_parts(
            snapshot_for_terminal(self.session_id.clone(), terminal),
            terminal.external_exec_handle(),
            terminal.external_control_handle(),
        );
    }

    fn refresh_parts(
        &self,
        snapshot: TerminalSessionSnapshot,
        exec: Option<TerminalExecHandle>,
        control: Option<TerminalControlHandle>,
    ) {
        {
            let mut exec_slot = self.exec.lock().expect("public MCP exec lock poisoned");
            *exec_slot = exec;
        }
        {
            let mut control_slot = self
                .control
                .lock()
                .expect("public MCP control lock poisoned");
            *control_slot = control;
        }
        let mut state = self.state.lock().expect("public MCP state lock poisoned");
        *state = snapshot;
        drop(state);
        self.ensure_terminal_exec_registered();
        self.ensure_terminal_control_registered();
    }

    fn ensure_terminal_exec_registered(&self) {
        let has_exec = self
            .exec
            .lock()
            .expect("public MCP exec lock poisoned")
            .is_some();
        if !has_exec || self.terminal_exec_registered.swap(true, Ordering::AcqRel) {
            return;
        }
        self.registry
            .register_terminal_exec(ThreadSafeTerminalExecHandle {
                state: self.state.clone(),
                exec: self.exec.clone(),
                command_store: self.registry.command_store().clone(),
            });
    }

    fn ensure_terminal_control_registered(&self) {
        let has_control = self
            .control
            .lock()
            .expect("public MCP control lock poisoned")
            .is_some();
        if !has_control
            || self
                .terminal_control_registered
                .swap(true, Ordering::AcqRel)
        {
            return;
        }
        self.registry
            .register_terminal_control(ThreadSafeTerminalControlHandle {
                state: self.state.clone(),
                control: self.control.clone(),
            });
    }

    pub fn unregister(&self, cx: &App) {
        if let Some(registry) = registry(cx) {
            registry.unregister(&self.session_id);
            registry.unregister_remote_ops(&self.session_id);
            registry.unregister_terminal_exec(&self.session_id);
            registry.unregister_terminal_control(&self.session_id);
        }
    }
}

pub fn register_terminal(terminal: &Terminal, cx: &App) -> Option<TerminalPublicMcpRegistration> {
    if terminal.connection_kind() != TerminalConnectionKind::Ssh {
        return None;
    }

    let connection_id = terminal.connection_id()?;
    let session_id = format!("ssh-terminal-{connection_id}-{}", Uuid::new_v4());
    let state = Arc::new(Mutex::new(snapshot_for_terminal(
        session_id.clone(),
        terminal,
    )));
    let target_registry = registry(cx)?;
    target_registry.register(ThreadSafeTerminalHandle {
        state: state.clone(),
    });
    let exec = Arc::new(Mutex::new(terminal.external_exec_handle()));
    let control = Arc::new(Mutex::new(terminal.external_control_handle()));

    // 注册结构化远程操作桥。remote ops 与 terminal handle 共享同一份 state，一次 refresh 同步两者。
    if let Some(session_manager) = terminal.ssh_session_manager() {
        let command_store = target_registry.command_store().clone();
        let remote_ops = crate::public_mcp_remote_ops::SshRemoteOpsHandle::with_shared_state(
            session_manager.clone(),
            state.clone(),
            command_store,
        );
        target_registry.register_remote_ops(remote_ops);
    }

    let registration = TerminalPublicMcpRegistration {
        session_id,
        state,
        registry: target_registry,
        exec,
        control,
        terminal_exec_registered: AtomicBool::new(false),
        terminal_control_registered: AtomicBool::new(false),
    };
    registration.ensure_terminal_exec_registered();
    registration.ensure_terminal_control_registered();
    Some(registration)
}

struct ThreadSafeTerminalHandle {
    state: Arc<Mutex<TerminalSessionSnapshot>>,
}

impl TerminalSessionHandle for ThreadSafeTerminalHandle {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        self.state
            .lock()
            .expect("public MCP state lock poisoned")
            .clone()
    }
}

struct ThreadSafeTerminalExecHandle {
    state: Arc<Mutex<TerminalSessionSnapshot>>,
    exec: Arc<Mutex<Option<TerminalExecHandle>>>,
    command_store: RemoteCommandStore,
}

struct ThreadSafeTerminalControlHandle {
    state: Arc<Mutex<TerminalSessionSnapshot>>,
    control: Arc<Mutex<Option<TerminalControlHandle>>>,
}

impl TerminalControlSessionHandle for ThreadSafeTerminalControlHandle {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        self.state
            .lock()
            .expect("public MCP state lock poisoned")
            .clone()
    }

    fn control_terminal(
        &self,
        request: TerminalControlRequest,
        cancellation: TerminalControlCancellation,
    ) -> TerminalControlFuture {
        let control_handle = self
            .control
            .lock()
            .expect("public MCP control lock poisoned")
            .clone();
        Box::pin(async move {
            let control_handle = control_handle
                .ok_or_else(|| anyhow::anyhow!("terminal control handle is not ready"))?;
            let target = request.target;
            let output = control_handle
                .control(
                    CoreTerminalControlRequest {
                        action: map_control_action(request.action),
                    },
                    cancellation,
                )
                .await
                .map_err(|error| anyhow::anyhow!(error))?;
            Ok(TerminalControlResult {
                target,
                action: request.action,
                sent: output.sent,
                readiness_before: map_control_readiness(output.readiness_before),
            })
        })
    }
}

impl TerminalExecSessionHandle for ThreadSafeTerminalExecHandle {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        self.state
            .lock()
            .expect("public MCP state lock poisoned")
            .clone()
    }

    fn exec_in_terminal(
        &self,
        request: TerminalExecRequest,
        cancellation: TerminalExecCancellation,
    ) -> TerminalExecFuture {
        let exec_handle = self
            .exec
            .lock()
            .expect("public MCP exec lock poisoned")
            .clone();
        let command_store = self.command_store.clone();
        let session_id = self
            .state
            .lock()
            .expect("public MCP state lock poisoned")
            .session_id
            .clone();
        Box::pin(async move {
            let exec_handle =
                exec_handle.ok_or_else(|| anyhow::anyhow!("terminal exec handle is not ready"))?;
            let tracked = (request.submit && request.wait_for_output)
                .then(|| command_store.register_observed(&session_id, &request.command));
            let observer = tracked.as_ref().map(|(_, entry)| {
                let entry = entry.clone();
                TerminalExecObserver::new(move |progress| {
                    apply_terminal_progress(&entry, progress);
                })
            });
            let core_result = match exec_handle
                .exec(
                    CoreTerminalExecRequest {
                        command: request.command.clone(),
                        submit: request.submit,
                        wait_for_output: request.wait_for_output,
                        ready_timeout: Duration::from_millis(request.ready_timeout_ms),
                        timeout: Duration::from_millis(
                            request.timeout_ms.unwrap_or(DEFAULT_OUTPUT_TIMEOUT_MS),
                        ),
                        observer,
                    },
                    cancellation,
                )
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    if let Some((command_id, entry)) = &tracked {
                        entry.push_stderr(error.to_string().as_bytes());
                        entry.complete(RemoteCommandStatus::Failed, None);
                        command_store.remove(command_id);
                    }
                    return Err(anyhow::anyhow!(error));
                }
            };
            if let Some((_, entry)) = &tracked {
                entry.replace_stdout(core_result.output.as_bytes());
                if core_result.completion != CoreTerminalExecCompletion::TimedOut {
                    entry.complete(
                        completed_terminal_status(core_result.completion, core_result.exit_code),
                        core_result.exit_code,
                    );
                }
            }
            Ok(TerminalExecResult {
                target: request.target,
                command: request.command,
                submitted: request.submit,
                completion: map_exec_completion(core_result.completion),
                exit_code: core_result.exit_code,
                output: core_result.output,
                duration_ms: core_result.duration_ms,
                command_id: (core_result.completion == CoreTerminalExecCompletion::TimedOut)
                    .then(|| tracked.as_ref().map(|(id, _)| id.clone()))
                    .flatten(),
            })
        })
    }
}

fn apply_terminal_progress(entry: &CommandEntry, progress: TerminalExecProgress) {
    entry.replace_stdout(progress.output.as_bytes());
    if progress.is_final {
        entry.complete(
            completed_terminal_status(progress.completion, progress.exit_code),
            progress.exit_code,
        );
    }
}

fn completed_terminal_status(
    completion: CoreTerminalExecCompletion,
    exit_code: Option<i32>,
) -> RemoteCommandStatus {
    if completion == CoreTerminalExecCompletion::TimedOut {
        return RemoteCommandStatus::TimedOut;
    }
    if exit_code.is_none() || exit_code == Some(0) {
        RemoteCommandStatus::Exited
    } else {
        RemoteCommandStatus::Failed
    }
}

fn map_exec_completion(completion: CoreTerminalExecCompletion) -> TerminalExecCompletion {
    match completion {
        CoreTerminalExecCompletion::ObservedOutput => TerminalExecCompletion::ObservedOutput,
        CoreTerminalExecCompletion::ShellIntegrationExit => {
            TerminalExecCompletion::ShellIntegrationExit
        }
        CoreTerminalExecCompletion::SubmittedOnly => TerminalExecCompletion::SubmittedOnly,
        CoreTerminalExecCompletion::TimedOut => TerminalExecCompletion::TimedOut,
    }
}

fn map_control_action(action: TerminalControlAction) -> CoreTerminalControlAction {
    match action {
        TerminalControlAction::Interrupt => CoreTerminalControlAction::Interrupt,
    }
}

fn map_control_readiness(readiness: CoreTerminalControlReadiness) -> TerminalControlReadiness {
    match readiness {
        CoreTerminalControlReadiness::SubmissionPending => {
            TerminalControlReadiness::SubmissionPending
        }
        CoreTerminalControlReadiness::CommandRunning => TerminalControlReadiness::CommandRunning,
    }
}

fn snapshot_for_terminal(session_id: String, terminal: &Terminal) -> TerminalSessionSnapshot {
    TerminalSessionSnapshot {
        session_id,
        connection_id: terminal.connection_id(),
        title: terminal.title().to_string(),
        host_label: host_label(terminal),
        cwd: terminal.current_working_dir().map(str::to_string),
        rows: terminal.rows(),
        cols: terminal.cols(),
        connection_kind: map_kind(terminal.connection_kind()),
        connection_state: map_state(terminal.connection_state()),
    }
}

fn host_label(terminal: &Terminal) -> String {
    terminal
        .connection_name()
        .or_else(|| {
            terminal
                .ssh_config()
                .map(|config| config.ssh_config.host.as_str())
        })
        .unwrap_or("ssh terminal")
        .to_string()
}

fn map_kind(kind: TerminalConnectionKind) -> McpKind {
    match kind {
        TerminalConnectionKind::Local => McpKind::Local,
        TerminalConnectionKind::Ssh => McpKind::Ssh,
        TerminalConnectionKind::Serial => McpKind::Serial,
    }
}

fn map_state(state: &ConnectionState) -> McpConnectionState {
    match state {
        ConnectionState::Connected => McpConnectionState::Connected,
        ConnectionState::Connecting => McpConnectionState::Connecting,
        ConnectionState::Disconnected { error } => McpConnectionState::Disconnected {
            error: error.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use public_mcp::registry::{TerminalControlCancellation, TerminalControlSessionHandle};
    use public_mcp::terminal_control::{
        TerminalControlAction, TerminalControlReadiness, TerminalControlRequest,
    };
    use std::sync::{Arc, Mutex};
    use terminal::{
        TerminalControlAction as CoreTerminalControlAction, TerminalControlHandle,
        TerminalControlOutput as CoreTerminalControlOutput,
        TerminalControlReadiness as CoreTerminalControlReadiness,
        TerminalControlRequest as CoreTerminalControlRequest,
    };

    fn snapshot(state: McpConnectionState) -> TerminalSessionSnapshot {
        TerminalSessionSnapshot {
            session_id: "terminal-1".to_string(),
            connection_id: Some(42),
            title: "terminal".to_string(),
            host_label: "prod-a".to_string(),
            cwd: Some("/root".to_string()),
            rows: 24,
            cols: 120,
            connection_kind: McpKind::Ssh,
            connection_state: state,
        }
    }

    fn fake_exec_handle(
        requests: Arc<Mutex<Vec<CoreTerminalExecRequest>>>,
        output: terminal::TerminalExecOutput,
    ) -> TerminalExecHandle {
        TerminalExecHandle::new(move |request, _cancellation| {
            let requests = requests.clone();
            let output = output.clone();
            Box::pin(async move {
                requests.lock().expect("requests lock").push(request);
                Ok(output)
            })
        })
    }

    fn fake_control_handle(
        requests: Arc<Mutex<Vec<CoreTerminalControlRequest>>>,
    ) -> TerminalControlHandle {
        TerminalControlHandle::new(move |request, _cancellation| {
            let requests = requests.clone();
            Box::pin(async move {
                requests.lock().expect("requests lock").push(request);
                Ok(CoreTerminalControlOutput {
                    action: CoreTerminalControlAction::Interrupt,
                    sent: true,
                    readiness_before: CoreTerminalControlReadiness::CommandRunning,
                })
            })
        })
    }

    #[tokio::test]
    async fn terminal_control_handle_maps_interrupt_to_backend_control_handle() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handle = ThreadSafeTerminalControlHandle {
            state: Arc::new(Mutex::new(snapshot(McpConnectionState::Connected))),
            control: Arc::new(Mutex::new(Some(fake_control_handle(requests.clone())))),
        };

        let result = handle
            .control_terminal(
                TerminalControlRequest {
                    target: "terminal-1".to_string(),
                    action: TerminalControlAction::Interrupt,
                },
                TerminalControlCancellation::new(),
            )
            .await
            .expect("terminal control should call backend handle");

        let recorded = requests.lock().unwrap();
        assert_eq!(1, recorded.len());
        assert_eq!(CoreTerminalControlAction::Interrupt, recorded[0].action);
        assert!(result.sent);
        assert_eq!(
            TerminalControlReadiness::CommandRunning,
            result.readiness_before
        );
    }

    #[tokio::test]
    async fn terminal_exec_handle_maps_request_to_backend_exec_handle() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handle = ThreadSafeTerminalExecHandle {
            state: Arc::new(Mutex::new(snapshot(McpConnectionState::Connected))),
            exec: Arc::new(Mutex::new(Some(fake_exec_handle(
                requests.clone(),
                terminal::TerminalExecOutput {
                    completion: CoreTerminalExecCompletion::SubmittedOnly,
                    exit_code: None,
                    output: String::new(),
                    duration_ms: 0,
                },
            )))),
            command_store: RemoteCommandStore::default(),
        };

        let result = handle
            .exec_in_terminal(
                TerminalExecRequest {
                    target: "terminal-1".to_string(),
                    command: "df -h".to_string(),
                    submit: true,
                    wait_for_output: false,
                    ready_timeout_ms: 0,
                    timeout_ms: None,
                },
                TerminalExecCancellation::new(),
            )
            .await
            .expect("terminal exec should call backend exec handle");

        let recorded = requests.lock().unwrap();
        assert_eq!(1, recorded.len());
        assert_eq!("df -h", recorded[0].command);
        assert!(recorded[0].submit);
        assert!(!recorded[0].wait_for_output);
        assert_eq!(Duration::from_millis(60_000), recorded[0].timeout);
        assert_eq!(TerminalExecCompletion::SubmittedOnly, result.completion);
        assert_eq!(None, result.exit_code);
        assert!(result.output.is_empty());
    }

    #[tokio::test]
    async fn terminal_exec_handle_maps_backend_output_to_public_mcp_result() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handle = ThreadSafeTerminalExecHandle {
            state: Arc::new(Mutex::new(snapshot(McpConnectionState::Connected))),
            exec: Arc::new(Mutex::new(Some(fake_exec_handle(
                requests,
                terminal::TerminalExecOutput {
                    completion: CoreTerminalExecCompletion::ShellIntegrationExit,
                    exit_code: Some(0),
                    output: "ssh.service loaded active running".to_string(),
                    duration_ms: 42,
                },
            )))),
            command_store: RemoteCommandStore::default(),
        };

        let result = handle
            .exec_in_terminal(
                TerminalExecRequest {
                    target: "terminal-1".to_string(),
                    command: "systemctl list-units --type=service".to_string(),
                    submit: true,
                    wait_for_output: true,
                    ready_timeout_ms: 0,
                    timeout_ms: Some(200),
                },
                TerminalExecCancellation::new(),
            )
            .await
            .expect("terminal exec should return backend output");

        assert_eq!(
            TerminalExecCompletion::ShellIntegrationExit,
            result.completion
        );
        assert_eq!(Some(0), result.exit_code);
        assert_eq!("ssh.service loaded active running", result.output);
        assert_eq!(42, result.duration_ms);
    }

    #[tokio::test]
    async fn terminal_exec_timeout_returns_tracked_command_id_and_partial_output() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let command_store = RemoteCommandStore::default();
        let handle = ThreadSafeTerminalExecHandle {
            state: Arc::new(Mutex::new(snapshot(McpConnectionState::Connected))),
            exec: Arc::new(Mutex::new(Some(fake_exec_handle(
                requests,
                terminal::TerminalExecOutput {
                    completion: CoreTerminalExecCompletion::TimedOut,
                    exit_code: None,
                    output: "partial output".to_string(),
                    duration_ms: 60_000,
                },
            )))),
            command_store: command_store.clone(),
        };

        let result = handle
            .exec_in_terminal(
                TerminalExecRequest {
                    target: "terminal-1".to_string(),
                    command: "long-command".to_string(),
                    submit: true,
                    wait_for_output: true,
                    ready_timeout_ms: 0,
                    timeout_ms: Some(60_000),
                },
                TerminalExecCancellation::new(),
            )
            .await
            .expect("timed out terminal exec should detach");

        let command_id = result.command_id.expect("timeout should return command id");
        assert_eq!(
            RemoteCommandStatus::Running,
            command_store.poll_by_id(&command_id).unwrap().status
        );
        assert_eq!(
            "partial output",
            command_store
                .output(&public_mcp::remote_ops::RemoteCommandOutputRequest {
                    command_id,
                    stdout_offset: 0,
                    stderr_offset: 0,
                    limit_bytes: None,
                })
                .unwrap()
                .stdout
        );
    }

    #[tokio::test]
    async fn terminal_exec_handle_fails_when_exec_handle_is_missing() {
        let handle = ThreadSafeTerminalExecHandle {
            state: Arc::new(Mutex::new(snapshot(McpConnectionState::Connected))),
            exec: Arc::new(Mutex::new(None)),
            command_store: RemoteCommandStore::default(),
        };

        let error = handle
            .exec_in_terminal(
                TerminalExecRequest {
                    target: "terminal-1".to_string(),
                    command: "df -h".to_string(),
                    submit: true,
                    wait_for_output: true,
                    ready_timeout_ms: 0,
                    timeout_ms: Some(10),
                },
                TerminalExecCancellation::new(),
            )
            .await
            .expect_err("missing exec handle should fail");

        assert!(
            error
                .to_string()
                .contains("terminal exec handle is not ready")
        );
    }

    #[tokio::test]
    async fn refresh_registers_terminal_exec_when_exec_handle_appears_after_initial_registration() {
        let registry = PublicMcpRegistry::default();
        let state = Arc::new(Mutex::new(snapshot(McpConnectionState::Connecting)));
        registry.register(ThreadSafeTerminalHandle {
            state: state.clone(),
        });

        let registration = TerminalPublicMcpRegistration {
            session_id: "terminal-1".to_string(),
            state,
            registry: registry.clone(),
            exec: Arc::new(Mutex::new(None)),
            control: Arc::new(Mutex::new(None)),
            terminal_exec_registered: std::sync::atomic::AtomicBool::new(false),
            terminal_control_registered: std::sync::atomic::AtomicBool::new(false),
        };
        let requests = Arc::new(Mutex::new(Vec::new()));

        registration.refresh_parts(
            snapshot(McpConnectionState::Connected),
            Some(fake_exec_handle(
                requests.clone(),
                terminal::TerminalExecOutput {
                    completion: CoreTerminalExecCompletion::SubmittedOnly,
                    exit_code: None,
                    output: String::new(),
                    duration_ms: 0,
                },
            )),
            None,
        );

        let sessions = registry.list_sessions();
        assert_eq!(1, sessions.len());
        assert!(
            sessions[0]
                .capabilities
                .iter()
                .any(|capability| format!("{capability:?}") == "TerminalExec")
        );

        registry
            .terminal_exec(
                "terminal-1",
                TerminalExecRequest {
                    target: "terminal-1".to_string(),
                    command: "df -h".to_string(),
                    submit: true,
                    wait_for_output: true,
                    ready_timeout_ms: 0,
                    timeout_ms: None,
                },
                TerminalExecCancellation::new(),
            )
            .await
            .expect("terminal exec should use the exec handle registered during refresh");

        assert_eq!(1, requests.lock().unwrap().len());
    }
}
