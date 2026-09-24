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
        // 复用弹窗的登记表同样按 window_id 清理：窗口真的被销毁（非 macOS、隐藏失败回落、
        // 应用退出）而条目还留着，就会变成永远不会被复用的脏条目。
        crate::popup_window::forget_reusable_popup_by_window(window_id);
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
/// - `Ok(false)`：当前平台不走这套（非 macOS），调用方照旧销毁。
/// - `Err`：隐藏失败。调用方同样应照旧销毁。
///
/// 与 `remote_file_editor::editor_window_visibility::hide_for_reuse` 同构 —— 那份是
/// issue #262 的原始修法，已经上真机验证有效。两者将来应收敛成一份。
/// 实测背景与第二版修法方向见 skill `navop-macos-selector-availability-crash` §9.3 / §9.11。
#[cfg(target_os = "macos")]
pub fn hide_for_reuse(window: &Window) -> anyhow::Result<bool> {
    use anyhow::Context as _;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

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

/// 关闭一个原生窗口：macOS 上优先「隐藏后复用」，其他平台或隐藏失败时退回销毁。
///
/// 返回 `true` 表示窗口只是被隐藏、**仍然存活**。
///
/// ⚠️ 只有**登记过复用键**的窗口（用
/// [`crate::popup_window::open_reusable_popup_window`] 打开的）才会真的被隐藏；其余窗口
/// 一律照旧销毁 —— 隐藏一个没人会重新显示的窗口只是泄漏。这条兜底让视图层可以放心
/// 用这个函数替换 `window.remove_window()`，写错了也只是回到原行为。
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
pub fn close_window_for_reuse(window: &mut Window, cx: &mut App) -> bool {
    if !crate::popup_window::is_reusable_popup(window.window_handle().window_id()) {
        window.remove_window();
        return false;
    }

    match hide_for_reuse(window) {
        Ok(true) => {
            crate::popup_window::end_reusable_popup_session(window, cx);
            true
        }
        Ok(false) => {
            window.remove_window();
            false
        }
        Err(error) => {
            tracing::warn!(?error, "failed to hide the window; falling back to removal");
            window.remove_window();
            false
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
}
