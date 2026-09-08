mod blob;
mod components;
pub(crate) mod connection;
mod context;
pub mod dev;

pub use dev::{DevHostOps, GlobalDevHostOps, set_dev_host_ops};

/// main 层 dev 操作实现所需的 gpui-shell 类型 re-export。
/// feature-gated,与 shell-plugins 一致;非 shell-plugins 构建下不存在。
#[cfg(feature = "shell-plugins")]
pub mod gpui_shell_reexport {
    pub use gpui_shell::{HostError, HostObject, HostValue, with_current_app};
}
mod event;
mod grant;
mod job;
mod log;
mod monitor;
mod opener;
mod policy;
mod resource;
mod runtime;
pub(crate) mod session;
mod value;

pub(crate) use context::ShellConnectionContext;
pub(crate) use policy::LoadedShellView;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::{Arc, atomic::AtomicU64},
};

use anyhow::{Context as _, Result, anyhow};
use extension_host::CancellationToken;
use extension_plugin_adapter::ActivationHandle;
use gpui::{App, WeakEntity, Window};
use gpui_shell::ShellRuntime;
use one_core::tab_container::{GlobalTabContainer, TabItem};

use self::{
    components::component_registry,
    connection::{PreparedShellConnection, ShellConnectionLaunch},
    session::ShellMountSession,
};
use crate::{
    extension_connection_tab::ExtensionConnectionTab, shell_plugin_tab::ShellPluginTab,
    universal_plugins::UniversalPluginService,
};

static NEXT_SHELL_TAB_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct ShellPluginHost {
    service: UniversalPluginService,
    runtime: Rc<ShellRuntime>,
    tokio: tokio::runtime::Handle,
    tabs: Rc<RefCell<HashMap<String, Vec<TrackedPluginTab>>>>,
    retiring: Rc<RefCell<HashSet<String>>>,
}

#[derive(Clone)]
#[cfg_attr(test, allow(dead_code))]
enum TrackedPluginTab {
    Shell {
        tab: WeakEntity<ShellPluginTab>,
    },
    Headless {
        runtime_id: String,
        tab: WeakEntity<ExtensionConnectionTab>,
    },
}

impl gpui::Global for ShellPluginHost {}

pub(crate) struct PreparedShellView {
    pub(crate) contribution: extension_runtime::RegisteredShellViewContribution,
    pub(crate) activations: Vec<ActivationHandle>,
    pub(crate) connection: Option<PreparedShellConnection>,
}

pub struct ConnectionShellOpen {
    pub connection: one_core::storage::StoredConnection,
    pub contribution: extension_runtime::RegisteredResourceConnectionContribution,
    pub mode: one_core::tab_container::TabOpenMode,
}

impl ShellPluginHost {
    pub(crate) fn new(service: UniversalPluginService, cx: &App) -> Result<Self> {
        let components = component_registry()?;
        Ok(Self {
            service,
            runtime: ShellRuntime::new_isolated_with_components(components)
                .context("create gpui-shell runtime")?,
            tokio: one_core::gpui_tokio::Tokio::handle(cx),
            tabs: Rc::new(RefCell::new(HashMap::new())),
            retiring: Rc::new(RefCell::new(HashSet::new())),
        })
    }

    pub(crate) fn contribution(
        &self,
        extension_id: &str,
        view_id: &str,
    ) -> Option<extension_runtime::RegisteredShellViewContribution> {
        self.service.shell_view(extension_id, view_id)
    }

    pub fn resource_connection(
        &self,
        extension_id: &str,
        contribution_id: &str,
    ) -> Option<extension_runtime::RegisteredResourceConnectionContribution> {
        self.service
            .resource_connection(extension_id, contribution_id)
    }

    pub fn open_connection(
        &self,
        request: ConnectionShellOpen,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        let ConnectionShellOpen {
            connection,
            contribution,
            mode,
        } = request;
        let connection_id = connection
            .id
            .ok_or_else(|| anyhow!("extension connection must be saved before opening"))?;
        let shell_view_id = contribution
            .shell_view_id
            .as_deref()
            .ok_or_else(|| anyhow!("extension connection has no shell view"))?;
        let view = self
            .contribution(&contribution.extension_id, shell_view_id)
            .ok_or_else(|| anyhow!("extension shell view was not found"))?;
        // 与 DB 驱动一致的统一凭据解析:Auth 密码簿引用在内存中还原为明文,
        // 随 open 载荷送达 provider;解析副本不落盘。
        let connection =
            crate::universal_plugins::resolve_extension_connection_for_runtime(connection, cx)?;
        let launch = ShellConnectionLaunch::new(&connection, &contribution, &view)?;
        let host = self.clone();
        let extension_id = contribution.extension_id;
        let title = connection.name;
        let tab_id = format!("extension-connection:{connection_id}");
        let tab_container = cx.global::<GlobalTabContainer>().primary_pane();
        tab_container.update(cx, |tabs, cx| {
            tabs.activate_or_add_tab_lazy_with_mode(
                tab_id.clone(),
                mode,
                move |window, cx| {
                    let registry_host = host.clone();
                    let tab = ShellPluginTab::load(
                        crate::shell_plugin_tab::ShellPluginLoad {
                            host,
                            contribution: view,
                            connection: Some(launch),
                            title_override: Some(title),
                        },
                        window,
                        cx,
                    );
                    registry_host.register_tab(extension_id, tab.downgrade());
                    TabItem::new(tab_id, "extension-connection", tab)
                },
                window,
                cx,
            );
        });
        Ok(())
    }

    async fn prepare_with_service(
        service: UniversalPluginService,
        contribution: extension_runtime::RegisteredShellViewContribution,
        connection: Option<ShellConnectionLaunch>,
        cancel: CancellationToken,
    ) -> Result<PreparedShellView> {
        policy::validate_entry_path(&contribution)?;
        let mut activations = Vec::new();
        for runtime_id in contribution.backends.values() {
            let activation = tokio::select! {
                _ = cancel.cancelled() => {
                    release_activations(&service, activations).await;
                    return Err(anyhow!("extension activation cancelled"));
                }
                activation = service.activate_runtime(runtime_id) => activation,
            };
            match activation {
                Ok(handle) => activations.push(handle),
                Err(error) => {
                    release_activations(&service, activations).await;
                    return Err(anyhow!(error.to_string()));
                }
            }
        }
        let connection = match connection {
            Some(connection) => {
                match connection::open_connection_resource(&service, connection, &cancel).await {
                    Ok(connection) => Some(connection),
                    Err(error) => {
                        release_activations(&service, activations).await;
                        return Err(error);
                    }
                }
            }
            None => None,
        };
        Ok(PreparedShellView {
            contribution,
            activations,
            connection,
        })
    }

    pub(crate) fn start_prepare(
        &self,
        contribution: extension_runtime::RegisteredShellViewContribution,
        connection: Option<ShellConnectionLaunch>,
        cancel: CancellationToken,
    ) -> tokio::sync::oneshot::Receiver<Result<PreparedShellView>> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let service = self.service.clone();
        self.tokio.spawn(async move {
            let result =
                Self::prepare_with_service(service.clone(), contribution, connection, cancel).await;
            if let Err(result) = sender.send(result)
                && let Ok(prepared) = result
            {
                release_activations(&service, prepared.activations).await;
            }
        });
        receiver
    }

    pub(crate) fn release(&self, activations: Vec<ActivationHandle>) {
        if activations.is_empty() {
            return;
        }
        let service = self.service.clone();
        self.tokio.spawn(async move {
            release_activations(&service, activations).await;
        });
    }

    pub(crate) fn release_after_session(
        &self,
        session: Option<Arc<ShellMountSession>>,
        activations: Vec<ActivationHandle>,
    ) {
        let _ = self.release_after_session_task(session, activations);
    }

    pub(crate) fn release_after_session_task(
        &self,
        session: Option<Arc<ShellMountSession>>,
        activations: Vec<ActivationHandle>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        if session.is_none() && activations.is_empty() {
            return None;
        }
        let service = self.service.clone();
        Some(self.tokio.spawn(async move {
            if let Some(session) = session {
                session.close_all().await;
            }
            release_activations(&service, activations).await;
        }))
    }

    pub(crate) fn service(&self) -> UniversalPluginService {
        self.service.clone()
    }

    fn register_tab(&self, extension_id: String, tab: WeakEntity<ShellPluginTab>) {
        self.tabs
            .borrow_mut()
            .entry(extension_id)
            .or_default()
            .push(TrackedPluginTab::Shell { tab });
    }

    pub fn register_headless_tab(
        &self,
        extension_id: String,
        runtime_id: String,
        tab: WeakEntity<ExtensionConnectionTab>,
    ) {
        self.tabs
            .borrow_mut()
            .entry(extension_id)
            .or_default()
            .push(TrackedPluginTab::Headless { runtime_id, tab });
    }
}

async fn release_activations(service: &UniversalPluginService, activations: Vec<ActivationHandle>) {
    for activation in activations {
        let _ = service.deactivate_activation(&activation).await;
    }
}
