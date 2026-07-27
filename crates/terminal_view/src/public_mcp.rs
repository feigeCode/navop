use gpui::{App, Global};
use public_mcp::command_store::{CommandEntry, RemoteCommandStore};
use public_mcp::registry::{
    ConnectionState as McpConnectionState, PublicMcpRegistry, TerminalConnectionKind as McpKind,
    TerminalControlCancellation, TerminalControlFuture, TerminalControlSessionHandle,
    TerminalExecCancellation, TerminalExecFuture, TerminalExecSessionHandle,
    TerminalReadSessionHandle, TerminalSessionHandle, TerminalSessionSnapshot,
};
use public_mcp::remote_ops::RemoteCommandStatus;
use public_mcp::terminal_control::{
    TerminalControlAction, TerminalControlReadiness, TerminalControlRequest, TerminalControlResult,
};
use public_mcp::terminal_exec::{TerminalExecCompletion, TerminalExecRequest, TerminalExecResult};
use public_mcp::terminal_read::{TerminalReadRequest, TerminalReadResult};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use terminal::terminal::{ConnectionState, Terminal, TerminalConnectionKind};
use terminal::{
    TerminalControlAction as CoreTerminalControlAction, TerminalControlHandle,
    TerminalControlReadiness as CoreTerminalControlReadiness,
    TerminalControlRequest as CoreTerminalControlRequest,
    TerminalExecCompletion as CoreTerminalExecCompletion, TerminalExecHandle, TerminalExecObserver,
    TerminalExecProgress, TerminalExecRequest as CoreTerminalExecRequest, TerminalScrollProxy,
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

fn terminal_session_id(
    kind: TerminalConnectionKind,
    connection_id: Option<i64>,
    nonce: Uuid,
) -> Option<String> {
    match kind {
        TerminalConnectionKind::Local => Some(format!("local-terminal-{nonce}")),
        TerminalConnectionKind::Ssh => connection_id.map(|id| format!("ssh-terminal-{id}-{nonce}")),
        TerminalConnectionKind::Serial => None,
    }
}

fn agent_resource_from_session(
    session: public_mcp::registry::PublicMcpSessionInfo,
) -> agent_runtime::ResourceRef {
    let label = if session.host_label.is_empty() {
        session.title.clone()
    } else {
        session.host_label.clone()
    };
    let mut resource = agent_runtime::ResourceRef::new(
        session.session_id.clone(),
        agent_runtime::ResourceKind::Terminal,
        label,
    )
    .with_alias(session.session_id);
    if let Some(connection_id) = session.connection_id {
        resource = resource.with_alias(connection_id.to_string());
    }
    for alias in [session.title, session.host_label] {
        if !alias.is_empty() {
            resource = resource.with_alias(alias);
        }
    }
    for capability in session.capabilities {
        resource = resource.with_capability(capability);
    }
    resource
}

impl TerminalPublicMcpRegistration {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 将活动终端转换为 Agent 资源，供侧边栏建立当前默认目标。
    pub fn agent_resource(&self) -> Option<agent_runtime::ResourceRef> {
        self.registry
            .list_sessions()
            .into_iter()
            .find(|session| session.session_id == self.session_id)
            .map(agent_resource_from_session)
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
            registry.unregister_terminal_read(&self.session_id);
            registry.unregister_terminal_control(&self.session_id);
        }
    }
}

pub fn register_terminal(terminal: &Terminal, cx: &App) -> Option<TerminalPublicMcpRegistration> {
    let connection_kind = terminal.live_connection_kind()?;
    let connection_id = terminal.connection_id();
    let session_id = terminal_session_id(connection_kind, connection_id, Uuid::new_v4())?;
    let state = Arc::new(Mutex::new(snapshot_for_terminal(
        session_id.clone(),
        terminal,
    )));
    let target_registry = registry(cx)?;
    target_registry.register(ThreadSafeTerminalHandle {
        state: state.clone(),
    });
    target_registry.register_terminal_read(ThreadSafeTerminalReadHandle {
        state: state.clone(),
        scroll: terminal.scroll_proxy(),
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

struct ThreadSafeTerminalReadHandle {
    state: Arc<Mutex<TerminalSessionSnapshot>>,
    scroll: TerminalScrollProxy,
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

impl TerminalReadSessionHandle for ThreadSafeTerminalReadHandle {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        self.state
            .lock()
            .expect("public MCP state lock poisoned")
            .clone()
    }

    fn read_terminal(&self, request: TerminalReadRequest) -> anyhow::Result<TerminalReadResult> {
        let snapshot = self.scroll.recent_text(request.lines);
        Ok(TerminalReadResult {
            target: request.target,
            text: snapshot.text,
            requested_lines: snapshot.requested_lines,
            returned_lines: snapshot.returned_lines,
            available_lines: snapshot.available_lines,
            history_size: snapshot.history_size,
            screen_lines: snapshot.screen_lines,
            columns: snapshot.columns,
            truncated: false,
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
                truncated: core_result.truncated,
                captured_bytes: core_result.captured_bytes,
                discarded_bytes: core_result.discarded_bytes,
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
        .unwrap_or(match terminal.connection_kind() {
            TerminalConnectionKind::Local => "local terminal",
            TerminalConnectionKind::Ssh => "ssh terminal",
            TerminalConnectionKind::Serial => "serial terminal",
        })
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
    use gpui::{AppContext as _, TestAppContext};
    use one_core::settings::AppSettings;
    use public_mcp::registry::{TerminalControlCancellation, TerminalControlSessionHandle};
    use public_mcp::terminal_control::{
        TerminalControlAction, TerminalControlReadiness, TerminalControlRequest,
    };
    use std::sync::{Arc, Mutex};
    use terminal::recording::{
        ASCIICAST_VERSION, NAVOP_EVENT_STREAM, NAVOP_RECORDING_FORMAT_VERSION, ParsedRecording,
        RecordingBackend, RecordingCompleteness, RecordingHeader, RecordingHeaderMetadata,
        RecordingPlaybackLimits,
    };
    use terminal::{
        TerminalControlAction as CoreTerminalControlAction, TerminalControlHandle,
        TerminalControlOutput as CoreTerminalControlOutput,
        TerminalControlReadiness as CoreTerminalControlReadiness,
        TerminalControlRequest as CoreTerminalControlRequest,
    };
    use uuid::Uuid;

    #[test]
    fn local_terminal_registration_does_not_require_a_saved_connection_id() {
        let session_id = terminal_session_id(TerminalConnectionKind::Local, None, Uuid::nil())
            .expect("local terminal should be exposed to AI tools");

        assert_eq!(
            "local-terminal-00000000-0000-0000-0000-000000000000",
            session_id
        );
    }

    #[test]
    fn serial_terminal_is_not_registered_as_an_ai_terminal_session() {
        assert!(
            terminal_session_id(TerminalConnectionKind::Serial, Some(42), Uuid::nil()).is_none()
        );
    }

    fn recording_playback(backend: RecordingBackend) -> ParsedRecording {
        ParsedRecording {
            header: RecordingHeader {
                version: ASCIICAST_VERSION,
                width: 80,
                height: 24,
                timestamp: 1_700_000_000,
                navop: RecordingHeaderMetadata {
                    format_version: NAVOP_RECORDING_FORMAT_VERSION,
                    recording_id: "public-mcp-playback".to_string(),
                    session_id: "recorded-ssh-session".to_string(),
                    backend,
                    application_version: "0.1.0-test".to_string(),
                    started_at_unix_ms: 1_700_000_000_000,
                    capture_input: false,
                    event_stream: NAVOP_EVENT_STREAM.to_string(),
                },
            },
            events: Vec::new(),
            completeness: RecordingCompleteness::Complete,
        }
    }

    #[gpui::test]
    fn ssh_recording_playback_is_not_registered_with_public_mcp(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(AppSettings::default());
            one_core::gpui_tokio::init(cx);
            init(cx);
            let terminal = cx.new(|cx| {
                Terminal::new_recording_playback(
                    recording_playback(RecordingBackend::Ssh),
                    RecordingPlaybackLimits::default(),
                    cx,
                )
                .expect("create SSH recording playback")
            });

            assert_eq!(
                TerminalConnectionKind::Ssh,
                terminal.read(cx).connection_kind()
            );
            assert_eq!(None, terminal.read(cx).live_connection_kind());
            assert!(register_terminal(terminal.read(cx), cx).is_none());
            assert!(
                registry(cx)
                    .expect("public MCP registry")
                    .list_sessions()
                    .is_empty()
            );
        });
    }

    #[gpui::test]
    fn local_recording_playback_is_not_registered_with_public_mcp(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(AppSettings::default());
            one_core::gpui_tokio::init(cx);
            init(cx);
            let terminal = cx.new(|cx| {
                Terminal::new_recording_playback(
                    recording_playback(RecordingBackend::Local),
                    RecordingPlaybackLimits::default(),
                    cx,
                )
                .expect("create local recording playback")
            });

            assert_eq!(
                TerminalConnectionKind::Local,
                terminal.read(cx).connection_kind()
            );
            assert_eq!(None, terminal.read(cx).live_connection_kind());
            assert!(register_terminal(terminal.read(cx), cx).is_none());
            assert!(
                registry(cx)
                    .expect("public MCP registry")
                    .list_sessions()
                    .is_empty()
            );
        });
    }

    #[test]
    fn local_terminal_session_becomes_an_agent_terminal_resource() {
        let resource = agent_resource_from_session(public_mcp::registry::PublicMcpSessionInfo {
            session_id: "local-terminal-1".to_string(),
            connection_id: None,
            title: "zsh".to_string(),
            host_label: "local terminal".to_string(),
            cwd: Some("/tmp/project".to_string()),
            rows: 24,
            cols: 80,
            connection_kind: McpKind::Local,
            connected: true,
            capabilities: vec![
                agent_runtime::ResourceCapability::TerminalExec,
                agent_runtime::ResourceCapability::TerminalControl,
            ],
        });

        assert_eq!(agent_runtime::ResourceKind::Terminal, resource.kind);
        assert_eq!("local-terminal-1", resource.id.as_str());
        assert_eq!("local terminal", resource.label);
        assert!(
            resource
                .capabilities
                .contains(&agent_runtime::ResourceCapability::TerminalExec)
        );
    }

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
                    truncated: false,
                    captured_bytes: 0,
                    discarded_bytes: 0,
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
                    truncated: true,
                    captured_bytes: 1024 * 1024,
                    discarded_bytes: 17,
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
        assert!(result.truncated);
        assert_eq!(1024 * 1024, result.captured_bytes);
        assert_eq!(17, result.discarded_bytes);
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
                    truncated: false,
                    captured_bytes: 14,
                    discarded_bytes: 0,
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
                    truncated: false,
                    captured_bytes: 0,
                    discarded_bytes: 0,
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
