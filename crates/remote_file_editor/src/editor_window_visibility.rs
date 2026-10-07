//! Keep the macOS editor's native window alive across close/reopen (issue #262).
//! This avoids the suspected Touch Bar observer teardown trigger, not all
//! possible AppKit exceptions. The caller still owns the GPUI window.
//!
//! 与 `one_core::window_close::hide_for_reuse` 同构（那份管弹窗，这份管编辑器窗口），
//! 因此同样受 `one_core::window_close::HIDE_WINDOWS_ON_CLOSE` 门控：只有打包时传了
//! `macos-touchbar-window-hide` 的 macOS 包才隐藏。该开关当前恒为 `false`（上游 zed#65186
//! 已修掉根因），所以所有构建都退回销毁。

/// `true` means hidden and reusable; `false` preserves other platforms' close
/// behavior. An error means the build opted in but the hide failed: the caller must
/// keep the window (and its session) instead of falling back to removal — removal is
/// the very AppKit close path this switch exists to avoid.
#[cfg(target_os = "macos")]
pub(super) fn hide_for_reuse(window: &gpui::Window) -> anyhow::Result<bool> {
    use anyhow::Context as _;
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // 未启用兜底的构建（ARM macOS / Windows / Linux）按原行为销毁；判据与弹窗共用同一个常量。
    if !one_core::window_close::HIDE_WINDOWS_ON_CLOSE {
        return Ok(false);
    }

    let Some(_main_thread) = objc2::MainThreadMarker::new() else {
        anyhow::bail!("remote editor window must be hidden on the AppKit main thread");
    };
    let handle = HasWindowHandle::window_handle(window)
        .context("remote editor has no native window handle")?;
    let RawWindowHandle::AppKit(raw) = handle.as_raw() else {
        anyhow::bail!("remote editor does not have an AppKit window handle");
    };
    // SAFETY: GPUI owns this live NSView for the duration of the window borrow.
    // The reference stays within this function and AppKit's main thread.
    let view: &NSView = unsafe { raw.ns_view.cast::<NSView>().as_ref() };
    let native = view
        .window()
        .context("remote editor NSView has no NSWindow")?;
    native.orderOut(None);
    anyhow::ensure!(!native.isVisible(), "AppKit did not hide the remote editor");
    Ok(true)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn hide_for_reuse(_window: &gpui::Window) -> anyhow::Result<bool> {
    Ok(false)
}
