//! 主窗口可见性适配：真正隐藏与恢复**已有**主窗口。
//!
//! GPUI 的 `App::hide()` 只覆盖 macOS（且是整应用级隐藏）；`Window` 也没有暴露
//! 隐藏/显示入口——只有 `minimize_window` / `activate_window`。托盘要求关闭按钮后
//! 主窗口从 Dock/任务栏消失、标签页与后台任务全部保留，所以这里直连各平台原生 API：
//!
//! - macOS：`NSWindow::orderOut:` 隐藏；`makeKeyAndOrderFront:` + `NSApplication::activate` 恢复
//! - Windows：`ShowWindow(SW_HIDE)` / `ShowWindow(SW_RESTORE)` + `SetForegroundWindow`
//! - Linux X11：`unmap_window` / `map_window`
//! - Linux Wayland：协议不允许客户端任意隐藏并重映射已有 xdg-toplevel，
//!   隐藏退化为 `minimize_window`（恢复仍由 `activate_window` 完成）
//!
//! # 为什么「恢复」要拆成两步
//!
//! 原生可见性/激活修改会**同步**回调进 GPUI：`gpui/src/window.rs` 的
//! `on_active_status_change` / `on_visibility_change` 直接调用 `handle.update(...)`
//! 去改窗口状态。如果在 `cx.update_window(...)` 的借用里改原生状态，回调立刻回头
//! 抢同一个 App 借用 ⇒ 二次借用失败，日志只剩一行
//! `ERROR gpui::window: RefCell already borrowed`（2026-09-17 真机实测，见下）。
//!
//! 所以恢复路径拆成「借用内取句柄」+「借用外改原生状态」：
//!
//! ```text
//! cx.update_window(handle, |_, window, _| main_window_target(window))  // 只读句柄
//! show_main_window(target)                                             // 借用已释放
//! ```
//!
//! 隐藏路径没这个问题：`orderOut:` 的可见性回调由 AppKit 异步投递，不在同一个借用内
//! 重入（真机实测隐藏无报错），所以 `hide_main_window` 仍可在借用内直接调用。
//!
//! 所有函数都必须在 GPUI 主线程调用。隐藏/恢复失败一律返回 `Err`，由调用方决定
//! 回退到退出确认还是保留进程。

use gpui::Window;

/// 主窗口的原生句柄快照。
///
/// 只承载「怎么找到这个原生窗口」的最小信息，不含任何借用，用来把原生状态修改挪到
/// GPUI 借用之外执行。取值来源固定为 [`main_window_target`]。
#[derive(Clone, Copy, Debug)]
pub(crate) struct NativeMainWindow(usize);

/// 读取主窗口的原生句柄，**不做任何原生状态修改**，因此可以在
/// `cx.update_window` 的借用内安全调用。
pub(crate) fn main_window_target(window: &Window) -> anyhow::Result<NativeMainWindow> {
    platform::target(window)
}

/// 显示并激活主窗口。对已经可见的窗口是幂等的，不会创建第二个窗口。
///
/// **必须在 GPUI 的窗口借用之外调用**，否则 AppKit/Win32 的激活回调会同步重入 GPUI
/// 造成二次借用（模块头注释有实测记录）。
pub(crate) fn show_main_window(target: NativeMainWindow) -> anyhow::Result<()> {
    platform::show(target)
}

/// 隐藏主窗口。成功后窗口不可见，但窗口实体、标签页、连接和后台任务都保留。
///
/// 可以在 GPUI 借用内调用：`orderOut:` 的可见性回调由 AppKit 异步投递（真机实测无报错）。
pub(crate) fn hide_main_window(window: &Window) -> anyhow::Result<()> {
    platform::hide(window)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod platform {
    use super::*;
    use anyhow::Context as _;
    // GPUI 的 `Window::window_handle()`（返回 `AnyWindowHandle`）会遮蔽
    // `raw_window_handle::HasWindowHandle::window_handle`，所以下面一律显式限定 trait。
    use raw_window_handle::HasWindowHandle;

    #[cfg(target_os = "macos")]
    pub(super) fn target(window: &Window) -> anyhow::Result<NativeMainWindow> {
        let handle =
            HasWindowHandle::window_handle(window).context("主窗口没有可用的原生窗口句柄")?;
        let raw_window_handle::RawWindowHandle::AppKit(raw) = handle.as_raw() else {
            anyhow::bail!("主窗口的原生句柄不是 AppKit 句柄");
        };
        Ok(NativeMainWindow(raw.ns_view.as_ptr() as usize))
    }

    #[cfg(target_os = "macos")]
    pub(super) fn show(target: NativeMainWindow) -> anyhow::Result<()> {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSView};

        let Some(mtm) = MainThreadMarker::new() else {
            anyhow::bail!("AppKit 窗口可见性只能在主线程修改");
        };

        // SAFETY: 指针来自 `main_window_target` 取到的 GPUI 原生 `NSView`；主窗口在进程
        // 存活期间不会被销毁（关闭按钮已被托盘策略拦截为隐藏），引用不逃逸本函数，
        // 且上面已确认运行在 AppKit 主线程。
        let view: &NSView =
            unsafe { (target.0 as *mut NSView).as_ref() }.context("主窗口的原生视图句柄已失效")?;
        let native: objc2::rc::Retained<objc2_app_kit::NSWindow> =
            view.window().context("NSView 尚未挂到 NSWindow 上")?;

        native.makeKeyAndOrderFront(None);
        NSApplication::sharedApplication(mtm).activate();

        if !native.isVisible() {
            anyhow::bail!("AppKit 未能显示主窗口（visibility=false）");
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub(super) fn hide(window: &Window) -> anyhow::Result<()> {
        let Some(_mtm) = objc2::MainThreadMarker::new() else {
            anyhow::bail!("AppKit 窗口可见性只能在主线程修改");
        };
        let native = native_window(window)?;
        native.orderOut(None);
        if native.isVisible() {
            anyhow::bail!("AppKit 未能隐藏主窗口（visibility=true）");
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn native_window(
        window: &Window,
    ) -> anyhow::Result<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
        use objc2_app_kit::NSView;

        let handle =
            HasWindowHandle::window_handle(window).context("主窗口没有可用的原生窗口句柄")?;
        let raw_window_handle::RawWindowHandle::AppKit(raw) = handle.as_raw() else {
            anyhow::bail!("主窗口的原生句柄不是 AppKit 句柄");
        };
        // SAFETY: 同 `show`：句柄指向仍然存活的 `NSView`，且已在主线程。
        let view: &NSView = unsafe { raw.ns_view.cast::<NSView>().as_ref() };
        view.window().context("NSView 尚未挂到 NSWindow 上")
    }

    #[cfg(target_os = "windows")]
    pub(super) fn target(window: &Window) -> anyhow::Result<NativeMainWindow> {
        let handle =
            HasWindowHandle::window_handle(window).context("主窗口没有可用的原生窗口句柄")?;
        let raw_window_handle::RawWindowHandle::Win32(raw) = handle.as_raw() else {
            anyhow::bail!("主窗口的原生句柄不是 Win32 句柄");
        };
        Ok(NativeMainWindow(raw.hwnd.get() as usize))
    }

    #[cfg(target_os = "windows")]
    pub(super) fn show(target: NativeMainWindow) -> anyhow::Result<()> {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            IsWindowVisible, SW_RESTORE, SetForegroundWindow, ShowWindow,
        };

        let hwnd = HWND(target.0 as *mut core::ffi::c_void);
        // SAFETY: `hwnd` 是本进程存活主窗口的句柄；两个调用对已处于目标状态的窗口都是
        // 幂等的，返回值只表示「之前的可见性」或「是否抢到前台」，不作为成功判据。
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
            if !IsWindowVisible(hwnd).as_bool() {
                anyhow::bail!("ShowWindow 未能显示主窗口");
            }
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    pub(super) fn hide(window: &Window) -> anyhow::Result<()> {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{IsWindowVisible, SW_HIDE, ShowWindow};

        let NativeMainWindow(raw_hwnd) = target(window)?;
        let hwnd = HWND(raw_hwnd as *mut core::ffi::c_void);
        // SAFETY: 同 `show`，句柄来自本进程存活主窗口。
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
            if IsWindowVisible(hwnd).as_bool() {
                anyhow::bail!("ShowWindow 未能隐藏主窗口");
            }
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(super) fn target(window: &Window) -> anyhow::Result<NativeMainWindow> {
        // 0 = 拿不到 X11 window id（Wayland 或非 X11 后端），恢复交给
        // GPUI 的 `activate_window`，隐藏退化为最小化。
        Ok(NativeMainWindow(
            x11_window_id(window)?.map_or(0, |id| id as usize),
        ))
    }

    #[cfg(target_os = "linux")]
    pub(super) fn show(target: NativeMainWindow) -> anyhow::Result<()> {
        if target.0 == 0 {
            return Ok(());
        }
        x11_set_mapped(target.0 as u32, true)
    }

    #[cfg(target_os = "linux")]
    pub(super) fn hide(window: &Window) -> anyhow::Result<()> {
        match x11_window_id(window)? {
            Some(window_id) => x11_set_mapped(window_id, false),
            None => {
                // Wayland（或拿不到 X11 window id）：没有通用的隐藏/重映射协议。
                // 隐藏退化为最小化；恢复由 `show_main_window` 之外的 `activate_window` 完成。
                tracing::warn!("当前显示服务器不支持真正隐藏顶层窗口，已退化为最小化到任务栏");
                window.minimize_window();
                Ok(())
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn x11_window_id(window: &Window) -> anyhow::Result<Option<u32>> {
        let handle =
            HasWindowHandle::window_handle(window).context("主窗口没有可用的原生窗口句柄")?;
        Ok(match handle.as_raw() {
            raw_window_handle::RawWindowHandle::Xcb(raw) => Some(raw.window.get()),
            raw_window_handle::RawWindowHandle::Xlib(raw) => u32::try_from(raw.window).ok(),
            _ => None,
        })
    }

    #[cfg(target_os = "linux")]
    fn x11_set_mapped(window_id: u32, mapped: bool) -> anyhow::Result<()> {
        use x11rb::connection::Connection as _;
        use x11rb::protocol::xproto::{map_window, unmap_window};

        // X11 的 window id 在 server 内全局唯一，unmap/map 只按 id 寻址，所以这里
        // 新建一条连接足够（raw-window-handle 0.6 的 Xcb 句柄不带连接可用）。
        let (connection, _screen) = x11rb::connect(None).context("无法连接 X11 server")?;
        if mapped {
            map_window(&connection, window_id)?;
        } else {
            unmap_window(&connection, window_id)?;
        }
        // X11 是异步协议：不 flush 请求不会真正下发。
        connection.flush()?;
        Ok(())
    }
}

/// 桌面三平台之外的兜底：明确失败，由调用方回退到既有退出确认。
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod platform {
    use super::*;

    pub(super) fn target(_window: &Window) -> anyhow::Result<NativeMainWindow> {
        anyhow::bail!("当前平台未实现托盘窗口可见性适配")
    }

    pub(super) fn show(_target: NativeMainWindow) -> anyhow::Result<()> {
        anyhow::bail!("当前平台未实现托盘窗口可见性适配")
    }

    pub(super) fn hide(_window: &Window) -> anyhow::Result<()> {
        anyhow::bail!("当前平台未实现托盘窗口可见性适配")
    }
}
