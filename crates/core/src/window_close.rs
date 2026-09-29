use std::{collections::HashMap, rc::Rc};

use gpui::{AnyWindowHandle, App, Global, Subscription, Window, WindowId};

type WindowCloseHandler = Rc<dyn Fn(AnyWindowHandle, &mut App) + 'static>;

#[derive(Default)]
struct WindowCloseState {
    handlers: HashMap<WindowId, Option<WindowCloseHandler>>,
}

impl WindowCloseState {
    fn register(&mut self, window_id: WindowId) {
        self.handlers.entry(window_id).or_default();
    }

    fn set_handler(&mut self, window_id: WindowId, handler: WindowCloseHandler) {
        self.handlers.insert(window_id, Some(handler));
    }

    fn handler(&self, window_id: WindowId) -> Option<WindowCloseHandler> {
        self.handlers.get(&window_id).and_then(Clone::clone)
    }

    fn remove(&mut self, window_id: WindowId) {
        self.handlers.remove(&window_id);
    }

    #[cfg(test)]
    fn contains(&self, window_id: WindowId) -> bool {
        self.handlers.contains_key(&window_id)
    }
}

struct WindowCloseRegistry {
    state: WindowCloseState,
    _window_closed_subscription: Subscription,
}

impl Global for WindowCloseRegistry {}

pub fn init(cx: &mut App) {
    if cx.has_global::<WindowCloseRegistry>() {
        return;
    }

    let subscription = cx.on_window_closed(|cx, window_id| {
        if cx.has_global::<WindowCloseRegistry>() {
            cx.global_mut::<WindowCloseRegistry>()
                .state
                .remove(window_id);
        }
        // 弹窗（复用登记的或一次性停放的）登记表都按 window_id 清理：窗口真的被销毁
        // （非 macOS、隐藏失败回落、应用退出）而条目还留着，就会变成永不失效的脏条目。
        crate::popup_window::forget_popup_by_window(window_id);
    });
    cx.set_global(WindowCloseRegistry {
        state: WindowCloseState::default(),
        _window_closed_subscription: subscription,
    });
}

pub fn register_window(window_handle: AnyWindowHandle, cx: &mut App) {
    init(cx);
    cx.global_mut::<WindowCloseRegistry>()
        .state
        .register(window_handle.window_id());
}

pub fn set_window_close_handler(
    window_handle: AnyWindowHandle,
    handler: impl Fn(AnyWindowHandle, &mut App) + 'static,
    cx: &mut App,
) {
    init(cx);
    cx.global_mut::<WindowCloseRegistry>()
        .state
        .set_handler(window_handle.window_id(), Rc::new(handler));
}

pub fn request_close_window(window_handle: AnyWindowHandle, cx: &mut App) {
    let handler = cx
        .try_global::<WindowCloseRegistry>()
        .and_then(|registry| registry.state.handler(window_handle.window_id()));

    if let Some(handler) = handler {
        handler(window_handle, cx);
        return;
    }

    cx.defer(move |cx| {
        let _ = window_handle.update(cx, |_, window, _| window.remove_window());
    });
}

/// 这个构建是否启用「关闭即隐藏」兜底（开着 `macos-touchbar-window-hide` 的 macOS 构建）。
///
/// 当前只有 `x86_64-apple-darwin` 的发布包会在打包时打开这个 feature
/// （见 `crates/core/Cargo.toml` 的 feature 说明与 `docs/macos-memory-investigation.md` §10）。
/// 判据里**没有架构条件**：Touch Bar 也存在于 Apple Silicon 的 13 英寸 MacBook Pro
/// （M1 2020 / M2 2022）上，那一侧需要同样保护时，给它的构建传同一个 feature 即可。
/// 未启用时所有隐藏路径都必须退回原来的「关闭即销毁」，行为与加这套机制之前一致。
///
/// 开关刻意收敛成**一个常量**而不是散落的 `#[cfg]`：弹窗（[`crate::popup_window`]）、
/// 编辑器窗口（`remote_file_editor::editor_window_visibility`）都要读它，写死在各处
/// 很容易漏掉一处而留下半开半关的状态。
pub const HIDE_WINDOWS_ON_CLOSE: bool = cfg!(all(
    target_os = "macos",
    feature = "macos-touchbar-window-hide"
));

/// 只隐藏原生窗口、**不销毁**它。macOS 上「关闭后复用」的基础动作。
///
/// # 为什么需要它
///
/// AppKit 的 Touch Bar 查找器（`_NSTouchBarFinder`）用 KVO 观察窗口及其 responder，而它
/// **只在下一个显示周期**（`NSDisplayCycleFlush`）里注销这些观察。如果窗口在那之前就已经
/// dealloc，`-[_NSTouchBarFinderObservation invalidate]` 会从
/// `removeObserver:forKeyPath:context:` 抛出一个 ObjC 异常；这条路径上没有任何人接住它，
/// 于是 `abort()` —— 用户看到的就是 `EXC_CRASH (SIGABRT)` 闪退。
///
/// 这个竞态只在带 Touch Bar 的机器上出现，所以在一台没有 Touch Bar 的开发机上怎么点都复现不了。
/// 把「关闭」换成「隐藏」可以从根上绕开它：窗口一直活着，AppKit 什么时候来注销都不会踩到
/// 已释放的对象。（另一条路是推迟 release，试过，被现场否证了。）
///
/// # 返回值
///
/// - `Ok(true)`：已隐藏。调用方**不要**再 `remove_window()`。
/// - `Ok(false)`：当前构建不走这套（非 macOS，或没开 `macos-touchbar-window-hide`），
///   调用方照旧销毁。
/// - `Err`：隐藏失败。**只有开了 `macos-touchbar-window-hide` 的构建才可能走到这里**
///   （未启用时函数在第一步就返回 `Ok(false)`），所以调用方**不要**退回销毁 —— 那正是
///   要规避的那条路径。受保护模式下的约定是「保留窗口、记录错误」，见
///   [`close_window_for_reuse`]。
///
/// 与 `remote_file_editor::editor_window_visibility::hide_for_reuse` 同构 —— 那份是
/// issue #262 的原始修法，已经上真机验证有效。两者将来应收敛成一份。
/// 实测背景与第二版修法方向见 skill `navop-macos-selector-availability-crash` §9.3 / §9.11。
#[cfg(target_os = "macos")]
pub fn hide_for_reuse(window: &Window) -> anyhow::Result<bool> {
    use anyhow::Context as _;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // 未启用兜底的构建（ARM macOS、Windows、Linux）直接说「不走这套」，让调用方按原行为
    // 销毁窗口。判据只有这一个常量，见 [`HIDE_WINDOWS_ON_CLOSE`]。
    if !HIDE_WINDOWS_ON_CLOSE {
        return Ok(false);
    }

    // NSWindow 只能在主线程上操作。
    let Some(_main_thread) = objc2::MainThreadMarker::new() else {
        anyhow::bail!("window must be hidden on the AppKit main thread");
    };
    let handle = HasWindowHandle::window_handle(window).context("no native window handle")?;
    let RawWindowHandle::AppKit(raw) = handle.as_raw() else {
        anyhow::bail!("window does not have an AppKit window handle");
    };
    // SAFETY: GPUI owns this live NSView for the duration of the window borrow; the
    // reference never leaves this call, and this call stays on AppKit's main thread.
    let view = unsafe { raw.ns_view.cast::<objc2_app_kit::NSView>().as_ref() };
    let native = view.window().context("NSView has no NSWindow")?;
    native.orderOut(None);
    anyhow::ensure!(!native.isVisible(), "AppKit did not hide the window");
    Ok(true)
}

/// 非 macOS 平台没有这套隐藏语义，一律交回调用方按原行为销毁。
#[cfg(not(target_os = "macos"))]
pub fn hide_for_reuse(_window: &Window) -> anyhow::Result<bool> {
    Ok(false)
}

/// 关闭动作的三种结果。
///
/// 「隐藏失败就退回销毁」曾经是故意的下坡路，但它和这套机制的目的事实相冲突：**受保护
/// 模式下的销毁同样会经过 AppKit 的关闭流程**，也就是要规避的那条路径。所以三种结果分开，
/// 由调用方（与日志）看得见差别，而不是悄悄降级成销毁。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowCloseOutcome {
    /// 原生窗口已隐藏、业务会话已结束。窗口仍然存活，同一个复用键可以重新显示它。
    Hidden,
    /// 窗口已销毁：要么它不是弹窗（主窗口、编辑器窗口这类不经过弹窗登记表的窗口），
    /// 要么当前构建没有打开 [`HIDE_WINDOWS_ON_CLOSE`]。
    Destroyed,
    /// **受保护模式下隐藏失败**：窗口和它的业务会话都保持原样，这次关闭没有生效。
    /// 调用方可以重试或提示用户；**不要**退回 `remove_window()`。
    Retained,
}

/// 关闭一个弹窗：macOS 上隐藏原生窗口（**不销毁**）并结束业务会话；不是弹窗、或当前构建
/// 没开保护时照旧销毁。
///
/// 返回值是 [`WindowCloseOutcome`]：调用方据此知道窗口是「隐藏复用」还是「已销毁」；
/// 受保护模式下隐藏失败时还会拿到 [`WindowCloseOutcome::Retained`]，表示**这次关闭没有
/// 生效**（窗口和它的业务会话都保持原样，调用方可以重试或提示用户）。
///
/// # 生效范围
///
/// 整套「隐藏不销毁」只在 [`HIDE_WINDOWS_ON_CLOSE`] 为真时生效，也就是打包时开了
/// `macos-touchbar-window-hide` 的 macOS 构建（当前发布流水线只给 `x86_64-apple-darwin`
/// 打开）。未启用时这个函数对弹窗的结果与普通窗口一样：`remove_window()`，即加这套机制
/// 之前的行为。
///
/// # 弹窗一律不销毁
///
/// 开关打开时，凡是通过 [`crate::popup_window::open_popup_window`] / [`crate::popup_window::open_reusable_popup_window`]
/// 打开的窗口，关闭时都只隐藏 —— 包括**没有复用键的一次性弹窗**。这不是保守，而是现场
/// 逼出来的：只要窗口是在**某条路线**上被销毁的，那条路线就会被 AppKit 的 Touch Bar
/// 观察者延迟注销踩到（#308 「确定」、#314 「保存」，以及 0.3.118 版里唯一还在崩的红点）。
/// 把销毁从应用运行期间彻底拿掉，这类现场就没了。
///
/// 代价是真实的，说在明处：一次性弹窗隐藏后没人会重新显示它，下一次打开是**新建**窗口，
/// 所以停放窗口数会随打开次数增长。留给后续的两条路：（1）把热点弹窗改成
/// [`crate::popup_window::open_reusable_popup_window`]（复用同一个窗口，数量收敛到「弹窗
/// 种类数」）；（2）给停放窗口数设上限，超过时按我们自己的路线延迟销毁最旧的一个 —— 那时
/// 销毁已经不在 AppKit 的关闭流程里。
///
/// 不在这条链上的窗口（主窗口）照旧走各自的关闭语义：主窗口关闭 = 隐藏到托盘或退出应用，
/// 与这套机制无关。
///
/// # 「关闭」包含两件事，缺一不可
///
/// 1. **原生窗口**：隐藏并留着复用 —— 这是绕开 AppKit Touch Bar 观察者注销崩溃的那一步；
/// 2. **业务会话**：结束掉 —— 卸载业务 view 及它持有的数据与任务句柄，清掉焦点和通知。
///
/// 第 2 步不能省。只隐藏、把上一次的 view 留到下次打开才替换，就会在用户不再打开那类窗口时
/// 一直扣着那份数据（**强引用来自仍然存活的内容树**，注册表里的 `WeakEntity` 管不到它）。
/// 会话卸载在关闭动作内部**同步**完成，所以不存在「上一轮的清理删掉刚打开的新会话」的窗口期 ——
/// 这也是不把它交给 `defer` 的原因。
///
/// 参数里有 `cx` 就是因为第 2 步必须能更新内容实体；只有 `&mut Window` 的接口做不到。
pub fn close_window_for_reuse(window: &mut Window, cx: &mut App) -> WindowCloseOutcome {
    // 不是弹窗：照旧销毁。它不在这条复用链上，也没有「AppKit 在关闭流程里销毁弹窗」
    // 那个崩溃点；顺手也把视图层「改错了本该销毁的窗口」变成隐藏的风险挡住。
    if !crate::popup_window::is_popup_window(window.window_handle().window_id()) {
        window.remove_window();
        return WindowCloseOutcome::Destroyed;
    }

    match hide_for_reuse(window) {
        Ok(true) => {
            crate::popup_window::end_popup_session(window, cx);
            WindowCloseOutcome::Hidden
        }
        Ok(false) => {
            window.remove_window();
            WindowCloseOutcome::Destroyed
        }
        Err(error) => {
            // `Err` 只可能来自开了 `macos-touchbar-window-hide` 的构建：未启用时
            // `hide_for_reuse` 在第一步就返回 `Ok(false)`（契约测试
            // `the_hide_switch_gates_every_link_of_the_chain` 钉住这个顺序）。
            //
            // 不销毁：受保护模式下的销毁同样要经过 AppKit 的关闭流程，正是要规避的那条
            // 路径。窗口与它的业务会话保持原样，调用方从 `Retained` 知道这次关闭没生效。
            tracing::error!(
                ?error,
                "failed to hide the window; keeping it alive instead of destroying it"
            );
            WindowCloseOutcome::Retained
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_registration_defaults_to_direct_close() {
        let window_id = WindowId::from(1);
        let mut state = WindowCloseState::default();

        state.register(window_id);

        assert!(state.contains(window_id));
        assert!(state.handler(window_id).is_none());
    }

    #[test]
    fn custom_handler_replaces_the_default_close_route() {
        let window_id = WindowId::from(2);
        let mut state = WindowCloseState::default();
        let handler: WindowCloseHandler = Rc::new(|_, _| {});

        state.register(window_id);
        state.set_handler(window_id, handler);

        assert!(state.handler(window_id).is_some());
    }

    #[test]
    fn closed_window_is_removed_from_the_registry() {
        let window_id = WindowId::from(3);
        let mut state = WindowCloseState::default();
        state.register(window_id);

        state.remove(window_id);

        assert!(!state.contains(window_id));
    }

    /// 除主窗口外，业务代码里不允许再自己销毁窗口。
    ///
    /// 「关闭即隐藏」只要漏掉一个入口就等于没修：那条入口照样会销毁原生窗口，AppKit 的
    /// Touch Bar 观察者照样会在下一个显示周期里踩到已释放的对象（#308 的「确定」和
    /// 「取消」、#314 的「保存」都是这么漏出来的）。所以把「弹窗 / 表单窗口把自己的关闭动作
    /// 一律交给 `close_window_for_reuse`」钉成一条源码契约。
    ///
    /// 允许直接销毁的地方只剩下面三类，它们都不在契约清单里：
    /// - 主窗口那条链（关闭主窗口 = 隐藏到托盘或退出应用）；
    /// - `close_window_for_reuse` 自己，以及 `hide_for_reuse` 返回 `Ok(false)` / `Err` 时的兜底；
    /// - 编辑器窗口的兜底分支（`prepare_window_close` 返回 `true` 才销毁，已经在它自己的
    ///   测试里逐个方法断言过）。
    #[test]
    fn secondary_windows_never_destroy_themselves() {
        // include_str! 用的是相对于本文件的路径，所以从 crates/core/src 往上退三级。
        let sources: [(&str, &str); 15] = [
            (
                "main/src/new_connection/connection_window.rs",
                include_str!("../../../main/src/new_connection/connection_window.rs"),
            ),
            (
                "main/src/credential_vault/form_window.rs",
                include_str!("../../../main/src/credential_vault/form_window.rs"),
            ),
            (
                "crates/db_view/src/connection_form_window.rs",
                include_str!("../../../crates/db_view/src/connection_form_window.rs"),
            ),
            (
                "crates/connection_form/src/middleware_form/window.rs",
                include_str!("../../../crates/connection_form/src/middleware_form/window.rs"),
            ),
            (
                "crates/mongodb_view/src/mongo_form_window.rs",
                include_str!("../../../crates/mongodb_view/src/mongo_form_window.rs"),
            ),
            (
                "crates/redis_view/src/redis_form_window.rs",
                include_str!("../../../crates/redis_view/src/redis_form_window.rs"),
            ),
            (
                "crates/remote_desktop_view/src/remote_desktop_form.rs",
                include_str!("../../../crates/remote_desktop_view/src/remote_desktop_form.rs"),
            ),
            (
                "crates/remote_desktop_view/src/remote_desktop_form/connection_test.rs",
                include_str!(
                    "../../../crates/remote_desktop_view/src/remote_desktop_form/connection_test.rs"
                ),
            ),
            (
                "crates/terminal_view/src/ssh_form_window.rs",
                include_str!("../../../crates/terminal_view/src/ssh_form_window.rs"),
            ),
            (
                "crates/terminal_view/src/ftp_form_window.rs",
                include_str!("../../../crates/terminal_view/src/ftp_form_window.rs"),
            ),
            (
                "crates/terminal_view/src/telnet_form_window.rs",
                include_str!("../../../crates/terminal_view/src/telnet_form_window.rs"),
            ),
            (
                "crates/terminal_view/src/serial_form_window.rs",
                include_str!("../../../crates/terminal_view/src/serial_form_window.rs"),
            ),
            (
                "crates/port_forwarding_view/src/form_window.rs",
                include_str!("../../../crates/port_forwarding_view/src/form_window.rs"),
            ),
            (
                "crates/port_forwarding_view/src/view.rs",
                include_str!("../../../crates/port_forwarding_view/src/view.rs"),
            ),
            (
                "crates/universal-plugins/src/extension_connection_form.rs",
                include_str!("../../../crates/universal-plugins/src/extension_connection_form.rs"),
            ),
        ];

        for (path, source) in sources {
            assert!(
                !source.contains("window.remove_window()"),
                "{path} 里还有直接销毁窗口的写法：这类窗口必须走 one_core::window_close::close_window_for_reuse，\
                 否则带 Touch Bar 的 Mac 上关闭时会闪退"
            );
        }
    }
}
