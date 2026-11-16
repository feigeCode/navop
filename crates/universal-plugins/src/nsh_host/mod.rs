//! nsh 扩展宿主：扩展页面以 nsh 程序（作者 `nsh build` / `nsh publish`
//! 的产物）运行，画在 navop 自己的 GPUI 窗口里。
//!
//! 这里取代 `shell_plugin_host`（gpui-shell）承担的 UI 平面职责，边界一致：
//! `extension_view::ShellAppOpener` 决定"打开哪个扩展的 app、初始 route 是
//! 什么"，本模块只负责把它变成可运行的 nsh `AppView` 并挂进 tab；资源 /
//! 作业 / 事件 / blob 仍由能力平面（provider runtime）处理，页面通过
//! `host.call` 过来。
//!
//! 迁移方案见 `docs/extension-resource-plugins/nsh-migration-plan.md`。

mod abi;
#[cfg(feature = "nsh-db")]
mod abi_db;
#[cfg(feature = "nsh-terminal")]
mod abi_terminal;
mod abi_ui;
mod bundle;
mod connection;
mod decision;
mod dev;
mod dispatch;
mod grant;
mod host_events;
mod native_ai;
mod native_api;
mod monitor;
mod native;
mod opener;
mod panel;
mod plugin_ai_panel;
mod session;
mod tab;
mod theme;

pub use dev::{DevOps, set_dev_ops};
pub use native::{NativeFactory, NativeViewContext, register_native_component};
pub use panel::NshPanel;
pub use tab::NshTab;
#[cfg(feature = "nsh-db")]
pub use abi_db::{set_db_state, set_tree_refresh_hook};
pub use native_api::install as install_native_api;
/// 由终端色板派生一份 nsh 主题（终端侧栏的扩展面板用）。
#[cfg(feature = "nsh-terminal")]
pub fn nsh_terminal_theme(
    colors: &terminal_view::TerminalColors,
    cx: &gpui::App,
) -> nsh_platform_gpui::Theme {
    use gpui_component::ActiveTheme as _;
    theme::nsh_theme_from_terminal(cx.theme(), colors)
}


/// 打开一条扩展连接的页面（provider 资源由页面自己开）。
pub struct ConnectionPageOpen {
    pub connection: one_core::storage::StoredConnection,
    pub contribution: extension_runtime::RegisteredResourceConnectionContribution,
    pub mode: one_core::tab_container::TabOpenMode,
}

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui::{App, Entity, Global, WeakEntity};

use crate::universal_plugins::UniversalPluginService;

/// 已打开 tab 的按扩展登记（弱引用：tab 关掉条目自己失效）。
type Tabs = Rc<RefCell<HashMap<String, Vec<WeakEntity<NshTab>>>>>;

/// 正在退役的扩展（卸载/停用中，不再接受新页面）。
type Retiring = Rc<RefCell<std::collections::HashSet<String>>>;

/// 需要跟随 provider 重启的实体：扩展页面。
#[derive(Clone)]
pub(crate) enum Tracked {
    Page(WeakEntity<NshTab>),
    /// 面板 surface（终端侧栏等宿主锚点里的 `NshPanel`）。
    Panel(WeakEntity<panel::NshPanel>),
}

pub(crate) type Trust = Rc<RefCell<nsh_platform_gpui::package::TrustStore>>;

/// nsh 扩展宿主。
///
/// 是 GPUI `Global`：`ShellAppOpener` 的实现需要它，关闭路径也要通过它
/// 拿回"这个扩展还开着哪些页面"。
#[derive(Clone)]
pub struct NshHost {
    service: UniversalPluginService,
    trust: Trust,
    tabs: Tabs,
    /// 正在退役的扩展 id 集合。
    retiring: Retiring,
    /// 监视事件的转发目标（扩展页面）。
    tracked: Rc<RefCell<HashMap<String, Vec<Tracked>>>>,
    /// 引擎线程 → 主线程的桥（`navop.dev` 这类要碰 GPUI 全局的操作走它）。
    dispatcher: dispatch::MainThread,
    /// provider 事件转发要一个 tokio 句柄（订阅流在 tokio 上收）。
    tokio: tokio::runtime::Handle,
}

impl Global for NshHost {}

impl NshHost {
    fn new(
        service: UniversalPluginService,
        dispatcher: dispatch::MainThread,
        tokio: tokio::runtime::Handle,
    ) -> Self {
        Self {
            service,
            trust: bundle::trust_store(),
            tabs: Rc::new(RefCell::new(HashMap::new())),
            retiring: Rc::new(RefCell::new(std::collections::HashSet::new())),
            tracked: Rc::new(RefCell::new(HashMap::new())),
            dispatcher,
            tokio,
        }
    }

    /// 主线程桥的句柄。
    pub(crate) fn retiring(&self) -> std::cell::Ref<'_, std::collections::HashSet<String>> {
        self.retiring.borrow()
    }

    pub(crate) fn retiring_mut(&self) -> std::cell::RefMut<'_, std::collections::HashSet<String>> {
        self.retiring.borrow_mut()
    }

    pub(crate) fn service(&self) -> &UniversalPluginService {
        &self.service
    }

    fn tokio(&self) -> tokio::runtime::Handle {
        self.tokio.clone()
    }

    pub(crate) fn dispatcher(&self) -> dispatch::MainThread {
        self.dispatcher.clone()
    }

    /// 连接贡献项：`home` 决定链用（与旧 shell 宿主同名，调用点不必改）。
    pub fn resource_connection(
        &self,
        extension_id: &str,
        contribution_id: &str,
    ) -> Option<extension_runtime::RegisteredResourceConnectionContribution> {
        self.service
            .resource_connection(extension_id, contribution_id)
    }

    /// 这个扩展的 nsh app（manifest 的 `contributes.shellApp`）。
    pub(crate) fn contribution(
        &self,
        extension_id: &str,
    ) -> Option<extension_runtime::RegisteredShellAppContribution> {
        self.service.shell_app(extension_id)
    }

    /// 信任库（根公钥、撤销表、已接受的 build）。
    pub(crate) fn trust(&self) -> &Trust {
        &self.trust
    }

    pub(crate) fn track(&self, extension_id: &str, tab: WeakEntity<NshTab>) {
        self.tabs
            .borrow_mut()
            .entry(extension_id.to_string())
            .or_default()
            .push(tab.clone());
        self.tracked
            .borrow_mut()
            .entry(extension_id.to_string())
            .or_default()
            .push(Tracked::Page(tab));
    }

    /// 面板实体只进 monitor 记账（provider 重启要通知它重装），不进 tab
    /// 记账（它不在 tab 容器里，关闭由宿主锚点负责）。
    pub(crate) fn track_panel(&self, extension_id: &str, panel: WeakEntity<panel::NshPanel>) {
        self.tracked
            .borrow_mut()
            .entry(extension_id.to_string())
            .or_default()
            .push(Tracked::Panel(panel));
    }

    /// 这个扩展当前开着的 tab（顺手清掉已关闭的条目）。
    pub(crate) fn open_tabs(&self, extension_id: &str) -> Vec<gpui::Entity<NshTab>> {
        let mut tabs = self.tabs.borrow_mut();
        let Some(entries) = tabs.get_mut(extension_id) else {
            return Vec::new();
        };
        entries.retain(|tab| tab.upgrade().is_some());
        entries.iter().filter_map(|tab| tab.upgrade()).collect()
    }

    pub(crate) fn forget(&self, extension_id: &str) {
        self.tabs.borrow_mut().remove(extension_id);
    }
}

/// 注册 nsh 宿主（占用 `GlobalShellAppOpener`）。
///
/// 调用点保证与 gpui-shell 宿主互斥：两个宿主都往 `GlobalShellAppOpener`
/// 里放实现，后注册的覆盖先注册的。
pub fn init(service: UniversalPluginService, cx: &mut App) {
    // `navop.ai` 组件：页面用 `nativeComponent("navop.ai")` 自己摆位置。
    native_ai::install(service.clone(), cx);
    let host = NshHost::new(
        service,
        dispatch::MainThread::new(cx),
        one_core::gpui_tokio::Tokio::handle(cx),
    );
    extension_view::register_shell_app_opener(Rc::new(host.clone()), cx);
    // provider 换进程后，已打开的页面要跟着重装或报错，否则会一直拿着
    // 作废的 generation 调用。
    host.start_monitor_bridge(cx);
    cx.set_global(host);
}

impl NshHost {
    /// 打开一个扩展面板：装载 `contributes.panels[]` 的一条为独立
    /// `AppView` 实例，返回给宿主锚点（终端侧栏）嵌入。
    ///
    /// 面板不进 tab 容器；同一 (extension, panel) 每次调用都新建实例，
    /// 去重由宿主锚点自己记（侧栏每个面板只开一份）。找不到贡献项时
    /// 返回 None（扩展可能已被停用）。
    pub fn open_panel(
        &self,
        extension_id: &str,
        panel_id: &str,
        cx: &mut App,
    ) -> Option<Entity<panel::NshPanel>> {
        self.open_panel_with_context(extension_id, panel_id, None, None, cx)
    }

    /// 打开扩展面板并带上宿主上下文（如数据库侧栏的当前连接）。
    ///
    /// 上下文与 DB 菜单那条路径同形：面板页面从 `host.params.context` 读到
    /// 它，用来定位“我挂在哪个连接的侧栏上”。同一面板重复打开时不会重放。
    pub fn open_panel_with_context(
        &self,
        extension_id: &str,
        panel_id: &str,
        launch_context: Option<serde_json::Value>,
        tabs: Option<gpui::WeakEntity<one_core::tab_container::TabContainer>>,
        cx: &mut App,
    ) -> Option<Entity<panel::NshPanel>> {
        if self.retiring.borrow().contains(extension_id) {
            return None;
        }
        let contribution = self.contribution(extension_id)?;
        let registered = extension_runtime::global_catalog(cx)
            .and_then(|catalog| catalog.panel(extension_id, panel_id).cloned());
        let registered = registered?;
        let entity = panel::NshPanel::open(
            contribution,
            registered,
            launch_context,
            tabs,
            None,
            self.service.clone(),
            self.trust.clone(),
            self.dispatcher(),
            cx,
        );
        self.track_panel(extension_id, entity.downgrade());
        Some(entity)
    }

    /// 打开扩展面板，并指定面板自带主题（终端侧栏面板跟终端配色）。
    #[allow(clippy::too_many_arguments)]
    pub fn open_panel_with_theme(
        &self,
        extension_id: &str,
        panel_id: &str,
        launch_context: Option<serde_json::Value>,
        tabs: Option<gpui::WeakEntity<one_core::tab_container::TabContainer>>,
        surface_theme: Option<nsh_platform_gpui::Theme>,
        cx: &mut App,
    ) -> Option<Entity<panel::NshPanel>> {
        if self.retiring.borrow().contains(extension_id) {
            return None;
        }
        let contribution = self.contribution(extension_id)?;
        let registered = extension_runtime::global_catalog(cx)
            .and_then(|catalog| catalog.panel(extension_id, panel_id).cloned());
        let registered = registered?;
        let entity = panel::NshPanel::open(
            contribution,
            registered,
            launch_context,
            tabs,
            surface_theme,
            self.service.clone(),
            self.trust.clone(),
            self.dispatcher(),
            cx,
        );
        Some(entity)
    }

    /// 打开连接型扩展页面：先按连接配置打开 provider 资源，再挂页面。
    ///
    /// tab id 用 `extension-connection:{id}`，同一连接重复打开复用同一个 tab
    /// ——连接的资源只该开一份。
    pub fn open_connection(
        &self,
        request: ConnectionPageOpen,
        window: &mut gpui::Window,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let ConnectionPageOpen {
            connection,
            contribution,
            mode,
        } = request;
        let connection_id = connection
            .id
            .ok_or_else(|| anyhow::anyhow!("扩展连接必须先保存再打开"))?;
        let app = self
            .contribution(&contribution.extension_id)
            .ok_or_else(|| anyhow::anyhow!("扩展连接声明的 shell app 不存在"))?;
        let route = contribution.route.clone();
        // 凭据在内存里还原成明文随 open 载荷送到 provider；解析副本不落盘。
        let connection =
            crate::universal_plugins::resolve_extension_connection_for_runtime(connection, cx)?;
        let launch = connection::NshConnectionLaunch::new(&connection, &contribution, &app)?;
        let host = self.clone();
        let extension_id = contribution.extension_id.clone();
        let title: gpui::SharedString = connection.name.clone().into();
        let tab_id = format!("extension-connection:{connection_id}");
        let tab_container = cx
            .global::<one_core::tab_container::GlobalTabContainer>()
            .primary_pane();
        // 激活新 tab 前，`TabContainer` 会同步对当前内容调用 `on_deactivate`，而
        // `TabContentView for Entity<T>` 是用 `Entity::update` 进入那个实体的。
        // 调用方往往正处在该实体的 update 里（首页连接卡片、连接侧栏、公共 MCP
        // 的 `connection.open` 都是 `home_page.update(...)`），同步挂 tab 会重入
        // 同一个实体触发 double lease panic（复合扩展连接因此闪退）。把挂载动作
        // 延后到本轮 effect flush，那时实体已离开调用栈。
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |tabs, cx| {
                tabs.activate_or_add_tab_lazy_with_mode(
                    tab_id.clone(),
                    mode,
                    move |_window, cx| {
                        let registry_host = host.clone();
                        let tab = NshTab::open_connection(
                            app.clone(),
                            route.clone(),
                            host.service.clone(),
                            host.trust().clone(),
                            host.dispatcher(),
                            launch,
                            title,
                            cx,
                        );
                        registry_host.track(&extension_id, tab.downgrade());
                        one_core::tab_container::TabItem::new(tab_id, "extension-connection", tab)
                    },
                    window,
                    cx,
                );
            });
        });
        Ok(())
    }
}

/// 界面语言变化后让已打开的页面重新取词条。
///
/// nsh 页面把 locale 当输入：`set_locale` 会触发重渲染，页面里 `t("…")`
/// 自然读到新语言；不需要像 gpui-shell 那样重跑脚本。
pub fn refresh_views(cx: &mut App) {
    let Some(host) = cx.try_global::<NshHost>().cloned() else {
        return;
    };
    let locale = rust_i18n::locale().to_string();
    let tabs = host
        .tabs
        .borrow()
        .values()
        .flatten()
        .filter_map(|tab| tab.upgrade())
        .collect::<Vec<_>>();
    for tab in tabs {
        let locale = locale.clone();
        tab.update(cx, |tab, cx| {
            tab.apply_locale(&locale, cx);
            cx.notify();
        });
    }
}

/// 打开一个扩展的 nsh app：同一 (extension, route) 复用同一个 tab。
///
/// 一个扩展只有一个 app，所以 tab id 由 `nsh:app:{extension_id}` 加 route
/// 组成；没有初始 route 的（工具箱卡片）就是裸 app id。连接型 tab 走
/// [`NshHost::open_connection`]，id 是 `extension-connection:{connection_id}`。
///
/// `launch_context`（可选）是宿主侧触发上下文（如 DB 树右键菜单），
/// 注入 `host.params.context`；仅在 tab 新建时生效，复用已有 tab 时丢弃
/// （同一 (extension, route) 视为同一页面场景）。
fn open_view(
    host: NshHost,
    contribution: extension_runtime::RegisteredShellAppContribution,
    route: Option<&str>,
    launch_context: Option<serde_json::Value>,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    use one_core::tab_container::GlobalTabContainer;

    let Some(tabs) = cx.try_global::<GlobalTabContainer>() else {
        tracing::warn!(
            extension_id = %contribution.extension_id,
            "no GlobalTabContainer yet; nsh app not opened"
        );
        return;
    };
    let tabs = tabs.primary_pane();
    open_view_in(host, contribution, route, launch_context, tabs, window, cx);
}

/// 打开扩展页面到指定 tab 容器（如数据库标签页自己的页签栏）。
fn open_view_in(
    host: NshHost,
    contribution: extension_runtime::RegisteredShellAppContribution,
    route: Option<&str>,
    launch_context: Option<serde_json::Value>,
    tabs: gpui::Entity<one_core::tab_container::TabContainer>,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    use one_core::tab_container::TabItem;

    let tab_id = match route {
        Some(route) => format!("nsh:app:{}#{route}", contribution.extension_id),
        None => format!("nsh:app:{}", contribution.extension_id),
    };
    let extension_id = contribution.extension_id.clone();
    let route = route.map(str::to_string);
    // 页面继承自己所在的页签容器：从这里 `App.open` 打开的页面也留在同一个
    // 数据库标签页里，而不是跳到主窗口的页签栏。
    let launch_tabs = tabs.downgrade();
    tabs.update(cx, |tabs, cx| {
        let host_for_tab = host.clone();
        tabs.activate_or_add_tab_lazy(
            tab_id.clone(),
            move |_window, cx| {
                let tab = match launch_context {
                    Some(context) => NshTab::open_with_context_in(
                        contribution,
                        route,
                        context,
                        launch_tabs,
                        host_for_tab.service.clone(),
                        host_for_tab.trust().clone(),
                        host_for_tab.dispatcher(),
                        cx,
                    ),
                    None => NshTab::open_in(
                        contribution,
                        route,
                        launch_tabs,
                        host_for_tab.service.clone(),
                        host_for_tab.trust().clone(),
                        host_for_tab.dispatcher(),
                        cx,
                    ),
                };
                host_for_tab.track(&extension_id, tab.downgrade());
                TabItem::new(tab_id, format!("nsh:{extension_id}"), tab)
            },
            window,
            cx,
        )
    });
}

/// M3 验收台子：扩展目录里登记的 shell view 能被装载成 nsh 页面并挂进 tab。
///
/// 证据仍然走页面自己回报：自检页挂载时 `host.call("spike.mounted", …)`，
/// 这里注册同名 handler 并断言数据到达 —— 覆盖了"从 contribution 找入口 →
/// 读源码 → 按 nsh.json 授予能力 → 引擎线程装载 → 页面执行 → 跨线程回报"。
///
/// ```bash
/// cargo test -p universal-plugins --features nsh-host -- nsh_host
/// ```
#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        ops::Deref as _,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use extension_runtime::RegisteredShellAppContribution;
    use extension_runtime::extension::manifest::ShellHostModule;
    use gpui::{Context, IntoElement, Render, TestAppContext, VisualTestContext, Window, div};

    use super::*;

    struct Empty;

    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    /// 测试用的扩展目录（`resources/nsh-host-test`）：`nsh.json` + `view.js`。
    fn fixture_contribution() -> RegisteredShellAppContribution {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/nsh-host-test");
        RegisteredShellAppContribution {
            extension_id: "dev.navop.nsh-host-test".into(),
            extension_version: "0.1.0".into(),
            title: "自检 app".into(),
            description: None,
            icon_path: None,
            entry_path: root.join("view.js"),
            extension_root: root,
            category: None,
            keywords: Vec::new(),
            backends: BTreeMap::new(),
            // 页面在启动时读宿主能力，所以台子要声明它用到的模块。v2 只认
            // resource/job/event/blob/log/runtime/dev 七种，自检页用前两个
            // （`navop.resource.*` 走 call，`navop.log` 走 post）。
            modules: [ShellHostModule::Resource, ShellHostModule::Log]
                .into_iter()
                .collect(),
            permissions: Vec::new(),
            shell_api_version: "2".into(),
            required_nsh_version: Some("0.3.0".into()),
        }
    }

    /// 测试用内存库（AI 侧栏回查连接走 GlobalStorageState）。
    fn test_sqlite() -> one_core::storage::connection::SqliteConnection {
        let conn =
            one_core::storage::connection::SqliteConnection::open_with_pool_size(":memory:", 1)
                .expect("sqlite should open");
        conn.with_connection(|db| {
            one_core::storage::migration::run_migrations(db)?;
            Ok(())
        })
        .expect("migrations should run");
        conn
    }

    #[gpui::test]
    fn a_registered_view_loads_and_reaches_the_host(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            // 能力平面会话要 tokio 句柄；台子里现起一个最小的。
            one_core::gpui_tokio::init(cx);
            // AI 侧栏装载时要从全局存储回查连接；台子里给一份空的。
            use one_core::storage::{
                ConnectionRepository, GlobalStorageState, StorageManager, WorkspaceRepository,
            };
            let storage = StorageManager::new_with_connection(test_sqlite());
            storage.register(ConnectionRepository::new(storage.connection()));
            storage.register(WorkspaceRepository::new(storage.connection()));
            cx.set_global(GlobalStorageState { storage });
            // AI 侧栏（DefaultAgentChatPanel）还需要 LLM provider 状态。
            one_core::llm::init(cx);
        });
        let service = cx
            .update(|cx| crate::universal_plugins::register_application_owner(cx))
            .expect("台子只注册一次 plugin service");
        // nsh 引擎有自己的线程；gpui 的测试调度器默认断言只有测试线程在跑。
        cx.executor().allow_parking();
        let window = cx.add_window(|_, _| Empty);
        let mut context = VisualTestContext::from_window(*window.deref(), cx);

        let mounted = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        // 台子不碰真实数据目录：开发形态不需要信任库。
        let trust: Trust = Rc::new(RefCell::new(nsh_platform_gpui::package::TrustStore::new()));
        let tab = context.update(|_window, cx| {
            let tab = NshTab::open(
                fixture_contribution(),
                Some("self-check".to_string()),
                service.clone(),
                trust,
                dispatch::MainThread::new(cx),
                cx,
            );
            let tab_ref = tab.read(cx);
            assert_eq!(tab_ref.extension_id(), "dev.navop.nsh-host-test");
            assert_eq!(
                tab_ref.route().map(|route| route.as_ref()),
                Some("self-check")
            );
            // 页面装载发生在第一次 render（provider 激活可能异步），而宿主
            // handler 必须早于页面 —— 通道在 `open` 就建好了，所以这里能挂。
            let mounted = mounted.clone();
            tab_ref.host_channel().handle("spike.mounted", move |call| {
                let data = call.data().clone();
                if let Ok(mut mounted) = mounted.lock() {
                    mounted.push(data);
                }
                call.resolve(serde_json::Value::Null);
            });
            tab
        });

        let draw = |context: &mut VisualTestContext| {
            context.draw(
                gpui::Point::default(),
                gpui::size(gpui::px(900.), gpui::px(700.)),
                {
                    let tab = tab.clone();
                    move |_, _| tab.clone().into_any_element()
                },
            )
        };

        // 引擎线程的装载不会被 run_until_parked 推动：真等。
        for _ in 0..100 {
            draw(&mut context);
            context.run_until_parked();
            if !mounted.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        draw(&mut context);
        context.run_until_parked();

        // 激活 → 装载都在 `render` 里落地，画到这里页面应该已经挂上了。
        let loaded = context.update(|_window, cx| {
            let tab = tab.read(cx);
            (tab.app_view().is_some(), tab.error().map(str::to_owned))
        });
        assert!(loaded.0, "首帧后页面应已装载，失败原因: {:?}", loaded.1);

        let mounted = mounted.lock().unwrap();
        assert_eq!(
            mounted.len(),
            1,
            "自检页应恰好回报一次 spike.mounted，实际 {} 次",
            mounted.len()
        );
        // 页面读的是 `host.params`：v2 的启动事实全部从这一份里来（spec §2），
        // 所以这些字段对得上，就说明 params 在首帧前已经写好。
        let payload = &mounted[0];
        assert_eq!(payload["apiVersion"], 2, "页面读到的 params: {payload}");
        assert_eq!(
            payload["extensionId"], "dev.navop.nsh-host-test",
            "页面读到的 params: {payload}"
        );
        assert_eq!(
            payload["route"], "self-check",
            "页面读到的 params: {payload}"
        );
        assert_eq!(
            payload["backends"],
            serde_json::json!([]),
            "没有 backend 的页面应读到空数组: {payload}"
        );
        assert_eq!(
            payload["hasConnection"], false,
            "非连接型页面的 params.connection 必须是 null: {payload}"
        );
        // 页面拿一个不存在的 backend 开资源，宿主要回结构化 code（spec §6）：
        // 这同时证明 call 真的走到了 provider 分发层。
        assert_eq!(
            payload["resourceErrorCode"], "BACKEND_NOT_FOUND",
            "结构化错误的 code 应带回来: {payload}"
        );
        // `locale()` 读的是 `AppView::set_locale` 写进 surface 的那份值。
        let expected = nsh_platform_gpui::Locale::parse(&rust_i18n::locale().to_string())
            .unwrap_or(nsh_platform_gpui::Locale::EN_US);
        assert_eq!(
            payload["locale"],
            expected.as_str(),
            "页面 locale 应等于宿主 set_locale 的值: {payload}"
        );
        drop(mounted);

        // ②③④：另一个页面只用手写 JS，验结构化错误、dev 门禁与 `navop.event` 推送。
        let probe_dir = tempfile::tempdir().expect("建临时扩展目录");
        std::fs::write(probe_dir.path().join("view.js"), PROBE_PAGE).expect("写探针页");
        // 能力面是 UI set（进程级）：测试要在建引擎之前装一次。
        crate::nsh_host::native_api::install();
        let probes = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let trust: Trust = Rc::new(RefCell::new(nsh_platform_gpui::package::TrustStore::new()));
        let tab2 = context.update(|_window, cx| {
            let tab = NshTab::open(
                probe_contribution(probe_dir.path().to_path_buf()),
                Some("probe".to_string()),
                service.clone(),
                trust,
                dispatch::MainThread::new(cx),
                cx,
            );
            let probes = probes.clone();
            tab.read(cx)
                .host_channel()
                .handle("spike.mounted", move |call| {
                    let data = call.data().clone();
                    if let Ok(mut probes) = probes.lock() {
                        probes.push(data);
                    }
                    call.resolve(serde_json::Value::Null);
                });
            tab
        });

        let draw_probe = |context: &mut VisualTestContext| {
            let tab = tab2.clone();
            context.draw(
                gpui::Point::default(),
                gpui::size(gpui::px(900.), gpui::px(700.)),
                move |_, _| tab.clone().into_any_element(),
            )
        };

        // 等到页面把「启动期」的几项探针都报完（探针数量会随能力面增减，
        // 所以按 kind 等，不按条数等）。
        let wait_for = |context: &mut VisualTestContext, kinds: &[&str]| {
            for _ in 0..100 {
                draw_probe(context);
                context.run_until_parked();
                let reported = probes.lock().unwrap();
                if kinds.iter().all(|kind| {
                    reported
                        .iter()
                        .any(|payload| payload["kind"].as_str() == Some(kind))
                }) {
                    return;
                }
                drop(reported);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        wait_for(&mut context, &["params", "invoke", "dev", "native", "native-notify"]);

        // 宿主的推送：`send("navop.event", …)` 之后页面 `host.on` 要收到。
        context.update(|_window, cx| {
            tab2.read(cx)
                .host_channel()
                .send(
                    "navop.event",
                    serde_json::json!({
                        "handle": "evt-1",
                        "cursor": 1,
                        "events": [{ "seq": 1, "data": { "kind": "ping" } }],
                    }),
                )
                .expect("通道应能推送 navop.event");
        });
        wait_for(&mut context, &["event"]);

        let probes_arc = probes.clone();
        let probes = probes.lock().unwrap();
        let probe = |kind: &str| {
            probes
                .iter()
                .find(|payload| payload["kind"] == kind)
                .unwrap_or_else(|| panic!("探针页没有回报 {kind}: {probes:?}"))
        };
        let params = probe("params");
        assert_eq!(params["apiVersion"], 2, "探针页 params: {params}");
        assert_eq!(params["extensionId"], "dev.navop.nsh-host-test");
        assert_eq!(params["route"], "probe");
        assert_eq!(params["hasConnection"], false);
        // 不存在的句柄：INVALID_HANDLE，且不是可重试错误（spec §6）。
        let invoke = probe("invoke");
        assert_eq!(invoke["code"], "INVALID_HANDLE", "探针页错误: {invoke}");
        assert_eq!(invoke["retryable"], false, "探针页错误: {invoke}");
        // 非内置扩展调 dev.*：PERMISSION_DENIED，details 指出缺的模块。
        let dev = probe("dev");
        assert_eq!(dev["code"], "PERMISSION_DENIED", "探针页错误: {dev}");
        assert_eq!(dev["details"]["module"], "dev", "探针页错误: {dev}");
        let event = probe("event");
        assert_eq!(event["frame"]["handle"], "evt-1", "页面收到的推送: {event}");
        assert_eq!(event["frame"]["events"][0]["data"]["kind"], "ping");
        // `handle: "host"` 的订阅：不碰 provider，句柄形如 `host:{topic}:{n}`。
        let subscribe = probe("host-subscribe");
        let host_handle = subscribe["handle"]
            .as_str()
            .unwrap_or_else(|| panic!("宿主事件订阅失败: {subscribe}"))
            .to_string();
        assert!(
            host_handle.starts_with("host:probe.ping:"),
            "宿主事件句柄形状: {subscribe}"
        );
        assert_eq!(subscribe["cursor"], 0, "宿主事件没有回放: {subscribe}");
        // nativeModule：同一套 UI set 里的模块，页面按 nsh 标准解析出对象。
        let native = probe("native");
        let modules = native["modules"].as_str().unwrap_or_default();
        assert!(
            modules.split(',').any(|module| module == "App"),
            "nativeModule 缺 App: {native}"
        );
        assert!(
            modules.split(',').any(|module| module == "Ui"),
            "nativeModule 缺 Ui: {native}"
        );
        let notify = probe("native-notify");
        assert_eq!(notify["ok"], serde_json::json!(true), "nativeModule 调用: {notify}");
        drop(probe);
        drop(probes);

        // 宿主发布：帧形状与 provider 事件一致，页面按句柄过滤。
        super::host_events::publish(
            "probe.ping",
            serde_json::json!({ "kind": "host-ping", "n": 7 }),
        );
        let mut received = None;
        for _ in 0..100 {
            draw_probe(&mut context);
            context.run_until_parked();
            let found = probes_arc
                .lock()
                .unwrap()
                .iter()
                .find(|payload| {
                    payload["kind"] == "event" && payload["frame"]["handle"] == host_handle.as_str()
                })
                .cloned();
            if found.is_some() {
                received = found;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let received = received.unwrap_or_else(|| panic!("宿主事件没有被推送: {probes_arc:?}"));
        assert_eq!(received["frame"]["events"][0]["data"]["n"], 7, "{received}");
        assert_eq!(received["frame"]["events"][0]["data"]["kind"], "host-ping");
    }

    /// 手写的探针页：不依赖构建工具，直接跑在 nsh 引擎里。
    const PROBE_PAGE: &str = r#"import { host, mount, nativeModule, View } from "nsh";

let started = false;

function report(data) {
  host.call("spike.mounted", data, { timeout: 5000 }).catch(() => {});
}

function probe() {
  const params = host.params ?? {};
  report({
    kind: "params",
    apiVersion: params.apiVersion,
    extensionId: params.extension?.id,
    route: params.route,
    hasConnection: params.connection != null,
  });
  host
    .call("navop.resource.invoke", { handle: "missing-handle", method: "noop" }, { timeout: 5000 })
    .then(() => report({ kind: "invoke", code: null }))
    .catch((error) => report({ kind: "invoke", code: error?.code ?? String(error), retryable: error?.retryable }));
  host
    .call("navop.dev.list", {}, { timeout: 5000 })
    .then(() => report({ kind: "dev", code: null }))
    .catch((error) => report({ kind: "dev", code: error?.code ?? String(error), details: error?.details }));
  host
    .call("navop.event.subscribe", { handle: "host", kind: "probe.ping" }, { timeout: 5000 })
    .then((answer) => report({ kind: "host-subscribe", handle: answer?.handle ?? null, cursor: answer?.cursor }))
    .catch((error) =>
      report({
        kind: "host-subscribe",
        handle: null,
        code: error?.code ?? String(error),
        details: error?.details,
      }),
    );
  host.on("navop.event", (frame) => report({ kind: "event", frame: frame }));
  // nativeModule：nsh 标准的强类型宿主能力面（与 host.call 并行）。
  try {
    const api = nativeModule("navop");
    report({ kind: "native", modules: Object.keys(api).sort().join(",") });
    api.Ui
      .notify({ level: "info", title: "探针通知" })
      .then((answer) => report({ kind: "native-notify", ok: answer?.ok === true }))
      .catch((error) => report({ kind: "native-notify", ok: false, message: error?.message ?? String(error) }));
  } catch (error) {
    report({ kind: "native", code: error?.name ?? String(error), message: error?.message ?? String(error) });
  }
}

function App() {
  if (!started) {
    started = true;
    probe();
  }
  return View({ children: null });
}

mount(App);
"#;

    /// 探针页的扩展目录：声明 resource + dev，才能证明 dev 是"按内置身份"拒绝的。
    fn probe_contribution(root: PathBuf) -> RegisteredShellAppContribution {
        RegisteredShellAppContribution {
            extension_id: "dev.navop.nsh-host-test".into(),
            extension_version: "0.1.0".into(),
            title: "探针 app".into(),
            description: None,
            icon_path: None,
            entry_path: root.join("view.js"),
            extension_root: root,
            category: None,
            keywords: Vec::new(),
            backends: BTreeMap::new(),
            modules: [
                ShellHostModule::Resource,
                ShellHostModule::Event,
                ShellHostModule::Dev,
                // nativeModule 的鉴权走同一份清单：探针要调 App/Ui 就得声明。
                ShellHostModule::App,
                ShellHostModule::Ui,
            ]
            .into_iter()
            .collect(),
            permissions: Vec::new(),
            shell_api_version: "2".into(),
            required_nsh_version: Some("0.3.0".into()),
        }
    }

    /// 挂载连接 tab 必须延后到本轮 effect flush：调用方常常正处在当前 tab
    /// 内容实体的 update 里（首页连接卡片、连接侧栏、公共 MCP 的
    /// `connection.open` 都是 `home_page.update(...)`），而激活新 tab 会同步对
    /// 当前内容调用 `on_deactivate`，对同一实体重入 `update` 会 double lease
    /// panic（复合扩展连接曾因此闪退）。
    #[test]
    fn open_connection_defers_tab_activation_out_of_the_caller_update() {
        let source = include_str!("mod.rs");
        let open = source
            .split("pub fn open_connection(")
            .nth(1)
            .and_then(|source| source.split("\n    }\n}\n").next())
            .expect("NshHost::open_connection source");
        let normalized = open.split_whitespace().collect::<Vec<_>>().join(" ");

        let defer = normalized
            .find("window.defer(cx, move |window, cx| {")
            .expect("open_connection 必须用 window.defer 延后挂载 tab");
        let mount = normalized
            .find("tab_container.update(cx,")
            .expect("open_connection 仍需在 tab container 上挂载");

        assert!(
            defer < mount,
            "tab 挂载必须发生在 window.defer 内部：{normalized}"
        );
    }
}
