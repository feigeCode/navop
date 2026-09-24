//! Keep the macOS editor's native window alive across close/reopen (issue #262).
//! This avoids the suspected Touch Bar observer teardown trigger, not all
//! possible AppKit exceptions. The caller still owns the GPUI window.

/// `true` means hidden and reusable; `false` preserves other platforms' close
/// behavior. Errors let the caller fall back to actual window removal.
#[cfg(target_os = "macos")]
pub(super) fn hide_for_reuse(window: &gpui::Window) -> anyhow::Result<bool> {
    use anyhow::Context as _;
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

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
