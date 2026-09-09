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

use crate::shell_plugin_host::connection::{ExtensionResourceLaunch, OpenedExtensionResource};
use crate::universal_plugins::UniversalPluginService;

enum State {
    Connecting,
    /// 变体载荷远大于其他变体,整体装箱以压缩枚举尺寸
    Connected(Box<ConnectedState>),
    Failed(String),
}

struct ConnectedState {
    activation: ActivationHandle,
    resource: OpenedExtensionResource,
}

pub(crate) struct ExtensionConnectionTab {
    connection_lease: Option<ActiveConnectionLease>,
    title: SharedString,
    focus_handle: FocusHandle,
    service: UniversalPluginService,
    #[cfg(not(test))]
    runtime_id: String,
    state: State,
    closing: bool,
    tokio: tokio::runtime::Handle,
}

impl ExtensionConnectionTab {
    pub(crate) fn load(
        service: UniversalPluginService,
        connection: StoredConnection,
        contribution: extension_runtime::RegisteredResourceConnectionContribution,
        cx: &mut App,
    ) -> Entity<Self> {
        let connection_id = connection.id.expect("saved extension connection");
        let connection_lease = cx
            .default_global::<ActiveConnections>()
            .lease(connection_id);
        let runtime_id = contribution.runtime_id.clone();
        let title = connection.name.clone().into();
        // SSH 隧道引用模式需要在打开时解析已保存的 SSH 连接,此处捕获连接仓库
        let repository = cx
            .global::<one_core::storage::GlobalStorageState>()
            .storage
            .get::<one_core::storage::ConnectionRepository>();
        let launch = ExtensionResourceLaunch::new(&connection, &contribution, repository);
        let view = cx.new(|cx| Self {
            connection_lease: Some(connection_lease),
            title,
            focus_handle: cx.focus_handle(),
            service: service.clone(),
            #[cfg(not(test))]
            runtime_id: runtime_id.clone(),
            state: State::Connecting,
            closing: false,
            tokio: one_core::gpui_tokio::Tokio::handle(cx),
        });
        view.update(cx, |_, cx| {
            cx.spawn(async move |this, cx| {
                let result = connect_resource(service, runtime_id, launch).await;
                let _ = this.update(cx, |this, cx| {
                    this.apply_connect_result(result, cx);
                });
            })
            .detach();
        });
        view
    }

    fn apply_connect_result(
        &mut self,
        result: anyhow::Result<(ActivationHandle, OpenedExtensionResource)>,
        cx: &mut Context<Self>,
    ) {
        self.state = match result {
            Ok((activation, resource)) if !self.closing => {
                State::Connected(Box::new(ConnectedState {
                    activation,
                    resource,
                }))
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
        let State::Connected(connected) = state else {
            return Task::ready(true);
        };
        let ConnectedState {
            activation,
            mut resource,
        } = *connected;
        let service = self.service.clone();
        let task = one_core::gpui_tokio::Tokio::spawn_result(cx, async move {
            resource.close().await;
            let _ = service.deactivate_activation(&activation).await;
            Ok(())
        });
        cx.spawn(async move |_, _| task.await.is_ok())
    }

    pub(crate) fn close_for_extension(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        self.close(cx)
    }

    #[cfg(not(test))]
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

impl Drop for ExtensionConnectionTab {
    fn drop(&mut self) {
        self.connection_lease.take();
        let state = std::mem::replace(&mut self.state, State::Failed("Connection dropped".into()));
        let State::Connected(connected) = state else {
            return;
        };
        let ConnectedState {
            activation,
            mut resource,
        } = *connected;
        let service = self.service.clone();
        self.tokio.spawn(async move {
            resource.close().await;
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
            State::Connected(connected) => {
                let resource = &connected.resource;
                format!(
                    "Connected\n\nCapabilities:\n{}",
                    resource.capabilities().join("\n")
                )
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
