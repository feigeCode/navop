use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use extension_host::CancellationToken;
use extension_plugin_adapter::ActivationHandle;
use gpui::{App, AppContext, Context, Entity, FocusHandle, SharedString, Task, Window};
use one_core::tab_container::TabContentEvent;

use crate::shell_plugin_host::connection::ShellConnectionLaunch;
use crate::shell_plugin_host::{LoadedShellView, PreparedShellView, ShellPluginHost};

struct PreparationCompletion {
    cancel: CancellationToken,
    done: AtomicBool,
    late_activations: Mutex<Vec<ActivationHandle>>,
    release_tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    notify: tokio::sync::Notify,
}

impl PreparationCompletion {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            cancel: CancellationToken::new(),
            done: AtomicBool::new(false),
            late_activations: Mutex::new(Vec::new()),
            release_tasks: Mutex::new(Vec::new()),
            notify: tokio::sync::Notify::new(),
        })
    }

    fn finish(&self) {
        self.done.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.done.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    fn push_late(&self, activations: Vec<ActivationHandle>) {
        self.late_activations
            .lock()
            .expect("shell activation completion poisoned")
            .extend(activations);
    }

    fn take_late(&self) -> Vec<ActivationHandle> {
        std::mem::take(
            &mut *self
                .late_activations
                .lock()
                .expect("shell activation completion poisoned"),
        )
    }

    fn push_release_task(&self, task: tokio::task::JoinHandle<()>) {
        self.release_tasks
            .lock()
            .expect("shell activation completion poisoned")
            .push(task);
    }

    fn take_release_tasks(&self) -> Vec<tokio::task::JoinHandle<()>> {
        std::mem::take(
            &mut *self
                .release_tasks
                .lock()
                .expect("shell activation completion poisoned"),
        )
    }
}

enum ShellPluginTabState {
    Loading,
    Ready(LoadedShellView),
    Failed(String),
}

pub(crate) struct ShellPluginTab {
    title: SharedString,
    focus_handle: FocusHandle,
    host: ShellPluginHost,
    state: ShellPluginTabState,
    activations: Vec<ActivationHandle>,
    preparation: Arc<PreparationCompletion>,
    closing: bool,
    #[cfg(not(test))]
    runtime_ids: Vec<String>,
    /// 重挂需要重新走一遍 prepare→load,因此这两个输入必须留下来。
    /// 消费者(`remount_after_runtime_change`)只在非测试构建里存在——重挂
    /// 必须有真实窗口,测试构建拿不到。
    #[cfg_attr(test, allow(dead_code))]
    contribution: extension_runtime::RegisteredShellViewContribution,
    #[cfg_attr(test, allow(dead_code))]
    connection: Option<ShellConnectionLaunch>,
    /// provider 换代后待处理的重挂请求。
    ///
    /// 重挂需要真实 `Window`(挂载 gpui 视图),而 `runtime_changed` 只有
    /// entity context,所以先在这里落一个标记,等下一次 render 再落地。
    #[cfg(not(test))]
    pending_remount: bool,
    connection_lease: Option<one_core::storage::ActiveConnectionLease>,
}

pub(crate) struct ShellPluginLoad {
    pub(crate) host: ShellPluginHost,
    pub(crate) contribution: extension_runtime::RegisteredShellViewContribution,
    pub(crate) connection: Option<ShellConnectionLaunch>,
    pub(crate) title_override: Option<String>,
}

impl ShellPluginTab {
    pub(crate) fn load(
        request: ShellPluginLoad,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let ShellPluginLoad {
            host,
            contribution,
            connection,
            title_override,
        } = request;
        let title =
            SharedString::from(title_override.unwrap_or_else(|| contribution.title.clone()));
        #[cfg(not(test))]
        let runtime_ids = contribution.backends.values().cloned().collect();
        let connection_id = connection
            .as_ref()
            .map(ShellConnectionLaunch::connection_id);
        let connection_lease = connection_id.map(|connection_id| {
            cx.default_global::<one_core::storage::ActiveConnections>()
                .lease(connection_id)
        });
        let preparation = PreparationCompletion::new();
        let view = cx.new(|cx| Self {
            title,
            focus_handle: cx.focus_handle(),
            host: host.clone(),
            state: ShellPluginTabState::Loading,
            activations: Vec::new(),
            preparation: Arc::clone(&preparation),
            closing: false,
            #[cfg(not(test))]
            runtime_ids,
            // 重挂要复用同一份 contribution/launch,所以这里留一份副本。
            contribution: contribution.clone(),
            connection: connection.clone(),
            #[cfg(not(test))]
            pending_remount: false,
            connection_lease,
        });
        view.update(cx, |this, cx| {
            this.start_loading(contribution, connection, window, cx)
        });
        view
    }

    /// 启动一次 prepare→load。
    ///
    /// 首次加载与 provider 换代后的重挂走的是同一条路径:重挂不是特例,
    /// 只是又挂了一次。
    fn start_loading(
        &mut self,
        contribution: extension_runtime::RegisteredShellViewContribution,
        connection: Option<ShellConnectionLaunch>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preparation = PreparationCompletion::new();
        self.preparation = Arc::clone(&preparation);
        self.state = ShellPluginTabState::Loading;
        let activation_task =
            self.host
                .start_prepare(contribution, connection, preparation.cancel.clone());
        let cleanup_host = self.host.clone();
        let task_completion = Arc::clone(&preparation);
        cx.spawn_in(window, async move |this, cx| {
            let result = activation_task
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("extension activation cancelled")));
            match result {
                Ok(prepared) => {
                    let activations = prepared.activations.clone();
                    if this
                        .update_in(cx, |this, window, cx| {
                            if this.closing {
                                this.preparation.push_late(prepared.activations);
                            } else {
                                this.finish_load(prepared, window, cx);
                            }
                        })
                        .is_err()
                    {
                        cleanup_host.release(activations);
                    }
                }
                Err(error) => {
                    let _ = this.update_in(cx, |this, _, cx| {
                        if !this.closing {
                            this.state = ShellPluginTabState::Failed(error.to_string());
                            cx.notify();
                        }
                    });
                }
            }
            task_completion.finish();
        })
        .detach();
    }

    fn finish_load(
        &mut self,
        prepared: PreparedShellView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let activations = prepared.activations.clone();
        match self.host.load(prepared, window, cx) {
            Ok(loaded) => {
                self.activations = activations;
                self.state = ShellPluginTabState::Ready(loaded);
            }
            Err(error) => {
                if let Some(task) = self
                    .host
                    .release_after_session_task(Some(error.session), activations)
                {
                    self.preparation.push_release_task(task);
                }
                self.state = ShellPluginTabState::Failed(error.error.to_string());
            }
        }
        cx.notify();
    }

    fn close(
        &mut self,
        cx: &mut Context<Self>,
    ) -> (
        Vec<ActivationHandle>,
        Option<Arc<crate::shell_plugin_host::session::ShellMountSession>>,
    ) {
        self.closing = true;
        self.preparation.cancel.cancel();
        let state = std::mem::replace(
            &mut self.state,
            ShellPluginTabState::Failed("Extension closing".into()),
        );
        let session = match state {
            ShellPluginTabState::Ready(mut loaded) => {
                loaded.unload(cx);
                Some(loaded.session())
            }
            _ => None,
        };
        self.connection_lease.take();
        (std::mem::take(&mut self.activations), session)
    }

    pub(crate) fn close_for_extension(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        self.close_task(true, cx)
    }

    #[cfg(not(test))]
    pub(crate) fn runtime_changed(
        &mut self,
        event: &extension_plugin_adapter::RuntimeMonitorEvent,
        cx: &mut Context<Self>,
    ) {
        use crate::shell_plugin_tab::ShellRuntimeChange;

        let runtime_id = event.runtime_id();
        let service = self.host.service();
        // provider 已在宿主侧换成新进程(generation 推进),或已从宿主消失时,
        // 已挂载的 shell view 仍然绑在旧进程上,必须处理。瞬时抖动
        // (Degraded、仍在退避中的 Restarting)不打断视图。
        let generations_stale = self
            .activations
            .iter()
            .filter(|activation| activation.runtime_id == runtime_id)
            .any(|activation| {
                service
                    .runtime_generation(&activation.runtime_id)
                    .map(|generation| generation != activation.runtime_generation)
                    .unwrap_or(true)
            });
        let state = service
            .runtime_healths()
            .into_iter()
            .find(|(id, _)| id == runtime_id)
            .map(|(_, health)| health.state);
        match decide_shell_runtime_change(event, &self.runtime_ids, generations_stale, state) {
            // 新进程已经就绪:原地重挂。以前这里直接把 tab 置为 Failed,要求用户
            // "关掉再打开这个连接"—— provider 崩一次就等于页面永久报废,即使宿主
            // 已经自动重启好了它。
            ShellRuntimeChange::Remount => {
                self.pending_remount = true;
                cx.notify();
            }
            ShellRuntimeChange::Fail => {
                self.state = ShellPluginTabState::Failed(
                    "Provider restarted. Close and reopen this connection.".into(),
                );
                self.release_loaded_view(cx);
                self.connection_lease.take();
                cx.notify();
            }
            ShellRuntimeChange::Ignore => {}
        }
    }

    /// 释放当前已挂载的视图(session + activation),但保留 tab 本身。
    #[cfg_attr(test, allow(dead_code))]
    fn release_loaded_view(&mut self, cx: &mut Context<Self>) {
        let state = std::mem::replace(
            &mut self.state,
            ShellPluginTabState::Failed("Provider restarted".into()),
        );
        if let ShellPluginTabState::Ready(mut loaded) = state {
            loaded.unload(cx);
            let activations = std::mem::take(&mut self.activations);
            self.host
                .release_after_session(Some(loaded.session()), activations);
        }
    }

    /// provider 换代后的原地重挂:先放下旧 session,再用新的 generation 挂一次。
    ///
    /// 只有拿到真实 `Window` 时才能做(挂载 gpui 视图),所以由 render 在下一帧
    /// 调用,而不是在 `runtime_changed` 里直接做。
    ///
    /// 不变量:**同一时刻只有一次加载在飞**。已经在 `Loading` 时不再另起一次——
    /// 那次加载的结果会覆盖新视图的 activation 归属,而它的 generation 又无从
    /// 判断。此时把标记放回去:那次加载落地会 `notify`,下一帧再挂。
    #[cfg(not(test))]
    pub(crate) fn remount_after_runtime_change(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing {
            return;
        }
        if matches!(self.state, ShellPluginTabState::Loading) {
            self.pending_remount = true;
            return;
        }
        self.release_loaded_view(cx);
        // 连接本身没变(同一个连接 tab),lease 继续持有;只有 provider 进程换了。
        let contribution = self.contribution.clone();
        let connection = self.connection.clone();
        self.start_loading(contribution, connection, window, cx);
    }

    /// render 每帧检查一次待处理的重挂请求。
    ///
    /// 标记在 render 里消费,而不是在 `runtime_changed` 里直接执行:后者没有
    /// `Window`,而挂载 shell view 必须有。
    #[cfg(not(test))]
    pub(crate) fn take_pending_remount(&mut self) -> bool {
        std::mem::replace(&mut self.pending_remount, false)
    }

    fn close_task(&mut self, request_removal: bool, cx: &mut Context<Self>) -> Task<bool> {
        let (activations, session) = self.close(cx);
        let preparation = Arc::clone(&self.preparation);
        let service = self.host.service();
        let release = one_core::gpui_tokio::Tokio::spawn_result(cx, async move {
            preparation.wait().await;
            if let Some(session) = session {
                session.close_all().await;
            }
            for task in preparation.take_release_tasks() {
                let _ = task.await;
            }
            let mut activations = activations;
            activations.extend(preparation.take_late());
            for activation in activations {
                let _ = service.deactivate_activation(&activation).await;
            }
            Ok(())
        });
        cx.spawn(async move |this, cx| {
            let closed = release.await.is_ok();
            if closed && request_removal {
                let _ = this.update(cx, |_, cx| cx.emit(TabContentEvent::CloseRequested));
            }
            closed
        })
    }
}

/// shell view 在 runtime 事件上的处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellRuntimeChange {
    /// 与本次事件无关,或是仍在退避中的瞬时抖动:视图继续用着。
    Ignore,
    /// provider 已被新进程替换且新进程已就绪:原地重挂,复用同一个连接。
    Remount,
    /// provider 已从宿主消失,或重启已经不可能:只能失败。
    Fail,
}

/// shell view 的处置判定。
///
/// 抽成自由函数是为了让判定可被单元测试直接覆盖——重挂本身需要真实窗口,
/// 在测试构建里拿不到,但"什么时候该重挂"这个决定必须在测试里钉住。
///
/// `state` 是宿主侧当前记录的 runtime 状态:generation 推进只说明进程换了,
/// 新进程可能还在退避重启窗口里。此时重挂会立刻撞上"runtime 未就绪",所以
/// 等到 `Active`/`Degraded` 再挂。缺状态(as `None`)视作"不知道",不冒进。
pub(crate) fn decide_shell_runtime_change(
    event: &extension_plugin_adapter::RuntimeMonitorEvent,
    runtime_ids: &[String],
    generations_stale: bool,
    state: Option<extension_plugin_adapter::RuntimeActivationState>,
) -> ShellRuntimeChange {
    use extension_plugin_adapter::{RuntimeActivationState, RuntimeMonitorEvent};

    if !runtime_ids
        .iter()
        .any(|candidate| candidate == event.runtime_id())
    {
        return ShellRuntimeChange::Ignore;
    }
    if let RuntimeMonitorEvent::RuntimeRemoved { .. } = event {
        return ShellRuntimeChange::Fail;
    }
    if !generations_stale {
        return ShellRuntimeChange::Ignore;
    }
    match state {
        Some(RuntimeActivationState::Active) | Some(RuntimeActivationState::Degraded) => {
            ShellRuntimeChange::Remount
        }
        Some(RuntimeActivationState::Failed) | Some(RuntimeActivationState::CrashLoop) => {
            ShellRuntimeChange::Fail
        }
        // Starting / Restarting / 状态未知:重启还没落地,等下一次事件。
        _ => ShellRuntimeChange::Ignore,
    }
}

mod render;

#[cfg(test)]
mod tests {
    use extension_plugin_adapter::{
        ActivationError, RuntimeActivationState, RuntimeHealth, RuntimeMonitorEvent,
    };

    use super::{ShellRuntimeChange, decide_shell_runtime_change};

    const BACKEND: &str = "com.example.elasticsearch::provider";

    fn backend_ids() -> Vec<String> {
        vec![BACKEND.to_owned()]
    }

    fn health_event(runtime_id: &str) -> RuntimeMonitorEvent {
        RuntimeMonitorEvent::HealthChanged {
            runtime_id: runtime_id.to_owned(),
            health: RuntimeHealth {
                state: RuntimeActivationState::Active,
                session_closed: false,
                ping_error: None,
                restart_attempts: 0,
                restart_budget: 3,
                restart_backoff_remaining: None,
            },
        }
    }

    fn decide(
        event: &RuntimeMonitorEvent,
        generations_stale: bool,
        state: Option<RuntimeActivationState>,
    ) -> ShellRuntimeChange {
        decide_shell_runtime_change(event, &backend_ids(), generations_stale, state)
    }

    #[test]
    fn transient_health_event_keeps_the_mounted_view() {
        assert_eq!(
            decide(
                &health_event(BACKEND),
                false,
                Some(RuntimeActivationState::Active)
            ),
            ShellRuntimeChange::Ignore
        );
    }

    #[test]
    fn advanced_generation_with_ready_provider_remounts_in_place() {
        // 旧行为是 Fail(要求用户关掉再打开),这正是"列表清空后再也回不来"
        // 的宿主侧成因:宿主明明已经把 provider 重启好了。
        assert_eq!(
            decide(
                &health_event(BACKEND),
                true,
                Some(RuntimeActivationState::Active)
            ),
            ShellRuntimeChange::Remount
        );
    }

    #[test]
    fn advanced_generation_waits_until_the_new_provider_is_ready() {
        for state in [
            RuntimeActivationState::Starting,
            RuntimeActivationState::Restarting,
        ] {
            assert_eq!(
                decide(&health_event(BACKEND), true, Some(state)),
                ShellRuntimeChange::Ignore,
                "{state:?} 期间重挂会撞上 runtime 未就绪"
            );
        }
        // 状态未知时同样不冒进。
        assert_eq!(
            decide(&health_event(BACKEND), true, None),
            ShellRuntimeChange::Ignore
        );
    }

    #[test]
    fn advanced_generation_with_dead_provider_fails() {
        for state in [
            RuntimeActivationState::Failed,
            RuntimeActivationState::CrashLoop,
        ] {
            assert_eq!(
                decide(&health_event(BACKEND), true, Some(state)),
                ShellRuntimeChange::Fail,
                "{state:?} 没有可重挂的进程"
            );
        }
    }

    #[test]
    fn check_failure_follows_the_generation_rule() {
        let event = RuntimeMonitorEvent::CheckFailed {
            runtime_id: BACKEND.to_owned(),
            error: ActivationError::RuntimeNotReady {
                runtime_id: BACKEND.to_owned(),
            },
        };
        assert_eq!(
            decide(&event, false, Some(RuntimeActivationState::Active)),
            ShellRuntimeChange::Ignore
        );
        assert_eq!(
            decide(&event, true, Some(RuntimeActivationState::Active)),
            ShellRuntimeChange::Remount
        );
    }

    #[test]
    fn removed_runtime_always_fails() {
        let event = RuntimeMonitorEvent::RuntimeRemoved {
            runtime_id: BACKEND.to_owned(),
        };
        // 即使宿主还残留着一个 Active 状态,进程已经没了,重挂没有意义。
        assert_eq!(
            decide(&event, false, Some(RuntimeActivationState::Active)),
            ShellRuntimeChange::Fail
        );
    }

    #[test]
    fn unrelated_runtime_never_affects_the_view() {
        let event = RuntimeMonitorEvent::RuntimeRemoved {
            runtime_id: "com.example.other::provider".to_owned(),
        };
        assert_eq!(
            decide(&event, true, Some(RuntimeActivationState::Active)),
            ShellRuntimeChange::Ignore
        );
    }
}
