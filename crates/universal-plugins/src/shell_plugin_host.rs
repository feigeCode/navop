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
mod error;
mod event;
mod grant;
mod i18n;
mod job;
mod log;
mod monitor;
mod opener;
mod policy;
mod resource;
mod runtime;
pub(crate) mod session;
mod theme;
mod value;
mod workbench;

#[cfg(test)]
mod extension_pages_render_tests;

pub(crate) use context::ShellConnectionContext;
pub(crate) use policy::LoadedShellView;
pub(crate) use policy::load_borrowed;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::{Arc, atomic::AtomicU64},
};

use anyhow::{Context as _, Result, anyhow};
use extension_host::CancellationToken;
use extension_plugin_adapter::ActivationHandle;
use gpui::{App, Entity, WeakEntity, Window};
use gpui_shell::{ScriptView, ShellRuntime};
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
    /// 已挂载的脚本视图。tab 级 shell 视图与嵌入式工作台页面**两条装载路径
    /// 都经过** `policy::load_with_session_and_connection`，所以这里是两者的
    /// 唯一汇合点 —— 宿主需要"让所有页面重跑脚本"时只认这张表。
    ///
    /// 存弱引用：视图关掉后条目自己失效，`refresh_views` 顺手清掉，
    /// 因此不需要配套的注销路径（注销漏一次就是一处保活泄漏）。
    views: Rc<RefCell<Vec<WeakEntity<ScriptView>>>>,
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

impl ShellPluginHost {
    pub(crate) fn new_mount_session(
        &self,
        backends: std::collections::BTreeMap<String, String>,
    ) -> std::sync::Arc<session::ShellMountSession> {
        std::sync::Arc::new(session::ShellMountSession::new(
            self.service.clone(),
            backends,
            self.tokio.clone(),
        ))
    }
    pub fn resource_workbench_for_connection(
        &self,
        extension_id: &str,
        contribution_id: &str,
    ) -> Option<extension_runtime::RegisteredResourceWorkbenchContribution> {
        self.service
            .resource_workbench_for_connection(extension_id, contribution_id)
    }
}

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
            views: Rc::new(RefCell::new(Vec::new())),
        })
    }

    /// 登记一个已挂载的脚本视图。见 [`Self::views`]。
    pub(crate) fn track_view(&self, view: WeakEntity<ScriptView>) {
        self.views.borrow_mut().push(view);
    }

    /// 让每个还活着的脚本视图**重建页面描述**（不只是重画）。
    ///
    /// gpui-shell 把这两件事分得很清：`cx.notify()` 是"重画这一帧"，
    /// `ScriptView::refresh` 才是"脚本读的东西变了、描述作废"。
    /// 用错方向的表现是界面**不更新**（停在旧语言/旧数据），所以这里取后者。
    ///
    /// ⚠️ 重建走的是**已有脚本实例的 `render()`**（`refresh` → `rebuild` →
    /// `call_render(object, …)`），`init()` 不会再跑一遍。所以扩展里凡是
    /// "读一次就存起来"的状态（语言、主题）都必须在 `render()` 里重读，
    /// 否则这里叫醒了也没用。
    ///
    /// 先筛活再逐个 `update`：`update` 会跑脚本，不该在 `borrow_mut` 里做。
    fn refresh_views(&self, cx: &mut App) {
        let live: Vec<Entity<ScriptView>> = {
            let mut views = self.views.borrow_mut();
            views.retain(|view| view.upgrade().is_some());
            views.iter().filter_map(WeakEntity::upgrade).collect()
        };
        for view in live {
            view.update(cx, |view, cx| view.refresh(cx));
        }
    }

    pub fn contribution(
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

/// 宿主界面上下文变了（目前只有语言）：告诉脚本世界这件事。
///
/// 做两件事，缺一不可：
///
/// 1. `navop.i18n.revision()` 加一 —— 页面据此判断自己缓存的词条表要不要重建；
/// 2. 让每个已挂载的 shell view 重建页面描述 —— 只加版本号而不重建，页面会一直
///    停在旧语言上（`cx.notify()` 只重画，不重跑脚本）。
///
/// ⚠️ 第 2 步重跑的是**已有实例的 `render()`**，`init()` 不会再来一遍（细节见
/// [`ShellPluginHost::refresh_views`]）。所以"在 `init` 里选好词条表"这种写法
/// 在切语言后仍然显示旧语言 —— 这是本模块最容易被误判成"刷新生效了"的地方。
///
/// 调用点：navop 切界面语言的地方（`crates/navop_app/src/setting_tab.rs` 的语言下拉）。
///
/// ⚠️ **已知边界：只覆盖宿主装载的顶层视图。** 脚本里用 `cx.new(Class)` 造的
/// 嵌套视图各自带一份描述缓存，宿主拿不到它们（gpui-shell 没有"重跑全部"的
/// 入口）；当前扩展仓 30 多个页面**没有一个**用 `cx.new`，所以这条边界今天不
/// 构成问题。真需要时，正确的修法在 gpui-shell 侧 —— 让每个 ScriptView 自己
/// 盯一个语言水位（照 `theme_tokens` 的做法），而不是在宿主这里加表。
pub fn refresh_shell_views(cx: &mut App) {
    i18n::bump_revision();
    // 先克隆再借用 cx：`refresh_views` 要 `&mut App`，而 `try_global` 持有的是
    // `&App`。未装配 shell 宿主（无 shell-plugins 构建）时什么都没得刷新。
    let Some(host) = cx.try_global::<ShellPluginHost>().cloned() else {
        return;
    };
    host.refresh_views(cx);
}

async fn release_activations(service: &UniversalPluginService, activations: Vec<ActivationHandle>) {
    for activation in activations {
        let _ = service.deactivate_activation(&activation).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{dev, i18n, theme};
    use extension_runtime::extension::manifest::ShellHostModule;

    /// 声明面拼写 ↔ 脚本 import specifier（两侧名字对不对得上）由
    /// `extension-runtime` 的 `every_host_module_specifier_matches_its_manifest_spelling`
    /// 守着；这条只管另一半：模块**真的挂到了** `module_specifier()` 说的那个名字上
    /// — 名字对不对、挂没挂上是两件事，各自都能单独坏掉。
    ///
    /// 只取不需要会话/连接的模块：其余几个的工厂要么吃 `ShellMountSession`、
    /// 要么吃已注册的 contribution，在单元测试里造不出来。语言、主题、dev 三个
    /// 也正好是最容易被顺手改名的。
    #[test]
    fn session_free_modules_register_under_the_declared_names() {
        assert_eq!(
            i18n::i18n_module().name(),
            ShellHostModule::I18n.module_specifier()
        );
        assert_eq!(
            theme::theme_module().name(),
            ShellHostModule::Theme.module_specifier()
        );
        assert_eq!(
            dev::dev_module().name(),
            ShellHostModule::Dev.module_specifier()
        );
    }
}
