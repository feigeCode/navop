use extension_host::CancellationToken;
use extension_plugin_adapter::{
    ActivationHandle, ResourceSessionOwner, RuntimeMonitorEvent,
};
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Task, Window, div,
};
use one_core::{
    storage::{ActiveConnectionLease, ActiveConnections, StoredConnection},
    tab_container::{TabContent, TabContentEvent},
};

use crate::extension_resource::{ExtensionResourceLaunch, OpenedExtensionResource};
use crate::universal_plugins::UniversalPluginService;

/// 自动重连时需要释放的旧连接:旧 activation lease 与旧 resource session。
type StaleConnection = (ActivationHandle, ResourceSessionOwner);

enum State {
    Connecting,
    Connected {
        activation: ActivationHandle,
        /// 连接主会话唯一所有者;workbench 只持有 handle。
        session: ResourceSessionOwner,
        workbench: Entity<resource_view::NativeResourceWorkbench>,
    },
    Failed(String),
}

/// `ExtensionConnectionTab::runtime_changed` 的纯决策结果。
///
/// 抽成自由函数是为了让"事件 → 动作"的映射可以被单元测试直接覆盖,
/// 不必在 GPUI 里构造真实 provider 进程与 session。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeChangeDecision {
    /// 与本 tab 无关,或只是瞬时抖动:保持现状。
    Ignore,
    /// provider 已无法自动恢复(被移除,或重启预算耗尽):置为 Failed。
    Fail,
    /// provider 已换成新进程(generation 推进):释放旧 session 并重连。
    Reconnect,
}

/// 推导 tab 对一次监视事件的响应动作。
///
/// 判定依据是 **provider 进程 generation** 而不是事件类型本身:
///
/// - `RuntimeRemoved` / 宿主已不再持有该 runtime:真没了,只能失败;
/// - 宿主 generation 已推进 = 真实重启已完成,旧 session 绑定的旧进程已消失,
///   必须重建(resource session 与 activation 都按 generation 判定归属);
/// - generation 未变且 session 已关闭、状态为 `Failed`/`CrashLoop` =
///   自动重启被禁用或重启预算耗尽,tab 无法自愈;
/// - 其余情况(`Degraded`,或仍在退避中的 `Restarting`)是瞬时抖动,忽略,
///   否则会在重启窗口内反复重建连接。
pub(crate) fn decide_runtime_change(
    event: &RuntimeMonitorEvent,
    runtime_id: &str,
    closing: bool,
    activation_generation: Option<u64>,
    current_generation: Option<u64>,
) -> RuntimeChangeDecision {
    use extension_plugin_adapter::{RuntimeActivationState, RuntimeMonitorEvent};

    if closing || event.runtime_id() != runtime_id {
        return RuntimeChangeDecision::Ignore;
    }
    if matches!(event, RuntimeMonitorEvent::RuntimeRemoved { .. }) {
        return RuntimeChangeDecision::Fail;
    }
    // 尚未建立连接(Connecting/Failed)时没有可失效的 lease。
    let Some(activation_generation) = activation_generation else {
        return RuntimeChangeDecision::Ignore;
    };
    // 宿主已不再持有该 runtime。
    let Some(current_generation) = current_generation else {
        return RuntimeChangeDecision::Fail;
    };
    if current_generation != activation_generation {
        return RuntimeChangeDecision::Reconnect;
    }
    match event {
        RuntimeMonitorEvent::HealthChanged { health, .. }
            if health.session_closed
                && matches!(
                    health.state,
                    RuntimeActivationState::Failed | RuntimeActivationState::CrashLoop
                ) =>
        {
            RuntimeChangeDecision::Fail
        }
        _ => RuntimeChangeDecision::Ignore,
    }
}

/// Docker 这类 headless 原生工作台的连接 tab。
///
/// provider 进程崩溃后由宿主 supervisor 重启并推进 generation;tab 检测到
/// generation 变化后自动重建 resource session,用户不需要手动关闭再打开。
pub struct ExtensionConnectionTab {
    connection_lease: Option<ActiveConnectionLease>,
    title: SharedString,
    focus_handle: FocusHandle,
    service: UniversalPluginService,
    workbench: extension_runtime::RegisteredResourceWorkbenchContribution,
    contribution: extension_runtime::RegisteredResourceConnectionContribution,
    /// 已解析凭据的连接。重连要复用同一份启动参数,因此随 tab 保存。
    connection: StoredConnection,
    runtime_id: String,
    state: State,
    closing: bool,
    tokio: tokio::runtime::Handle,
}

impl ExtensionConnectionTab {
    pub fn load(
        service: UniversalPluginService,
        connection: StoredConnection,
        contribution: extension_runtime::RegisteredResourceConnectionContribution,
        workbench: extension_runtime::RegisteredResourceWorkbenchContribution,
        cx: &mut App,
    ) -> Entity<Self> {
        let connection_id = connection.id.expect("saved extension connection");
        let connection_lease = cx
            .default_global::<ActiveConnections>()
            .lease(connection_id);
        let runtime_id = contribution.runtime_id.clone();
        let title = connection.name.clone().into();
        let connection = crate::universal_plugins::resolve_extension_connection_for_runtime(
            connection.clone(),
            cx,
        )
        .unwrap_or(connection);
        let view = cx.new(|cx| Self {
            connection_lease: Some(connection_lease),
            title,
            focus_handle: cx.focus_handle(),
            service,
            workbench,
            contribution: contribution.clone(),
            connection,
            runtime_id,
            state: State::Connecting,
            closing: false,
            tokio: one_core::gpui_tokio::Tokio::handle(cx),
        });
        view.update(cx, |this, cx| this.spawn_connect(None, cx));
        register_with_shell_host(&view, &contribution, cx);
        view
    }

    /// 在宿主 Tokio runtime 上建立连接,并把结果应用回 tab。
    ///
    /// `stale` 是自动重连时需要释放的旧 session/lease。释放动作与新的
    /// `activate_runtime` 排在同一个任务里顺序执行,避免两个异步任务竞争:
    /// 若"释放旧 lease"先于"激活新 lease"完成,激活计数会归零、宿主会拆掉
    /// provider 进程,于是刚重启好的进程立刻又被重建一次。
    fn spawn_connect(&mut self, stale: Option<StaleConnection>, cx: &mut Context<Self>) {
        let service = self.service.clone();
        let runtime_id = self.runtime_id.clone();
        let launch = ExtensionResourceLaunch::new(&self.connection, &self.contribution);
        let runtime_handle = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            // 连接建立会创建 provider 进程与 local-socket listener,
            // 必须落在应用 Tokio runtime 上执行,不能跑在 GPUI foreground executor。
            let result = runtime_handle
                .spawn(async move { connect_resource(service, runtime_id, launch, stale).await })
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))
                .and_then(|result| result);
            let _ = this.update(cx, |this, cx| {
                this.apply_connect_result(result, cx);
            });
        })
        .detach();
    }

    fn apply_connect_result(
        &mut self,
        result: anyhow::Result<(ActivationHandle, OpenedExtensionResource)>,
        cx: &mut Context<Self>,
    ) {
        self.state = match result {
            Ok((activation, resource)) if !self.closing => {
                let identity = extension_plugin_adapter::ResourceSessionIdentity {
                    extension_id: self.workbench.extension_id.clone(),
                    runtime_id: self.runtime_id.clone(),
                    runtime_generation: resource.generation(),
                    session_epoch: 0,
                };
                let session = resource.into_session(identity);
                let handle = session.handle();
                let workbench = cx.new(|cx| {
                    resource_view::NativeResourceWorkbench::new(self.workbench.clone(), handle, cx)
                });
                State::Connected {
                    activation,
                    session,
                    workbench,
                }
            }
            Ok((activation, mut resource)) => {
                let service = self.service.clone();
                one_core::gpui_tokio::Tokio::spawn_result(cx, async move {
                    resource.close().await;
                    let _ = service.deactivate_activation(&activation).await;
                    Ok(())
                })
                .detach();
                State::Failed("Connection closed".into())
            }
            Err(error) => State::Failed(error.to_string()),
        };
        cx.notify();
    }

    /// provider 无法自愈:释放连接并展示失败原因(与关闭一样是终态)。
    #[cfg_attr(test, allow(dead_code))]
    fn fail(&mut self, reason: &str, cx: &mut Context<Self>) {
        self.close(cx).detach();
        self.state = State::Failed(reason.to_string());
        cx.notify();
    }

    /// provider 已完成真实重启:释放旧 session/lease 后重建连接。
    #[cfg_attr(test, allow(dead_code))]
    fn reconnect(&mut self, cx: &mut Context<Self>) {
        let stale = match std::mem::replace(&mut self.state, State::Connecting) {
            State::Connected {
                activation, session, ..
            } => Some((activation, session)),
            other => {
                // 未处于已连接状态:没有可释放的旧连接,状态原样放回。
                self.state = other;
                return;
            }
        };
        self.spawn_connect(stale, cx);
        cx.notify();
    }

    /// 宿主监视事件入口。仅由 shell host 的 monitor bridge 调用,而该 bridge
    /// 在测试构建下被 `cfg(not(test))` 移除,所以测试视图里这三个方法看起来
    /// 未被使用。
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn runtime_changed(&mut self, event: &RuntimeMonitorEvent, cx: &mut Context<Self>) {
        let activation_generation = match &self.state {
            State::Connected { activation, .. } => Some(activation.runtime_generation),
            _ => None,
        };
        let current_generation = self.service.runtime_generation(&self.runtime_id).ok();
        match decide_runtime_change(
            event,
            &self.runtime_id,
            self.closing,
            activation_generation,
            current_generation,
        ) {
            RuntimeChangeDecision::Ignore => {}
            RuntimeChangeDecision::Fail => self.fail(
                "Provider is unavailable and cannot be restarted automatically. \
                 Close and reopen this connection.",
                cx,
            ),
            RuntimeChangeDecision::Reconnect => self.reconnect(cx),
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        self.closing = true;
        let state = std::mem::replace(&mut self.state, State::Failed("Connection closed".into()));
        self.connection_lease.take();
        let State::Connected {
            activation,
            session,
            workbench: _,
        } = state
        else {
            return Task::ready(true);
        };
        let service = self.service.clone();
        let task = one_core::gpui_tokio::Tokio::spawn_result(cx, async move {
            let _ = session.close().await;
            let _ = service.deactivate_activation(&activation).await;
            Ok(())
        });
        cx.spawn(async move |_, _| task.await.is_ok())
    }

    pub(crate) fn close_for_extension(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        self.close(cx)
    }
}

async fn connect_resource(
    service: UniversalPluginService,
    runtime_id: String,
    launch: anyhow::Result<ExtensionResourceLaunch>,
    stale: Option<StaleConnection>,
) -> anyhow::Result<(ActivationHandle, OpenedExtensionResource)> {
    let launch = match launch {
        Ok(launch) => launch,
        Err(error) => {
            release_stale(&service, stale).await;
            return Err(error);
        }
    };
    // 重连时先取当前 generation 的新 lease,再释放旧 lease:顺序固定,
    // 释放旧 lease 时激活计数不会归零,重启好的 provider 进程被直接复用。
    // 对已经过期的 handle,deactivate 本身是幂等 no-op。
    let activation = match service.activate_runtime(&runtime_id).await {
        Ok(activation) => activation,
        Err(error) => {
            release_stale(&service, stale).await;
            return Err(error.into());
        }
    };
    release_stale(&service, stale).await;
    match launch.open(&service, &CancellationToken::new()).await {
        Ok(resource) => Ok((activation, resource)),
        Err(error) => {
            let _ = service.deactivate_activation(&activation).await;
            Err(error)
        }
    }
}

/// 释放自动重连前的旧 session 与旧 activation lease(均为尽力而为)。
async fn release_stale(service: &UniversalPluginService, stale: Option<StaleConnection>) {
    let Some((activation, session)) = stale else {
        return;
    };
    let _ = session.close().await;
    let _ = service.deactivate_activation(&activation).await;
}

/// 当 Shell host global 存在(shell-plugins 构建)时,把 headless tab
/// 注册进宿主重启通知;无 Shell 构建下是 no-op。
fn register_with_shell_host(
    view: &Entity<ExtensionConnectionTab>,
    contribution: &extension_runtime::RegisteredResourceConnectionContribution,
    cx: &App,
) {
    #[cfg(feature = "shell-plugins")]
    if let Some(host) = cx.try_global::<crate::shell_plugin_host::ShellPluginHost>() {
        host.register_headless_tab(
            contribution.extension_id.clone(),
            contribution.runtime_id.clone(),
            view.downgrade(),
        );
    }
    #[cfg(not(feature = "shell-plugins"))]
    {
        let _ = (view, contribution, cx);
    }
}

impl Drop for ExtensionConnectionTab {
    fn drop(&mut self) {
        self.connection_lease.take();
        let state = std::mem::replace(&mut self.state, State::Failed("Connection dropped".into()));
        let State::Connected {
            activation,
            session,
            workbench: _,
        } = state
        else {
            return;
        };
        let service = self.service.clone();
        self.tokio.spawn(async move {
            let _ = session.close().await;
            let _ = service.deactivate_activation(&activation).await;
        });
    }
}

impl EventEmitter<TabContentEvent> for ExtensionConnectionTab {}

impl Focusable for ExtensionConnectionTab {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ExtensionConnectionTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p_4().child(match &self.state {
            State::Connecting => "Connecting extension...".to_string(),
            State::Connected { workbench, .. } => {
                return div()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .child(workbench.clone());
            }
            State::Failed(error) => format!("Extension connection failed: {error}"),
        })
    }
}

impl TabContent for ExtensionConnectionTab {
    fn content_key(&self) -> &'static str {
        "ExtensionConnection"
    }

    fn title(&self, _cx: &App) -> SharedString {
        self.title.clone()
    }

    fn can_rename(&self, _cx: &App) -> bool {
        false
    }

    fn try_close(&mut self, _id: &str, _window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        self.close(cx)
    }
}

#[cfg(test)]
mod tests {
    use extension_plugin_adapter::{RuntimeActivationState, RuntimeHealth};

    use super::*;

    const RUNTIME: &str = "com.example.docker::provider";

    fn health_event(state: RuntimeActivationState, session_closed: bool) -> RuntimeMonitorEvent {
        RuntimeMonitorEvent::HealthChanged {
            runtime_id: RUNTIME.to_owned(),
            health: RuntimeHealth {
                state,
                session_closed,
                ping_error: None,
                restart_attempts: 0,
                restart_budget: 3,
                restart_backoff_remaining: None,
            },
        }
    }

    fn removed_event() -> RuntimeMonitorEvent {
        RuntimeMonitorEvent::RuntimeRemoved {
            runtime_id: RUNTIME.to_owned(),
        }
    }

    fn check_failed_event() -> RuntimeMonitorEvent {
        RuntimeMonitorEvent::CheckFailed {
            runtime_id: RUNTIME.to_owned(),
            error: extension_plugin_adapter::ActivationError::RuntimeNotReady {
                runtime_id: RUNTIME.to_owned(),
            },
        }
    }

    fn decide(
        event: &RuntimeMonitorEvent,
        closing: bool,
        activation_generation: Option<u64>,
        current_generation: Option<u64>,
    ) -> RuntimeChangeDecision {
        decide_runtime_change(
            event,
            RUNTIME,
            closing,
            activation_generation,
            current_generation,
        )
    }

    #[test]
    fn transient_health_changes_keep_the_connection() {
        // Degraded:transport 仍开着,只是 ping 失败。
        let degraded = health_event(RuntimeActivationState::Degraded, false);
        assert_eq!(
            RuntimeChangeDecision::Ignore,
            decide(&degraded, false, Some(7), Some(7))
        );
        // Restarting:仍在重启退避窗口内,generation 尚未推进。
        let restarting = health_event(RuntimeActivationState::Restarting, true);
        assert_eq!(
            RuntimeChangeDecision::Ignore,
            decide(&restarting, false, Some(7), Some(7))
        );
        // 正常的 Active 心跳不改变任何状态。
        let active = health_event(RuntimeActivationState::Active, false);
        assert_eq!(
            RuntimeChangeDecision::Ignore,
            decide(&active, false, Some(7), Some(7))
        );
    }

    #[test]
    fn completed_restart_reconnects() {
        let active = health_event(RuntimeActivationState::Active, false);
        assert_eq!(
            RuntimeChangeDecision::Reconnect,
            decide(&active, false, Some(7), Some(8))
        );
    }

    #[test]
    fn check_failure_reconnects_only_after_generation_moved() {
        let failure = check_failed_event();
        assert_eq!(
            RuntimeChangeDecision::Ignore,
            decide(&failure, false, Some(7), Some(7))
        );
        assert_eq!(
            RuntimeChangeDecision::Reconnect,
            decide(&failure, false, Some(7), Some(8))
        );
    }

    #[test]
    fn exhausted_restart_budget_fails_the_tab() {
        for state in [
            RuntimeActivationState::CrashLoop,
            RuntimeActivationState::Failed,
        ] {
            let event = health_event(state, true);
            assert_eq!(
                RuntimeChangeDecision::Fail,
                decide(&event, false, Some(7), Some(7)),
                "{state:?} with a closed session and unchanged generation cannot self-heal"
            );
        }
    }

    #[test]
    fn removed_runtime_fails_the_tab() {
        assert_eq!(
            RuntimeChangeDecision::Fail,
            decide(&removed_event(), false, Some(7), None)
        );
    }

    #[test]
    fn runtime_missing_from_the_host_fails_the_tab() {
        let active = health_event(RuntimeActivationState::Active, false);
        assert_eq!(
            RuntimeChangeDecision::Fail,
            decide(&active, false, Some(7), None)
        );
    }

    #[test]
    fn closing_short_circuits_every_event() {
        assert_eq!(
            RuntimeChangeDecision::Ignore,
            decide(&removed_event(), true, Some(7), None)
        );
        let active = health_event(RuntimeActivationState::Active, false);
        assert_eq!(
            RuntimeChangeDecision::Ignore,
            decide(&active, true, Some(7), Some(8))
        );
    }

    #[test]
    fn unrelated_runtime_is_ignored() {
        let other = RuntimeMonitorEvent::RuntimeRemoved {
            runtime_id: "com.example.other::provider".to_owned(),
        };
        assert_eq!(
            RuntimeChangeDecision::Ignore,
            decide(&other, false, Some(7), None)
        );
    }

    #[test]
    fn tab_without_a_live_connection_ignores_health_but_not_removal() {
        let active = health_event(RuntimeActivationState::Active, false);
        assert_eq!(
            RuntimeChangeDecision::Ignore,
            decide(&active, false, None, Some(8))
        );
        assert_eq!(
            RuntimeChangeDecision::Fail,
            decide(&removed_event(), false, None, None)
        );
    }
}
