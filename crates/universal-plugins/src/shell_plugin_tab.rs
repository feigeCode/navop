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
            connection_lease,
        });
        let activation_task =
            host.start_prepare(contribution, connection, preparation.cancel.clone());
        let cleanup_host = host.clone();
        let task_completion = Arc::clone(&preparation);
        view.update(cx, |_, cx| {
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
        });
        view
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
        let runtime_id = event.runtime_id();
        let service = self.host.service();
        // provider 已在宿主侧换成新进程(generation 推进),或已从宿主消失时,
        // 已挂载的 shell view 仍然绑在旧进程上,必须失效。瞬时抖动
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
        if !runtime_change_invalidates(event, &self.runtime_ids, generations_stale) {
            return;
        }
        let state = std::mem::replace(
            &mut self.state,
            ShellPluginTabState::Failed(
                "Provider restarted. Close and reopen this connection.".into(),
            ),
        );
        if let ShellPluginTabState::Ready(mut loaded) = state {
            loaded.unload(cx);
            let activations = std::mem::take(&mut self.activations);
            self.host
                .release_after_session(Some(loaded.session()), activations);
        }
        self.connection_lease.take();
        cx.notify();
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

/// shell view 的失效判定:runtime 真的消失,或宿主 generation 已推进
/// (provider 已被新进程替换)时才失效。
///
/// 抽成自由函数是为了让判定可被单元测试直接覆盖;真正的重连需要重建
/// `LoadedShellView`/`ShellMountSession`,风险显著高于 headless 连接,因此
/// 这里只做"失效",自动重连留作后续项。
pub(crate) fn runtime_change_invalidates(
    event: &extension_plugin_adapter::RuntimeMonitorEvent,
    runtime_ids: &[String],
    generations_stale: bool,
) -> bool {
    if !runtime_ids
        .iter()
        .any(|candidate| candidate == event.runtime_id())
    {
        return false;
    }
    match event {
        extension_plugin_adapter::RuntimeMonitorEvent::RuntimeRemoved { .. } => true,
        _ => generations_stale,
    }
}

mod render;

#[cfg(test)]
mod tests {
    use extension_plugin_adapter::{ActivationError, RuntimeHealth, RuntimeMonitorEvent};

    use super::runtime_change_invalidates;

    const BACKEND: &str = "com.example.elasticsearch::provider";

    fn backend_ids() -> Vec<String> {
        vec![BACKEND.to_owned()]
    }

    fn health_event(runtime_id: &str) -> RuntimeMonitorEvent {
        RuntimeMonitorEvent::HealthChanged {
            runtime_id: runtime_id.to_owned(),
            health: RuntimeHealth {
                state: extension_plugin_adapter::RuntimeActivationState::Active,
                session_closed: false,
                ping_error: None,
                restart_attempts: 0,
                restart_budget: 3,
                restart_backoff_remaining: None,
            },
        }
    }

    #[test]
    fn transient_health_event_keeps_the_mounted_view() {
        assert!(!runtime_change_invalidates(
            &health_event(BACKEND),
            &backend_ids(),
            false
        ));
    }

    #[test]
    fn advanced_generation_invalidates_the_mounted_view() {
        assert!(runtime_change_invalidates(
            &health_event(BACKEND),
            &backend_ids(),
            true
        ));
    }

    #[test]
    fn check_failure_follows_the_generation_rule() {
        let event = RuntimeMonitorEvent::CheckFailed {
            runtime_id: BACKEND.to_owned(),
            error: ActivationError::RuntimeNotReady {
                runtime_id: BACKEND.to_owned(),
            },
        };
        assert!(!runtime_change_invalidates(&event, &backend_ids(), false));
        assert!(runtime_change_invalidates(&event, &backend_ids(), true));
    }

    #[test]
    fn removed_runtime_always_invalidates() {
        let event = RuntimeMonitorEvent::RuntimeRemoved {
            runtime_id: BACKEND.to_owned(),
        };
        assert!(runtime_change_invalidates(&event, &backend_ids(), false));
    }

    #[test]
    fn unrelated_runtime_never_invalidates() {
        let event = RuntimeMonitorEvent::RuntimeRemoved {
            runtime_id: "com.example.other::provider".to_owned(),
        };
        assert!(!runtime_change_invalidates(&event, &backend_ids(), true));
    }
}
