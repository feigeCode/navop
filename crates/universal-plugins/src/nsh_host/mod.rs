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
mod bundle;
mod connection;
mod decision;
mod dev;
mod dispatch;
mod grant;
mod monitor;
mod native;
mod opener;
mod plugin_ai_panel;
mod session;
mod tab;
mod theme;

pub use dev::{DevOps, set_dev_ops};
pub use native::{NativeFactory, NativeViewContext, register_native_component};
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

/// 需要跟随 provider 重启的实体：扩展页面。
#[derive(Clone)]
pub(crate) enum Tracked {
    Page(WeakEntity<NshTab>),
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
fn open_view(
    host: NshHost,
    contribution: extension_runtime::RegisteredShellAppContribution,
    route: Option<&str>,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    use one_core::tab_container::{GlobalTabContainer, TabItem};

    let tab_id = match route {
        Some(route) => format!("nsh:app:{}#{route}", contribution.extension_id),
        None => format!("nsh:app:{}", contribution.extension_id),
    };
    let Some(tabs) = cx.try_global::<GlobalTabContainer>() else {
        tracing::warn!(
            extension_id = %contribution.extension_id,
            "no GlobalTabContainer yet; nsh app not opened"
        );
        return;
    };
    let tabs = tabs.primary_pane();
    let extension_id = contribution.extension_id.clone();
    let route = route.map(str::to_string);
    tabs.update(cx, |tabs, cx| {
        let host_for_tab = host.clone();
        tabs.activate_or_add_tab_lazy(
            tab_id.clone(),
            move |_window, cx| {
                let tab = NshTab::open(
                    contribution,
                    route,
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

        for _ in 0..100 {
            draw_probe(&mut context);
            context.run_until_parked();
            if probes.lock().unwrap().len() >= 3 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

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
        for _ in 0..100 {
            draw_probe(&mut context);
            context.run_until_parked();
            if probes.lock().unwrap().len() >= 4 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

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
    }

    /// 手写的探针页：不依赖构建工具，直接跑在 nsh 引擎里。
    const PROBE_PAGE: &str = r#"import { host, mount, View } from "nsh";

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
  host.on("navop.event", (frame) => report({ kind: "event", frame: frame }));
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
            modules: [ShellHostModule::Resource, ShellHostModule::Dev]
                .into_iter()
                .collect(),
            permissions: Vec::new(),
            shell_api_version: "2".into(),
            required_nsh_version: Some("0.3.0".into()),
        }
    }
}
