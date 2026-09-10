use extension_host::CancellationToken;
use extension_plugin_adapter::ActivationHandle;
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

enum State {
    Connecting,
    Connected {
        activation: ActivationHandle,
        /// 连接主会话唯一所有者;workbench 只持有 handle。
        #[allow(dead_code)]
        session: extension_plugin_adapter::ResourceSessionOwner,
        workbench: Entity<resource_view::NativeResourceWorkbench>,
    },
    Failed(String),
}

pub struct ExtensionConnectionTab {
    connection_lease: Option<ActiveConnectionLease>,
    title: SharedString,
    focus_handle: FocusHandle,
    service: UniversalPluginService,
    workbench: extension_runtime::RegisteredResourceWorkbenchContribution,
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
        let resolved = crate::universal_plugins::resolve_extension_connection_for_runtime(
            connection.clone(),
            cx,
        )
        .unwrap_or(connection);
        let launch = ExtensionResourceLaunch::new(&resolved, &contribution);
        let runtime_handle = one_core::gpui_tokio::Tokio::handle(cx);
        let view = cx.new(|cx| Self {
            connection_lease: Some(connection_lease),
            title,
            focus_handle: cx.focus_handle(),
            service: service.clone(),
            workbench,
            runtime_id: runtime_id.clone(),
            state: State::Connecting,
            closing: false,
            tokio: one_core::gpui_tokio::Tokio::handle(cx),
        });
        view.update(cx, |_, cx| {
            cx.spawn(async move |this, cx| {
                // 连接建立会创建 provider 进程与 local-socket listener,
                // 必须落在应用 Tokio runtime 上执行,不能跑在 GPUI foreground executor。
                let result = runtime_handle
                    .spawn(async move { connect_resource(service, runtime_id, launch).await })
                    .await
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
                    .and_then(|result| result);
                let _ = this.update(cx, |this, cx| {
                    this.apply_connect_result(result, cx);
                });
            })
            .detach();
        });
        register_with_shell_host(&view, &contribution, cx);
        view
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

    pub(crate) fn runtime_changed(&mut self, runtime_id: &str, cx: &mut Context<Self>) {
        if runtime_id != self.runtime_id {
            return;
        }
        self.close(cx).detach();
        self.state = State::Failed("Provider restarted. Close and reopen this connection.".into());
        cx.notify();
    }
}

async fn connect_resource(
    service: UniversalPluginService,
    runtime_id: String,
    launch: anyhow::Result<ExtensionResourceLaunch>,
) -> anyhow::Result<(ActivationHandle, OpenedExtensionResource)> {
    let launch = launch?;
    let activation = service.activate_runtime(&runtime_id).await?;
    match launch.open(&service, &CancellationToken::new()).await {
        Ok(resource) => Ok((activation, resource)),
        Err(error) => {
            let _ = service.deactivate_activation(&activation).await;
            Err(error)
        }
    }
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
