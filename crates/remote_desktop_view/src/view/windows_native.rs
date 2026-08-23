use gpui::{Bounds, Pixels, Point};

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
use super::windows_native_overlay::{
    WindowsNativeOverlay, WindowsNativeOverlayBounds, WindowsNativeOverlayError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Win32ClientPhysicalBounds {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: i32,
    pub(super) height: i32,
}

pub(super) fn logical_bounds_to_physical(
    bounds: Bounds<Pixels>,
    parent_client_origin: Point<Pixels>,
    scale_factor: f32,
) -> Option<Win32ClientPhysicalBounds> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }

    let origin_x = pixels_to_f64(bounds.origin.x) - pixels_to_f64(parent_client_origin.x);
    let origin_y = pixels_to_f64(bounds.origin.y) - pixels_to_f64(parent_client_origin.y);
    let width = pixels_to_f64(bounds.size.width);
    let height = pixels_to_f64(bounds.size.height);
    if width < 0.0 || height < 0.0 {
        return None;
    }

    Some(Win32ClientPhysicalBounds {
        x: scale_physical(origin_x, scale_factor)?,
        y: scale_physical(origin_y, scale_factor)?,
        width: scale_physical(width, scale_factor)?,
        height: scale_physical(height, scale_factor)?,
    })
}

fn pixels_to_f64(value: Pixels) -> f64 {
    let value: f32 = value.into();
    f64::from(value)
}

fn scale_physical(value: f64, scale_factor: f32) -> Option<i32> {
    let value = (value * f64::from(scale_factor)).round();
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return None;
    }
    Some(value as i32)
}

trait NativePresentationSink {
    type Error;

    fn set_bounds(&mut self, bounds: Win32ClientPhysicalBounds) -> Result<(), Self::Error>;
    fn show(&mut self) -> Result<(), Self::Error>;
    fn focus_child(&mut self) -> Result<(), Self::Error>;
    fn focus_parent(&mut self) -> Result<(), Self::Error>;
    fn hide(&mut self) -> Result<(), Self::Error>;
    /// Whether the native overlay is actually visible right now, independent
    /// of any requested visibility.
    fn is_effectively_visible(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NativePresentationState {
    #[default]
    Open,
    Closing,
    Destroyed,
}

#[derive(Debug, Default)]
struct WindowsNativePresentation {
    state: NativePresentationState,
    /// Requested activation (tab active).
    active: bool,
    /// Requested visibility; only set to true after a successful show.
    visible: bool,
    latest_bounds: Option<Win32ClientPhysicalBounds>,
    /// Set by LoginComplete / Reconnected; the overlay must never present
    /// before the session has a drawable framebuffer.
    login_complete: bool,
    /// Last native presentation-state query: the ActiveX drawing window is
    /// inside the host subtree with non-zero rects.
    native_child_ready: bool,
    /// Native focus requested while the replacement ActiveX drawing child was
    /// not ready. The next successful activation delivers it.
    focus_on_activate: bool,
    /// Actual overlay visibility verified against the HWND, not just requested.
    effective_visible: bool,
}

impl WindowsNativePresentation {
    fn update_bounds<S: NativePresentationSink>(
        &mut self,
        bounds: Win32ClientPhysicalBounds,
        sink: &mut S,
    ) -> Result<(), S::Error> {
        if self.state != NativePresentationState::Open {
            return Ok(());
        }
        self.latest_bounds = Some(bounds);
        if self.active {
            sink.set_bounds(bounds)?;
            self.effective_visible = sink.is_effectively_visible();
        }
        Ok(())
    }

    fn activate<S: NativePresentationSink>(
        &mut self,
        focus_child: bool,
        sink: &mut S,
    ) -> Result<(), S::Error> {
        if self.state != NativePresentationState::Open {
            return Ok(());
        }
        if self.active {
            return self.deliver_pending_focus(sink);
        }
        self.focus_on_activate |= focus_child;
        // Readiness gate: never show the overlay before the session has
        // completed login/reconnect AND the native child is structurally
        // ready. The view re-synchronizes after LoginComplete/Reconnected.
        if !self.can_present() {
            return Ok(());
        }

        if let Some(bounds) = self.latest_bounds {
            sink.set_bounds(bounds)?;
        }
        if !self.visible {
            sink.show()?;
            self.visible = true;
        }
        self.effective_visible = sink.is_effectively_visible();
        self.active = true;
        self.deliver_pending_focus(sink)
    }

    fn focus<S: NativePresentationSink>(&mut self, sink: &mut S) -> Result<(), S::Error> {
        if self.state != NativePresentationState::Open {
            return Ok(());
        }
        if self.active && self.visible {
            self.focus_on_activate = true;
            self.deliver_pending_focus(sink)?;
        } else {
            self.focus_on_activate = true;
        }
        Ok(())
    }

    fn deliver_pending_focus<S: NativePresentationSink>(
        &mut self,
        sink: &mut S,
    ) -> Result<(), S::Error> {
        if self.focus_on_activate {
            sink.focus_child()?;
            self.focus_on_activate = false;
        }
        Ok(())
    }

    fn deactivate<S: NativePresentationSink>(&mut self, sink: &mut S) -> Result<(), S::Error> {
        if self.state != NativePresentationState::Open {
            return Ok(());
        }
        self.focus_on_activate = false;
        if !self.active && !self.visible {
            return Ok(());
        }

        sink.focus_parent()?;
        if self.visible {
            sink.hide()?;
        }
        self.active = false;
        self.visible = false;
        self.effective_visible = false;
        Ok(())
    }

    fn begin_reconnect<S: NativePresentationSink>(&mut self, sink: &mut S) -> Result<(), S::Error> {
        if self.state != NativePresentationState::Open {
            return Ok(());
        }

        self.focus_on_activate = false;
        let focus_result = if self.active || self.visible {
            sink.focus_parent()
        } else {
            Ok(())
        };
        let hide_result = if self.visible { sink.hide() } else { Ok(()) };

        // A reconnect invalidates the ActiveX drawing subtree. Keep the
        // cached bounds and the open lifecycle state, but close every
        // presentation gate before reporting a sink error so Reconnected
        // must perform a fresh SetBounds -> Show sequence.
        self.active = false;
        self.visible = false;
        self.effective_visible = false;
        self.login_complete = false;
        self.native_child_ready = false;

        focus_result.and(hide_result)
    }

    fn begin_close<S: NativePresentationSink>(&mut self, sink: &mut S) -> Result<bool, S::Error> {
        if self.state != NativePresentationState::Open {
            return Ok(false);
        }

        // Close callback admission before touching focus or visibility. Even
        // when either operation fails, late resize/focus/activate calls must
        // never reopen the native child.
        self.state = NativePresentationState::Closing;
        self.focus_on_activate = false;
        let focus_result = sink.focus_parent();
        let hide_result = if self.visible { sink.hide() } else { Ok(()) };
        self.active = false;
        self.visible = false;
        self.effective_visible = false;
        focus_result.and(hide_result)?;
        Ok(true)
    }

    fn finish_destroy(&mut self) {
        if self.state == NativePresentationState::Closing {
            self.state = NativePresentationState::Destroyed;
            self.focus_on_activate = false;
        }
    }

    /// Marks the session login/reconnect phase so the overlay may present.
    fn mark_login_complete(&mut self) {
        self.login_complete = true;
    }

    /// Updates the native child structural readiness snapshot.
    fn set_native_child_ready(&mut self, ready: bool) {
        self.native_child_ready = ready;
    }

    /// Updates the actual overlay visibility observed on the HWND.
    fn set_effective_visible(&mut self, visible: bool) {
        self.effective_visible = visible;
    }

    /// Whether the presentation may attempt to show: login phase reached,
    /// native child structurally ready, and non-zero bounds.
    fn can_present(&self) -> bool {
        self.state == NativePresentationState::Open
            && self.login_complete
            && self.native_child_ready
            && self
                .latest_bounds
                .is_some_and(|bounds| bounds.width > 0 && bounds.height > 0)
    }

    /// Whether the presentation is fully presented: can_present and the
    /// overlay is actually visible (not just requested).
    fn presentation_ready(&self) -> bool {
        self.can_present() && self.effective_visible
    }

    /// Whether activation work remains after a readiness transition or a
    /// transient native-focus failure.
    fn activation_pending(&self) -> bool {
        self.state == NativePresentationState::Open && (!self.active || self.focus_on_activate)
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeCloseProgress {
    Ready,
    WaitingForEvents { generation: u64 },
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeDestroyProgress {
    PendingCallbacks,
    Destroyed,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
pub(crate) struct WindowsNativeAdapter {
    presentation: WindowsNativePresentation,
    host: windows_rdp_host::WindowsRdpHost,
    overlay: WindowsNativeOverlay,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
#[derive(Debug)]
pub(crate) enum WindowsNativeAdapterCreateError {
    WindowHandle(raw_window_handle::HandleError),
    ParentHandleNotWin32,
    Overlay(WindowsNativeOverlayError),
    Host(windows_rdp_host::WindowsRdpHostError),
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
impl std::fmt::Display for WindowsNativeAdapterCreateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WindowHandle(error) => {
                write!(formatter, "failed to get GPUI window handle: {error}")
            }
            Self::ParentHandleNotWin32 => {
                formatter.write_str("GPUI window did not expose a Win32 parent handle")
            }
            Self::Overlay(error) => error.fmt(formatter),
            Self::Host(error) => error.fmt(formatter),
        }
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
impl std::error::Error for WindowsNativeAdapterCreateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            // `raw-window-handle` only implements `Error` for `HandleError`
            // when its optional `std` feature is enabled.
            Self::WindowHandle(_) => None,
            Self::ParentHandleNotWin32 => None,
            Self::Overlay(error) => Some(error),
            Self::Host(error) => Some(error),
        }
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
impl From<WindowsNativeOverlayError> for WindowsNativeAdapterCreateError {
    fn from(error: WindowsNativeOverlayError) -> Self {
        Self::Overlay(error)
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
impl WindowsNativeAdapter {
    /// Extracts the Win32 owner HWND from a GPUI window without performing any
    /// COM or window creation work. Safe to call while holding the App borrow.
    pub(crate) fn parent_window_owner(
        window: &gpui::Window,
    ) -> Result<usize, WindowsNativeAdapterCreateError> {
        use raw_window_handle::RawWindowHandle;

        let raw = raw_window_handle::HasWindowHandle::window_handle(window)
            .map_err(WindowsNativeAdapterCreateError::WindowHandle)?
            .as_raw();
        let RawWindowHandle::Win32(handle) = raw else {
            return Err(WindowsNativeAdapterCreateError::ParentHandleNotWin32);
        };
        Ok(handle.hwnd.get() as usize)
    }

    /// Creates the ActiveX host for an already-extracted owner HWND.
    ///
    /// This pumps Win32 messages (window creation, `AtlAxCreateControl`,
    /// `CoCreateInstance`), so it MUST NOT be called while holding the App
    /// borrow: the message pump can re-enter GPUI foreground tasks and
    /// re-borrow the App context.
    pub(crate) fn create_with_owner(
        owner: usize,
        generation: u64,
    ) -> Result<Self, WindowsNativeAdapterCreateError> {
        use windows_rdp_host::{WindowsRdpHost, WindowsRdpHostOptions, WindowsRdpParentWindow};

        let overlay = WindowsNativeOverlay::create(owner, generation)?;
        let parent = unsafe { WindowsRdpParentWindow::from_raw(overlay.hwnd()) };
        let host = unsafe {
            WindowsRdpHost::create_with_parent(parent, WindowsRdpHostOptions::new(generation))
        }
        .map_err(WindowsNativeAdapterCreateError::Host)?;

        Ok(Self {
            presentation: WindowsNativePresentation::default(),
            overlay,
            host,
        })
    }

    pub(crate) fn create(
        window: &gpui::Window,
        generation: u64,
    ) -> Result<Self, WindowsNativeAdapterCreateError> {
        let owner = Self::parent_window_owner(window)?;
        Self::create_with_owner(owner, generation)
    }

    pub(crate) fn generation(&self) -> u64 {
        self.host.generation()
    }

    pub(crate) fn is_open(&self) -> bool {
        self.presentation.state == NativePresentationState::Open
            && self.host.lifecycle() == windows_rdp_host::WindowsRdpHostLifecycle::Open
    }

    pub(crate) fn update_session_display_settings(
        &mut self,
        settings: windows_rdp_host::WindowsRdpSessionDisplaySettings,
    ) -> Result<(), windows_rdp_host::WindowsRdpHostError> {
        self.host.update_session_display_settings(settings)
    }

    /// Marks the LoginComplete / Reconnected phase. The native overlay remains
    /// hidden until this gate and the native child readiness gate both pass.
    pub(crate) fn mark_login_complete(&mut self) {
        self.presentation.mark_login_complete();
    }

    pub(crate) fn update_bounds(
        &mut self,
        bounds: Bounds<Pixels>,
        parent_client_origin: Point<Pixels>,
        scale_factor: f32,
    ) -> anyhow::Result<()> {
        let bounds = logical_bounds_to_physical(bounds, parent_client_origin, scale_factor)
            .ok_or_else(|| anyhow::anyhow!("invalid native child bounds or scale factor"))?;
        let mut sink = WindowsNativePresentationSink {
            overlay: &mut self.overlay,
            host: &mut self.host,
            focus_parent: None,
        };
        self.presentation.update_bounds(bounds, &mut sink)?;
        Ok(())
    }

    pub(crate) fn connect(
        &mut self,
        options: &windows_rdp_host::WindowsRdpConnectionOptions,
    ) -> Result<(), windows_rdp_host::WindowsRdpHostError> {
        self.host.connect(options)
    }

    pub(crate) fn apply_credentials(
        &mut self,
        credentials: &windows_rdp_host::WindowsRdpCredentialBundle,
    ) -> Result<(), windows_rdp_host::WindowsRdpHostError> {
        self.host.apply_credentials(credentials)
    }

    pub(crate) fn activate(&mut self, focus_child: bool) -> anyhow::Result<()> {
        let mut sink = WindowsNativePresentationSink {
            overlay: &mut self.overlay,
            host: &mut self.host,
            focus_parent: None,
        };
        self.presentation.activate(focus_child, &mut sink)?;
        Ok(())
    }

    pub(crate) fn focus(&mut self) -> anyhow::Result<()> {
        let mut sink = WindowsNativePresentationSink {
            overlay: &mut self.overlay,
            host: &mut self.host,
            focus_parent: None,
        };
        self.presentation.focus(&mut sink)?;
        Ok(())
    }

    pub(crate) fn deactivate(&mut self, focus_parent: &mut dyn FnMut()) -> anyhow::Result<()> {
        let mut sink = WindowsNativePresentationSink {
            overlay: &mut self.overlay,
            host: &mut self.host,
            focus_parent: Some(focus_parent),
        };
        self.presentation.deactivate(&mut sink)?;
        Ok(())
    }

    pub(crate) fn begin_reconnect(&mut self, focus_parent: &mut dyn FnMut()) -> anyhow::Result<()> {
        let mut sink = WindowsNativePresentationSink {
            overlay: &mut self.overlay,
            host: &mut self.host,
            focus_parent: Some(focus_parent),
        };
        self.presentation.begin_reconnect(&mut sink)?;
        Ok(())
    }

    /// Re-reads the native child structural readiness and the actual overlay
    /// visibility. Returns whether the presentation may now attempt to show.
    pub(crate) fn refresh_native_readiness(&mut self) -> bool {
        let native_ready = match self.host.presentation_state() {
            Ok(state) => state.child_ready(),
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "failed to read Windows native RDP presentation state"
                );
                false
            }
        };
        self.presentation.set_native_child_ready(native_ready);
        self.presentation
            .set_effective_visible(self.overlay.is_actually_visible());
        self.presentation.can_present()
    }

    /// Whether the presentation may attempt to show (login phase reached,
    /// native child structurally ready, non-zero bounds).
    pub(crate) fn can_present(&self) -> bool {
        self.presentation.can_present()
    }

    /// Whether the presentation is fully presented (requested + effective).
    pub(crate) fn presentation_ready(&self) -> bool {
        self.presentation.presentation_ready()
    }

    /// Whether a show has been requested (and succeeded) since the last hide.
    pub(crate) fn requested_visible(&self) -> bool {
        self.presentation.visible
    }

    pub(crate) fn activation_pending(&self) -> bool {
        self.presentation.activation_pending()
    }

    pub(crate) fn begin_close(
        &mut self,
        focus_parent: &mut dyn FnMut(),
    ) -> anyhow::Result<NativeCloseProgress> {
        let mut sink = WindowsNativePresentationSink {
            overlay: &mut self.overlay,
            host: &mut self.host,
            focus_parent: Some(focus_parent),
        };
        if let Err(error) = self.presentation.begin_close(&mut sink) {
            tracing::warn!(
                ?error,
                "failed to hide Windows native RDP presentation while closing"
            );
        }

        use windows_rdp_host::{WindowsRdpHostLifecycle, WindowsRdpRequestCloseStatus};
        match self.host.lifecycle() {
            WindowsRdpHostLifecycle::Closed | WindowsRdpHostLifecycle::Closing => {
                Ok(NativeCloseProgress::Ready)
            }
            WindowsRdpHostLifecycle::Open => match self.host.request_close()? {
                WindowsRdpRequestCloseStatus::CanProceed => Ok(NativeCloseProgress::Ready),
                WindowsRdpRequestCloseStatus::WaitForEvents => {
                    Ok(NativeCloseProgress::WaitingForEvents {
                        generation: self.host.generation(),
                    })
                }
            },
        }
    }

    pub(super) fn close_confirmed(
        &self,
        state: &mut super::native_events::NativeRdpEventState,
    ) -> bool {
        self.drain_events(state);
        state.close_confirmed()
    }

    pub(super) fn drain_events(
        &self,
        state: &mut super::native_events::NativeRdpEventState,
    ) -> Vec<super::native_events::NativeRdpUiEffect> {
        super::native_events::drain_native_events(&self.host, state)
    }

    pub(crate) fn finish_destroy(&mut self) -> anyhow::Result<NativeDestroyProgress> {
        use windows_rdp_host::WindowsRdpHostError;

        match self.host.close() {
            Ok(()) => {
                self.overlay.close()?;
                self.presentation.finish_destroy();
                Ok(NativeDestroyProgress::Destroyed)
            }
            Err(WindowsRdpHostError::CallbackInFlight) => {
                Ok(NativeDestroyProgress::PendingCallbacks)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn force_close(
        &mut self,
        focus_parent: &mut dyn FnMut(),
    ) -> anyhow::Result<NativeDestroyProgress> {
        let mut sink = WindowsNativePresentationSink {
            overlay: &mut self.overlay,
            host: &mut self.host,
            focus_parent: Some(focus_parent),
        };
        if let Err(error) = self.presentation.begin_close(&mut sink) {
            tracing::warn!(
                ?error,
                "failed to hide Windows native RDP presentation while force-closing"
            );
        }

        if self.host.lifecycle() == windows_rdp_host::WindowsRdpHostLifecycle::Open
            && let Err(error) = self.host.disconnect()
        {
            tracing::warn!(
                ?error,
                "failed to disconnect Windows native RDP before destroy"
            );
        }
        self.finish_destroy()
    }

    pub(crate) fn is_destroyed(&self) -> bool {
        self.host.is_closed()
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
impl Drop for WindowsNativeAdapter {
    fn drop(&mut self) {
        use windows_rdp_host::WindowsRdpHostError;

        match self.host.close() {
            Ok(()) => {
                if let Err(error) = self.overlay.close() {
                    tracing::error!(
                        ?error,
                        "failed to destroy Windows native RDP overlay after host drop"
                    );
                }
            }
            Err(WindowsRdpHostError::CallbackInFlight) => {
                self.overlay
                    .abandon("host_callback_in_flight_during_adapter_drop");
            }
            Err(error) => {
                tracing::error!(
                    ?error,
                    "failed to destroy Windows native RDP host during adapter drop"
                );
                self.overlay
                    .abandon("host_destroy_failed_during_adapter_drop");
            }
        }
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
struct WindowsNativePresentationSink<'a> {
    overlay: &'a mut WindowsNativeOverlay,
    host: &'a mut windows_rdp_host::WindowsRdpHost,
    focus_parent: Option<&'a mut dyn FnMut()>,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
impl NativePresentationSink for WindowsNativePresentationSink<'_> {
    type Error = anyhow::Error;

    fn set_bounds(&mut self, bounds: Win32ClientPhysicalBounds) -> Result<(), Self::Error> {
        let clipped = self.overlay.set_bounds(WindowsNativeOverlayBounds {
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
        })?;
        let Some(clipped) = clipped else {
            self.host.set_bounds(0, 0, 0, 0)?;
            return Ok(());
        };
        self.host.set_bounds(0, 0, clipped.width, clipped.height)?;
        Ok(())
    }

    fn show(&mut self) -> Result<(), Self::Error> {
        self.overlay.show()?;
        if let Err(error) = self.host.set_visible(true) {
            if let Err(hide_error) = self.overlay.hide() {
                tracing::warn!(
                    ?hide_error,
                    "failed to hide Windows native RDP overlay after host show failed"
                );
            }
            return Err(error.into());
        }
        if !self.overlay.is_actually_visible() {
            // The request succeeded but the owner cannot present right now
            // (hidden/minimized) or the bounds were clipped away; never report
            // a silent show.
            return Err(anyhow::anyhow!(
                "Windows native RDP overlay show did not become effective"
            ));
        }
        self.overlay.log_composition_diagnostics("show_complete");
        Ok(())
    }

    fn focus_child(&mut self) -> Result<(), Self::Error> {
        self.host.focus()?;
        Ok(())
    }

    fn focus_parent(&mut self) -> Result<(), Self::Error> {
        if let Some(focus_parent) = self.focus_parent.as_mut() {
            focus_parent();
        }
        Ok(())
    }

    fn hide(&mut self) -> Result<(), Self::Error> {
        let host_result = self.host.set_visible(false);
        let overlay_result = self.overlay.hide();
        match (host_result, overlay_result) {
            (Err(host_error), Err(overlay_error)) => {
                tracing::warn!(
                    ?overlay_error,
                    "failed to hide Windows native RDP overlay after host hide failed"
                );
                Err(host_error.into())
            }
            (Err(error), Ok(())) => Err(error.into()),
            (Ok(()), Err(error)) => Err(error.into()),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn is_effectively_visible(&self) -> bool {
        self.overlay.is_actually_visible()
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use gpui::{Bounds, point, px, size};

    use super::{
        NativePresentationSink, NativePresentationState, Win32ClientPhysicalBounds,
        WindowsNativePresentation, logical_bounds_to_physical,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Command {
        SetBounds(Win32ClientPhysicalBounds),
        Show,
        FocusChild,
        FocusParent,
        Hide,
    }

    #[derive(Default)]
    struct Recorder {
        commands: Vec<Command>,
    }

    impl NativePresentationSink for Recorder {
        type Error = Infallible;

        fn set_bounds(&mut self, bounds: Win32ClientPhysicalBounds) -> Result<(), Self::Error> {
            self.commands.push(Command::SetBounds(bounds));
            Ok(())
        }

        fn show(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::Show);
            Ok(())
        }

        fn focus_child(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::FocusChild);
            Ok(())
        }

        fn focus_parent(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::FocusParent);
            Ok(())
        }

        fn hide(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::Hide);
            Ok(())
        }

        fn is_effectively_visible(&self) -> bool {
            matches!(self.commands.last(), Some(Command::Show))
        }
    }

    fn bounds() -> Win32ClientPhysicalBounds {
        Win32ClientPhysicalBounds {
            x: 20,
            y: 40,
            width: 1600,
            height: 900,
        }
    }

    /// A presentation that passed the login phase and native child readiness
    /// gate, so activate() may actually show the overlay.
    fn ready_presentation() -> WindowsNativePresentation {
        let mut presentation = WindowsNativePresentation::default();
        presentation.mark_login_complete();
        presentation.set_native_child_ready(true);
        presentation
    }

    #[test]
    fn activate_before_login_complete_defers_the_show() {
        let mut presentation = WindowsNativePresentation::default();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(true, &mut recorder).unwrap();

        assert!(recorder.commands.is_empty());
        assert!(!presentation.can_present());
        assert!(!presentation.presentation_ready());

        // The LoginComplete/Reconnected transition unlocks activation.
        presentation.mark_login_complete();
        presentation.set_native_child_ready(true);
        presentation.activate(true, &mut recorder).unwrap();
        assert_eq!(
            vec![
                Command::SetBounds(bounds()),
                Command::Show,
                Command::FocusChild,
            ],
            recorder.commands
        );
        assert!(presentation.can_present());
        assert!(presentation.presentation_ready());
    }

    #[test]
    fn activate_requires_the_native_child_to_be_structurally_ready() {
        let mut presentation = WindowsNativePresentation::default();
        presentation.mark_login_complete();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(false, &mut recorder).unwrap();
        assert!(recorder.commands.is_empty());

        presentation.set_native_child_ready(true);
        presentation.activate(false, &mut recorder).unwrap();
        assert_eq!(
            vec![Command::SetBounds(bounds()), Command::Show],
            recorder.commands
        );
    }

    #[test]
    fn zero_sized_bounds_never_become_presentable() {
        let mut presentation = ready_presentation();
        let mut recorder = Recorder::default();
        let zero = Win32ClientPhysicalBounds {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };

        presentation.update_bounds(zero, &mut recorder).unwrap();
        presentation.activate(false, &mut recorder).unwrap();
        assert!(recorder.commands.is_empty());
        assert!(!presentation.presentation_ready());
    }

    #[test]
    fn activate_applies_bounds_then_shows_and_focuses() {
        let mut presentation = ready_presentation();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        assert!(recorder.commands.is_empty());

        presentation.activate(true, &mut recorder).unwrap();
        assert_eq!(
            vec![
                Command::SetBounds(bounds()),
                Command::Show,
                Command::FocusChild,
            ],
            recorder.commands
        );
    }

    #[test]
    fn deactivate_focuses_parent_before_hiding() {
        let mut presentation = ready_presentation();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(false, &mut recorder).unwrap();
        recorder.commands.clear();
        presentation.deactivate(&mut recorder).unwrap();

        assert_eq!(vec![Command::FocusParent, Command::Hide], recorder.commands);
        assert!(!presentation.presentation_ready());
    }

    #[test]
    fn begin_reconnect_focuses_parent_hides_and_resets_presentation_gates() {
        let mut presentation = ready_presentation();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(false, &mut recorder).unwrap();
        presentation.set_effective_visible(true);
        recorder.commands.clear();

        presentation.begin_reconnect(&mut recorder).unwrap();

        assert_eq!(vec![Command::FocusParent, Command::Hide], recorder.commands);
        assert_eq!(NativePresentationState::Open, presentation.state);
        assert_eq!(Some(bounds()), presentation.latest_bounds);
        assert!(!presentation.active);
        assert!(!presentation.visible);
        assert!(!presentation.effective_visible);
        assert!(!presentation.login_complete);
        assert!(!presentation.native_child_ready);
        assert!(!presentation.can_present());
    }

    #[test]
    fn reconnected_presentation_reapplies_bounds_and_shows_again() {
        let mut presentation = ready_presentation();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(false, &mut recorder).unwrap();
        presentation.begin_reconnect(&mut recorder).unwrap();
        recorder.commands.clear();

        presentation.mark_login_complete();
        presentation.set_native_child_ready(true);
        presentation.activate(false, &mut recorder).unwrap();

        assert_eq!(
            vec![Command::SetBounds(bounds()), Command::Show],
            recorder.commands
        );
        assert!(presentation.active);
        assert!(presentation.visible);
    }

    #[test]
    fn focus_requested_before_reconnected_child_is_ready_is_delivered_on_deferred_activation() {
        let mut presentation = WindowsNativePresentation::default();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.mark_login_complete();

        // Reconnected first tries to synchronize the presentation, but the
        // replacement ActiveX drawing child may not exist yet.
        presentation.activate(false, &mut recorder).unwrap();

        // The UI event then requests native focus. This request must survive
        // until the readiness maintenance tick can actually present.
        presentation.focus(&mut recorder).unwrap();
        assert!(recorder.commands.is_empty());

        presentation.set_native_child_ready(true);
        presentation.activate(false, &mut recorder).unwrap();

        assert_eq!(
            vec![
                Command::SetBounds(bounds()),
                Command::Show,
                Command::FocusChild,
            ],
            recorder.commands
        );
    }

    #[test]
    fn deactivation_cancels_focus_waiting_for_native_child_readiness() {
        let mut presentation = WindowsNativePresentation::default();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.mark_login_complete();
        presentation.focus(&mut recorder).unwrap();

        // The tab became inactive before the replacement child was ready.
        presentation.deactivate(&mut recorder).unwrap();
        presentation.set_native_child_ready(true);
        presentation.activate(false, &mut recorder).unwrap();

        assert_eq!(
            vec![Command::SetBounds(bounds()), Command::Show],
            recorder.commands
        );
    }

    #[test]
    fn transient_focus_failure_keeps_activation_committed_and_retries_without_reshowing() {
        let mut presentation = ready_presentation();
        let mut recorder = FailingFocusOnceRecorder {
            fail_next_focus: true,
            ..FailingFocusOnceRecorder::default()
        };

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.focus(&mut recorder).unwrap();

        assert_eq!(
            Err(PresentationError),
            presentation.activate(false, &mut recorder)
        );
        assert!(presentation.active);
        assert!(presentation.visible);
        assert!(presentation.activation_pending());
        assert_eq!(
            vec![
                Command::SetBounds(bounds()),
                Command::Show,
                Command::FocusChild,
            ],
            recorder.commands
        );

        presentation.activate(false, &mut recorder).unwrap();

        assert!(!presentation.activation_pending());
        assert_eq!(
            vec![
                Command::SetBounds(bounds()),
                Command::Show,
                Command::FocusChild,
                Command::FocusChild,
            ],
            recorder.commands
        );
    }

    #[test]
    fn transient_focus_failure_on_active_presentation_is_retried_by_activation_tick() {
        let mut presentation = ready_presentation();
        let mut recorder = FailingFocusOnceRecorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(false, &mut recorder).unwrap();
        recorder.commands.clear();
        recorder.fail_next_focus = true;

        assert_eq!(Err(PresentationError), presentation.focus(&mut recorder));
        assert!(presentation.active);
        assert!(presentation.visible);
        assert!(presentation.activation_pending());

        presentation.activate(false, &mut recorder).unwrap();

        assert!(!presentation.activation_pending());
        assert_eq!(
            vec![Command::FocusChild, Command::FocusChild],
            recorder.commands
        );
    }

    #[test]
    fn activate_and_deactivate_are_idempotent() {
        let mut presentation = ready_presentation();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(true, &mut recorder).unwrap();
        presentation.activate(true, &mut recorder).unwrap();
        assert_eq!(3, recorder.commands.len());

        presentation.deactivate(&mut recorder).unwrap();
        presentation.deactivate(&mut recorder).unwrap();
        assert_eq!(5, recorder.commands.len());
    }

    #[test]
    fn inactive_resize_only_updates_the_cached_bounds() {
        let mut presentation = ready_presentation();
        let mut recorder = Recorder::default();
        let latest = Win32ClientPhysicalBounds {
            x: 30,
            y: 50,
            width: 1280,
            height: 720,
        };

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.update_bounds(latest, &mut recorder).unwrap();
        assert!(recorder.commands.is_empty());

        presentation.activate(false, &mut recorder).unwrap();
        assert_eq!(
            vec![Command::SetBounds(latest), Command::Show],
            recorder.commands
        );
    }

    #[test]
    fn active_resize_updates_bounds_without_changing_visibility() {
        let mut presentation = ready_presentation();
        let mut recorder = Recorder::default();
        let latest = Win32ClientPhysicalBounds {
            x: 30,
            y: 50,
            width: 1280,
            height: 720,
        };

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(false, &mut recorder).unwrap();
        recorder.commands.clear();
        presentation.update_bounds(latest, &mut recorder).unwrap();

        assert_eq!(vec![Command::SetBounds(latest)], recorder.commands);
    }

    #[test]
    fn begin_close_focuses_parent_then_hides_and_is_idempotent() {
        let mut presentation = ready_presentation();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(false, &mut recorder).unwrap();
        recorder.commands.clear();

        assert!(presentation.begin_close(&mut recorder).unwrap());
        assert!(!presentation.begin_close(&mut recorder).unwrap());
        assert_eq!(vec![Command::FocusParent, Command::Hide], recorder.commands);
        assert_eq!(NativePresentationState::Closing, presentation.state);
        assert!(!presentation.presentation_ready());
    }

    #[test]
    fn closing_gate_ignores_late_resize_activate_focus_and_deactivate() {
        let mut presentation = ready_presentation();
        let mut recorder = Recorder::default();

        presentation.update_bounds(bounds(), &mut recorder).unwrap();
        presentation.activate(true, &mut recorder).unwrap();
        presentation.begin_close(&mut recorder).unwrap();
        recorder.commands.clear();

        let late = Win32ClientPhysicalBounds {
            x: 30,
            y: 50,
            width: 1280,
            height: 720,
        };
        presentation.update_bounds(late, &mut recorder).unwrap();
        presentation.activate(true, &mut recorder).unwrap();
        presentation.focus(&mut recorder).unwrap();
        presentation.deactivate(&mut recorder).unwrap();

        assert!(recorder.commands.is_empty());
        assert_eq!(Some(bounds()), presentation.latest_bounds);
    }

    #[test]
    fn finish_destroy_is_idempotent() {
        let mut presentation = WindowsNativePresentation::default();
        let mut recorder = Recorder::default();

        presentation.begin_close(&mut recorder).unwrap();
        presentation.finish_destroy();
        presentation.finish_destroy();

        assert_eq!(NativePresentationState::Destroyed, presentation.state);
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct PresentationError;

    #[derive(Default)]
    struct FailingFocusOnceRecorder {
        commands: Vec<Command>,
        fail_next_focus: bool,
    }

    impl NativePresentationSink for FailingFocusOnceRecorder {
        type Error = PresentationError;

        fn set_bounds(&mut self, bounds: Win32ClientPhysicalBounds) -> Result<(), Self::Error> {
            self.commands.push(Command::SetBounds(bounds));
            Ok(())
        }

        fn show(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::Show);
            Ok(())
        }

        fn focus_child(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::FocusChild);
            if self.fail_next_focus {
                self.fail_next_focus = false;
                return Err(PresentationError);
            }
            Ok(())
        }

        fn focus_parent(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::FocusParent);
            Ok(())
        }

        fn hide(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::Hide);
            Ok(())
        }

        fn is_effectively_visible(&self) -> bool {
            self.commands.contains(&Command::Show)
                && !matches!(self.commands.last(), Some(Command::Hide))
        }
    }

    #[derive(Default)]
    struct FailingCloseRecorder {
        commands: Vec<Command>,
    }

    impl NativePresentationSink for FailingCloseRecorder {
        type Error = PresentationError;

        fn set_bounds(&mut self, bounds: Win32ClientPhysicalBounds) -> Result<(), Self::Error> {
            self.commands.push(Command::SetBounds(bounds));
            Ok(())
        }

        fn show(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::Show);
            Ok(())
        }

        fn focus_child(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::FocusChild);
            Ok(())
        }

        fn focus_parent(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::FocusParent);
            Err(PresentationError)
        }

        fn hide(&mut self) -> Result<(), Self::Error> {
            self.commands.push(Command::Hide);
            Err(PresentationError)
        }

        fn is_effectively_visible(&self) -> bool {
            matches!(self.commands.last(), Some(Command::Show))
        }
    }

    #[test]
    fn failed_begin_close_keeps_the_admission_gate_closed() {
        let mut presentation = WindowsNativePresentation {
            active: true,
            visible: true,
            ..WindowsNativePresentation::default()
        };
        let mut recorder = FailingCloseRecorder::default();

        assert_eq!(
            Err(PresentationError),
            presentation.begin_close(&mut recorder)
        );
        assert_eq!(vec![Command::FocusParent, Command::Hide], recorder.commands);
        assert_eq!(NativePresentationState::Closing, presentation.state);
        assert!(!presentation.active);
        assert!(!presentation.visible);
    }

    #[test]
    fn failed_begin_reconnect_still_resets_all_presentation_gates() {
        let mut presentation = WindowsNativePresentation {
            active: true,
            visible: true,
            effective_visible: true,
            login_complete: true,
            native_child_ready: true,
            latest_bounds: Some(bounds()),
            ..WindowsNativePresentation::default()
        };
        let mut recorder = FailingCloseRecorder::default();

        assert_eq!(
            Err(PresentationError),
            presentation.begin_reconnect(&mut recorder)
        );
        assert_eq!(vec![Command::FocusParent, Command::Hide], recorder.commands);
        assert_eq!(NativePresentationState::Open, presentation.state);
        assert_eq!(Some(bounds()), presentation.latest_bounds);
        assert!(!presentation.active);
        assert!(!presentation.visible);
        assert!(!presentation.effective_visible);
        assert!(!presentation.login_complete);
        assert!(!presentation.native_child_ready);
    }

    #[test]
    fn converts_logical_bounds_at_supported_dpi_scales() {
        let logical = Bounds::new(point(px(10.0), px(20.0)), size(px(800.0), px(600.0)));
        let parent_origin = point(px(2.0), px(4.0));

        for (scale_factor, expected) in [
            (
                1.0,
                Win32ClientPhysicalBounds {
                    x: 8,
                    y: 16,
                    width: 800,
                    height: 600,
                },
            ),
            (
                1.25,
                Win32ClientPhysicalBounds {
                    x: 10,
                    y: 20,
                    width: 1000,
                    height: 750,
                },
            ),
            (
                1.5,
                Win32ClientPhysicalBounds {
                    x: 12,
                    y: 24,
                    width: 1200,
                    height: 900,
                },
            ),
            (
                2.0,
                Win32ClientPhysicalBounds {
                    x: 16,
                    y: 32,
                    width: 1600,
                    height: 1200,
                },
            ),
        ] {
            assert_eq!(
                Some(expected),
                logical_bounds_to_physical(logical, parent_origin, scale_factor)
            );
        }
    }

    #[test]
    fn conversion_preserves_negative_origins_and_zero_size() {
        let logical = Bounds::new(point(px(1.0), px(2.0)), size(px(0.0), px(0.0)));
        let parent_origin = point(px(3.0), px(5.0));

        assert_eq!(
            Some(Win32ClientPhysicalBounds {
                x: -3,
                y: -5,
                width: 0,
                height: 0,
            }),
            logical_bounds_to_physical(logical, parent_origin, 1.5)
        );
    }

    #[test]
    fn conversion_rejects_invalid_scale_factors() {
        let logical = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(600.0)));
        let origin = point(px(0.0), px(0.0));

        assert_eq!(None, logical_bounds_to_physical(logical, origin, 0.0));
        assert_eq!(None, logical_bounds_to_physical(logical, origin, f32::NAN));
    }
}
