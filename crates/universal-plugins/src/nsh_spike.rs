//! M2 spike：把一个 nsh `AppView` 挂进 navop 的 TabContainer。
//!
//! 这是 nsh 迁移方案（`docs/extension-resource-plugins/nsh-migration-plan.md`）
//! 的第一块接线：证明「navop 窗口 + nsh 扩展页面」可以在同一个 gpui-fast
//! 进程里共存、主题能从宿主桥过去。后续 M3 会把这里替换成正式的
//! `NshHost`（TrustStore 两阶段装载 + `navop.json` 解析）。
//!
//! 页面源码在 `resources/nsh-spike/App.tsx`，编译产物（`dist/bundle.js` +
//! source map）随 crate 一起入库并由 `include_str!` 打进二进制 —— 这正是
//! 迁移方案里扩展的分发形态（作者侧 `nsh build`，宿主只装载已构建的
//! bundle，不做编译）。改 TSX 后重新生成：
//!
//! ```bash
//! cd crates/universal-plugins/resources/nsh-spike
//! nsh build --out dist   # 产出 dist/bundle.js + dist/bundle.js.map
//! ```
//!
//! 页面只用 `nsh` / `nsh/base`，不依赖宿主 nativeModule，所以不需要
//! `set_ui_sets`（引擎默认就带 `nsh_base::registry()`）。

use std::rc::Rc;

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Task, Window, div,
};
use gpui_component::ActiveTheme as _;
use nsh_platform_gpui::{AdaptiveColor, AppOptions, AppView, Appearance, Bundle, Hsla};
use one_core::tab_container::{GlobalTabContainer, TabContent, TabContentEvent, TabItem};

/// spike 页面的 tab id（singleton 形态，重复打开会聚焦同一页）。
const SPIKE_TAB_ID: &str = "nsh-spike:embed-gpui";

/// `nsh_core::element::tokens::{FONT_SANS, FONT_MONO}` 的字面值（"font-sans"
/// / "font-mono"）；nsh-platform-gpui 没有重导出 tokens 模块，spike 不为此
/// 引入 nsh-core 直接依赖。
const FONT_SANS: &str = "font-sans";
const FONT_MONO: &str = "font-mono";

/// spike 页面 id（`nsh.json` 的 `id`，也是它落盘数据目录的名字）。
const SPIKE_APP_ID: &str = "dev.navop.spike.embed-gpui";

/// 已构建的页面代码与它声明的能力（作者侧 `nsh build` 的产物）。
const SPIKE_JS: &str = include_str!("../resources/nsh-spike/dist/bundle.js");
const SPIKE_CAPABILITIES: &str = include_str!("../resources/nsh-spike/nsh.json");

/// 把入库的页面组装成引擎认的 [`Bundle`]。
///
/// 走 `Bundle::source` 而不是 `read_trusted`：源码随二进制发布、没有外部
/// 文件需要信任；能力仍然显式声明并解析（页面拿不到未声明的 `host.call`）。
fn spike_bundle() -> Bundle {
    let manifest: serde_json::Value =
        serde_json::from_str(SPIKE_CAPABILITIES).expect("nsh.json 必须是合法 JSON");
    let mut bundle = Bundle::source(SPIKE_JS.to_owned());
    bundle.app_id = Some(SPIKE_APP_ID.to_owned());
    match manifest.get("capabilities") {
        Some(declared) => {
            let (capabilities, dropped) =
                nsh_core::access::Capabilities::from_json_lenient(declared)
                    .expect("nsh.json 的 capabilities 必须合法");
            if !dropped.is_empty() {
                tracing::warn!(?dropped, "nsh spike capabilities: 有字段被丢弃");
            }
            bundle.policy.declared = Some(capabilities);
        }
        None => tracing::warn!("nsh spike: nsh.json 没有声明 capabilities，页面将拿不到任何能力"),
    }
    bundle
}

/// gpui 颜色 → nsh 颜色（通道一一对应）。
fn color(c: gpui::Hsla) -> Hsla {
    Hsla {
        h: c.h,
        s: c.s,
        l: c.l,
        a: c.a,
    }
}

/// gpui-component 主题 → nsh 主题（shadcn token），映射同 embed-gpui 示例。
///
/// navop 的组件主题就是 gpui-kit 的 `Theme`，token 一一对应；宿主主题或
/// 明暗变化时重新计算并 `set_theme` 推给 AppView。
fn nsh_theme(theme: &gpui_component::Theme) -> nsh_platform_gpui::Theme {
    let mut nsh = nsh_platform_gpui::Theme::default();
    let colors = &theme.colors;
    for (name, c) in [
        ("background", colors.background),
        ("foreground", colors.foreground),
        ("card", colors.group_box),
        ("card-foreground", colors.group_box_foreground),
        ("popover", colors.popover),
        ("popover-foreground", colors.popover_foreground),
        ("primary", colors.primary),
        ("primary-foreground", colors.primary_foreground),
        ("secondary", colors.secondary),
        ("secondary-foreground", colors.secondary_foreground),
        ("muted", colors.muted),
        ("muted-foreground", colors.muted_foreground),
        ("accent", colors.accent),
        ("accent-foreground", colors.accent_foreground),
        ("destructive", colors.danger),
        ("border", colors.border),
        ("input", colors.input),
        ("ring", colors.ring),
        ("chart-1", colors.chart_1),
        ("chart-2", colors.chart_2),
        ("chart-3", colors.chart_3),
        ("chart-4", colors.chart_4),
        ("chart-5", colors.chart_5),
        ("sidebar", colors.sidebar),
        ("sidebar-foreground", colors.sidebar_foreground),
        ("sidebar-primary", colors.sidebar_primary),
        (
            "sidebar-primary-foreground",
            colors.sidebar_primary_foreground,
        ),
        ("sidebar-accent", colors.sidebar_accent),
        (
            "sidebar-accent-foreground",
            colors.sidebar_accent_foreground,
        ),
        ("sidebar-border", colors.sidebar_border),
        ("sidebar-ring", colors.ring),
    ] {
        nsh.set_color(name, AdaptiveColor::from(color(c)));
    }
    nsh.set_number("radius", f32::from(theme.radius));
    nsh.set_font(FONT_SANS, theme.font_family.to_string());
    nsh.set_font(FONT_MONO, theme.mono_font_family.to_string());
    nsh
}

fn appearance(mode: gpui_component::ThemeMode) -> Appearance {
    if mode.is_dark() {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// 承载 nsh AppView 的 tab 页面。
pub struct NshSpikeTab {
    view: Entity<AppView>,
    title: SharedString,
}

impl NshSpikeTab {
    fn new(window: &mut Window, cx: &mut App) -> Self {
        let bundle = spike_bundle();
        let options = AppOptions {
            app_id: SPIKE_APP_ID.into(),
            // 页面没有图片等外部资产：素材都随 JS 打进 bundle。
            asset_base: None,
            ..AppOptions::default()
        };
        let view = cx.new(|cx| AppView::new(options, window, cx));
        push_theme(&view, cx);
        view.update(cx, |view, _cx| {
            view.on_error(|message| tracing::error!("nsh spike app: {message}"));
            view.load(nsh_platform_gpui::App::Js(bundle));
        });
        Self {
            view,
            title: "NSH Spike".into(),
        }
    }
}

/// 宿主当前主题 → AppView（首帧前或主题变化后调用）。
fn push_theme(view: &Entity<AppView>, cx: &mut App) {
    let theme = cx.theme();
    let (nsh, look) = (nsh_theme(theme), appearance(theme.mode));
    view.update(cx, |view, cx| {
        view.set_theme(Some(nsh), cx);
        view.set_appearance(look, cx);
    });
}

impl EventEmitter<TabContentEvent> for NshSpikeTab {}

impl Focusable for NshSpikeTab {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.view.read(cx).focus_handle(cx)
    }
}

impl Render for NshSpikeTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.view.clone())
    }
}

impl TabContent for NshSpikeTab {
    fn content_key(&self) -> &'static str {
        "NshSpike"
    }

    fn title(&self, _cx: &App) -> SharedString {
        self.title.clone()
    }

    fn can_rename(&self, _cx: &App) -> bool {
        false
    }
}

/// 注册 spike opener：占用 `GlobalShellViewOpener`，把所有 shell view 打开
/// 请求导到同一个示例页。只在 `nsh-spike` 开启且 gpui-shell 宿主
/// （`shell-plugins`）未注册时调用 —— 两个宿主并存时 gpui-shell（现行
/// 机制）优先。
pub fn init(cx: &mut App) {
    extension_view::register_shell_view_opener(Rc::new(NshSpikeOpener), cx);
    // 冒烟入口：`NAVOP_NSH_SPIKE=1` 时窗口一出来就把 spike 页开成 tab，
    // 不用先装一个带 shellViews 的扩展才看得见（M2 只证明共存，不改产品路径）。
    if std::env::var_os("NAVOP_NSH_SPIKE").is_none() {
        return;
    }
    cx.spawn(async move |cx| {
        for _ in 0..100 {
            let window = cx.update(|cx| cx.windows().first().copied());
            if let Some(window) = window {
                let _ = window.update(cx, |_, window, cx| open_in(window, cx));
                return;
            }
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;
        }
        tracing::warn!("NAVOP_NSH_SPIKE set but no window appeared in 10s");
    })
    .detach();
}

/// 把 spike 页开成主面板的 tab（singleton：已开就聚焦）。
fn open_in(window: &mut Window, cx: &mut App) {
    let Some(tabs) = cx.try_global::<GlobalTabContainer>() else {
        tracing::warn!("no GlobalTabContainer yet; spike tab not opened");
        return;
    };
    let tabs = tabs.primary_pane();
    tabs.update(cx, |tabs, cx| {
        tabs.activate_or_add_tab_lazy(
            SPIKE_TAB_ID,
            |window, cx| {
                let tab = cx.new(|cx| NshSpikeTab::new(window, cx));
                TabItem::new(SPIKE_TAB_ID, "nsh-spike", tab)
            },
            window,
            cx,
        )
    });
}

/// 打开 spike tab 的 opener：任何 extension/view id 都开同一个示例页。
struct NshSpikeOpener;

impl extension_view::ShellViewOpener for NshSpikeOpener {
    fn open(&self, _extension_id: &str, _view_id: &str, window: &mut Window, cx: &mut App) {
        open_in(window, cx);
    }

    fn close_extension(
        &self,
        _extension_id: &str,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<bool> {
        Task::ready(true)
    }

    fn finish_extension_change(&self, _extension_id: &str) {}
}

/// M2 的验收台子：同一进程里 navop 的 GPUI 窗口能加载并跑起 nsh 页面。
///
/// 证据不用"没 panic"：spike 页面挂载时会 `host.call("spike.mounted", ...)`，
/// 这里注册同名 handler 并断言它带着页面的数据到达。走到这一步意味着
/// 字节码在引擎线程上装载、QuickJS 执行了 TSX、元素树 mount 完成、
/// host 能力白名单放行、跨线程 host 桥回得来 —— 也就是 M2 要证明的全部。
///
/// ```bash
/// cargo test -p universal-plugins --features nsh-spike -- nsh_spike
/// ```
#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        ops::Deref as _,
        rc::Rc,
        sync::{Arc, Mutex},
    };

    use gpui::{Render, TestAppContext, VisualTestContext};

    use super::*;

    /// 台子里占位的根视图：真正要画的是它 child 出去的 `NshSpikeTab`。
    struct Empty;

    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
        }
    }

    #[gpui::test]
    fn nsh_spike_page_mounts_and_reaches_the_host(cx: &mut TestAppContext) {
        // 主题桥要读 gpui-component 的 Theme global：生产里由 navop_app 的
        // 启动流程 init，台子里自己补上。
        cx.update(|cx| {
            gpui_component::init(cx);
        });
        // nsh 的引擎有自己的线程（页面的 VM/布局/绘制都在那），而 gpui 的
        // 测试调度器默认断言"只有测试线程在跑"。这个台子要的正是那个线程
        // 干真活，所以显式放开（非确定性只影响时序，不断言时序）。
        cx.executor().allow_parking();
        let window = cx.add_window(|_, _| Empty);
        let mut context = VisualTestContext::from_window(*window.deref(), cx);

        let errors = Rc::new(RefCell::new(Vec::<String>::new()));
        // host handler 跑在引擎线程上，必须 Send + Sync。
        let mounted = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let tab = {
            let errors = errors.clone();
            let mounted = mounted.clone();
            context.update(|window, cx| {
                let tab = cx.new(|cx| NshSpikeTab::new(window, cx));
                tab.read(cx).view.clone().update(cx, |view, _| {
                    let errors = errors.clone();
                    view.on_error(move |message| errors.borrow_mut().push(message.to_owned()));
                    let mounted = mounted.clone();
                    view.host().handle("spike.mounted", move |call| {
                        let data = call.data().clone();
                        if let Ok(mut mounted) = mounted.lock() {
                            mounted.push(data);
                        }
                        call.resolve(serde_json::Value::Null);
                    });
                });
                tab
            })
        };

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

        // 引擎在自己的线程上装载字节码、跑 TSX、mount，这个进程外异步
        // 不会被 `run_until_parked` 推着走：要真等。每轮画一帧（surface 的
        // 首帧会启动引擎）+ 泵主线程 + 给引擎线程一点时间。
        for _ in 0..100 {
            draw(&mut context);
            context.run_until_parked();
            if !mounted.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // 报完到再画一帧，确认回报之后页面还在正常出帧。
        draw(&mut context);
        context.run_until_parked();

        let errors = errors.borrow();
        assert!(errors.is_empty(), "nsh 页面报错: {errors:?}");
        let mounted = mounted.lock().unwrap();
        assert_eq!(
            mounted.len(),
            1,
            "页面挂载时应恰好回报一次 spike.mounted，实际 {} 次",
            mounted.len()
        );
        assert_eq!(
            mounted[0]["title"], "NSH Spike",
            "回报数据来自页面: {}",
            mounted[0]
        );
    }
}
