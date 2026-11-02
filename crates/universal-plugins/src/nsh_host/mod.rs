//! nsh 扩展宿主：扩展页面以 nsh 程序（作者 `nsh build` / `nsh publish`
//! 的产物）运行，画在 navop 自己的 GPUI 窗口里。
//!
//! 这里取代 `shell_plugin_host`（gpui-shell）承担的 UI 平面职责，边界一致：
//! `extension_view::ShellViewOpener` 决定"打开哪个扩展的哪个 view"，本模块
//! 只负责把它变成可运行的 nsh `AppView` 并挂进 tab；资源 / 作业 / 事件 /
//! blob 仍由能力平面（provider runtime）处理，页面通过 `host.call` 过来。
//!
//! 迁移方案见 `docs/extension-resource-plugins/nsh-migration-plan.md`。

mod abi;
mod bundle;
mod connection;
mod dev;
mod dispatch;
mod grant;
mod monitor;
mod opener;
mod page_host;
mod session;
mod tab;
mod theme;

pub use dev::{DevOps, set_dev_ops};
pub use tab::NshTab;

/// 打开一条扩展连接的页面（provider 资源由页面自己开）。
pub struct ConnectionPageOpen {
    pub connection: one_core::storage::StoredConnection,
    pub contribution: extension_runtime::RegisteredResourceConnectionContribution,
    pub mode: one_core::tab_container::TabOpenMode,
}

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui::{App, Global, WeakEntity};

use crate::universal_plugins::UniversalPluginService;

/// 已打开 tab 的按扩展登记（弱引用：tab 关掉条目自己失效）。
type Tabs = Rc<RefCell<HashMap<String, Vec<WeakEntity<NshTab>>>>>;

/// 正在退役的扩展（卸载/停用中，不再接受新页面）。
type Retiring = Rc<RefCell<std::collections::HashSet<String>>>;

/// 需要跟随 provider 重启的实体：脚本页，以及 headless 连接 tab。
#[derive(Clone)]
pub(crate) enum Tracked {
    Page(WeakEntity<NshTab>),
    Headless {
        runtime_id: String,
        tab: WeakEntity<crate::ExtensionConnectionTab>,
    },
}

/// headless tab 的登记表（按扩展 id 分组）。
type HeadlessTabs =
    Rc<RefCell<HashMap<String, Vec<(String, WeakEntity<crate::ExtensionConnectionTab>)>>>>;

pub(crate) type Trust = Rc<RefCell<nsh_platform_gpui::package::TrustStore>>;

/// nsh 扩展宿主。
///
/// 是 GPUI `Global`：`ShellViewOpener` 的实现需要它，关闭路径也要通过它
/// 拿回"这个扩展还开着哪些页面"。
#[derive(Clone)]
pub struct NshHost {
    service: UniversalPluginService,
    trust: Trust,
    tabs: Tabs,
    headless: HeadlessTabs,
    /// 正在退役的扩展 id 集合。
    retiring: Retiring,
    /// 监视事件的转发目标（脚本页 + headless tab）。
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
            headless: Rc::new(RefCell::new(HashMap::new())),
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

    /// 这个扩展名下还开着的 headless 连接 tab。
    pub(crate) fn headless_tabs(
        &self,
        extension_id: &str,
    ) -> Vec<gpui::Entity<crate::ExtensionConnectionTab>> {
        let mut headless = self.headless.borrow_mut();
        let Some(entries) = headless.get_mut(extension_id) else {
            return Vec::new();
        };
        let mut open = Vec::new();
        entries.retain(|(_, tab)| match tab.upgrade() {
            Some(tab) => {
                open.push(tab);
                true
            }
            None => false,
        });
        open
    }

    fn tokio(&self) -> tokio::runtime::Handle {
        self.tokio.clone()
    }

    pub(crate) fn dispatcher(&self) -> dispatch::MainThread {
        self.dispatcher.clone()
    }

    /// 目录里登记的 shell view（manifest 的 `contributes.shellViews`）。
    /// 连接贡献项：`home` 决定链用（与旧 shell 宿主同名，调用点不必改）。
    pub fn resource_connection(
        &self,
        extension_id: &str,
        contribution_id: &str,
    ) -> Option<extension_runtime::RegisteredResourceConnectionContribution> {
        self.service
            .resource_connection(extension_id, contribution_id)
    }

    /// 这条连接绑定的原生工作台（有就走原生页，没有才开脚本页）。
    pub fn resource_workbench_for_connection(
        &self,
        extension_id: &str,
        contribution_id: &str,
    ) -> Option<extension_runtime::RegisteredResourceWorkbenchContribution> {
        self.service
            .resource_workbench_for_connection(extension_id, contribution_id)
    }

    pub(crate) fn contribution(
        &self,
        extension_id: &str,
        view_id: &str,
    ) -> Option<extension_runtime::RegisteredShellViewContribution> {
        self.service.shell_view(extension_id, view_id)
    }

    /// 信任库（根公钥、撤销表、已接受的 build）。
    pub(crate) fn trust(&self) -> &Trust {
        &self.trust
    }

    /// 登记一个跟随 provider 重启的 headless 连接 tab。
    pub(crate) fn register_headless_tab(
        &self,
        extension_id: String,
        runtime_id: String,
        tab: WeakEntity<crate::ExtensionConnectionTab>,
    ) {
        self.headless
            .borrow_mut()
            .entry(extension_id.clone())
            .or_default()
            .push((runtime_id.clone(), tab.clone()));
        self.tracked
            .borrow_mut()
            .entry(extension_id)
            .or_default()
            .push(Tracked::Headless { runtime_id, tab });
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

/// 注册 nsh 宿主（占用 `GlobalShellViewOpener`）。
///
/// 调用点保证与 gpui-shell 宿主互斥：两个宿主都往 `GlobalShellViewOpener`
/// 里放实现，后注册的覆盖先注册的。
pub fn init(service: UniversalPluginService, cx: &mut App) {
    let host = NshHost::new(
        service,
        dispatch::MainThread::new(cx),
        one_core::gpui_tokio::Tokio::handle(cx),
    );
    extension_view::register_shell_view_opener(Rc::new(host.clone()), cx);
    // 同一份宿主也接嵌入页：资源工作台里的 nsh 页面走的还是这套
    // session/ABI，只是主资源是借来的。
    page_host::install(host.clone(), cx);
    // provider 换进程后，已打开的页面要跟着重装或报错，否则会一直拿着
    // 作废的 generation 调用。
    host.start_monitor_bridge(cx);
    cx.set_global(host);
}

impl NshHost {
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
        let view_id = contribution
            .shell_view_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("这条扩展连接没有对应的 shell view"))?;
        let view = self
            .contribution(&contribution.extension_id, view_id)
            .ok_or_else(|| anyhow::anyhow!("扩展连接声明的 shell view 不存在"))?;
        // 凭据在内存里还原成明文随 open 载荷送到 provider；解析副本不落盘。
        let connection =
            crate::universal_plugins::resolve_extension_connection_for_runtime(connection, cx)?;
        let launch = connection::NshConnectionLaunch::new(&connection, &contribution, &view)?;
        let host = self.clone();
        let extension_id = contribution.extension_id.clone();
        let title: gpui::SharedString = connection.name.clone().into();
        let tab_id = format!("extension-connection:{connection_id}");
        let tab_container = cx
            .global::<one_core::tab_container::GlobalTabContainer>()
            .primary_pane();
        tab_container.update(cx, |tabs, cx| {
            tabs.activate_or_add_tab_lazy_with_mode(
                tab_id.clone(),
                mode,
                move |_window, cx| {
                    let registry_host = host.clone();
                    let tab = NshTab::open_connection(
                        view.clone(),
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

/// 打开一个扩展页面：singleton 的 view 复用同一个 tab id。
fn open_view(
    host: NshHost,
    contribution: extension_runtime::RegisteredShellViewContribution,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    use one_core::tab_container::{GlobalTabContainer, TabItem};

    let tab_id = if contribution.singleton {
        format!("nsh:{}", contribution.view_key)
    } else {
        format!(
            "nsh:{}:{}",
            contribution.view_key,
            NEXT_TAB_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )
    };
    let Some(tabs) = cx.try_global::<GlobalTabContainer>() else {
        tracing::warn!(
            extension_id = %contribution.extension_id,
            view_id = %contribution.id,
            "no GlobalTabContainer yet; nsh view not opened"
        );
        return;
    };
    let tabs = tabs.primary_pane();
    let extension_id = contribution.extension_id.clone();
    tabs.update(cx, |tabs, cx| {
        let host_for_tab = host.clone();
        tabs.activate_or_add_tab_lazy(
            tab_id.clone(),
            move |_window, cx| {
                let tab = NshTab::open(
                    contribution,
                    host_for_tab.service.clone(),
                    host_for_tab.trust().clone(),
                    host_for_tab.dispatcher(),
                    cx,
                );
                host_for_tab.track(&extension_id, tab.downgrade());
                TabItem::new(tab_id, format!("nsh:{extension_id}"), tab)
            },
            window,
            cx,
        )
    });
}

static NEXT_TAB_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

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

    use extension_runtime::RegisteredShellViewContribution;
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
    fn fixture_contribution() -> RegisteredShellViewContribution {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/nsh-host-test");
        RegisteredShellViewContribution {
            extension_id: "dev.navop.nsh-host-test".into(),
            extension_version: "0.1.0".into(),
            id: "self-check".into(),
            view_key: "dev.navop.nsh-host-test::self-check".into(),
            title: "自检页".into(),
            description: None,
            icon_path: None,
            entry_path: root.join("view.js"),
            extension_root: root,
            surface: extension_runtime::extension::manifest::ShellSurface::Tab,
            category: None,
            keywords: Vec::new(),
            singleton: true,
            backends: BTreeMap::new(),
            // 页面在启动时读宿主能力，所以台子要声明这几个模块（真实 manifest
            // 里就是 `contributes.shellViews[].modules: ["context","theme",…]`）。
            modules: [
                ShellHostModule::Context,
                ShellHostModule::Log,
                ShellHostModule::I18n,
                ShellHostModule::Theme,
                ShellHostModule::Resource,
                ShellHostModule::Runtime,
            ]
            .into_iter()
            .collect(),
            permissions: Vec::new(),
            shell_api_version: "1".into(),
            required_gpui_shell_version: String::new(),
            required_nsh_version: Some("0.3.0".into()),
        }
    }

    #[gpui::test]
    fn a_registered_view_loads_and_reaches_the_host(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            // 能力平面会话要 tokio 句柄；台子里现起一个最小的。
            one_core::gpui_tokio::init(cx);
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
                service,
                trust,
                dispatch::MainThread::new(cx),
                cx,
            );
            let tab_ref = tab.read(cx);
            assert_eq!(tab_ref.extension_id(), "dev.navop.nsh-host-test");
            assert_eq!(tab_ref.view_id(), "self-check");
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
        assert_eq!(
            mounted[0]["title"], "NSH Host",
            "回报数据来自页面: {}",
            mounted[0]
        );
        // 页面在回报里带上了宿主 ABI 的应答，这两条断言盯的是"页面调到的是
        // 宿主，而不是自己编的值"。
        assert_eq!(
            mounted[0]["viewId"], "self-check",
            "navop.context.current 的应答应带 view 身份: {}",
            mounted[0]
        );
        assert!(
            mounted[0]["locale"].as_str().is_some_and(|l| !l.is_empty()),
            "navop.i18n.locale 应答出一个语言: {}",
            mounted[0]
        );
        assert!(
            mounted[0]["themeRevision"].as_u64().is_some_and(|r| r > 0),
            "navop.theme.snapshot 应带版本号（空主题也算 1）: {}",
            mounted[0]
        );
    }
}
