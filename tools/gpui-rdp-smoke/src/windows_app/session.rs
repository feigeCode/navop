use gpui::{Window, WindowCompositionSurface};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_rdp_host::{
    WindowsRdpColorDepth, WindowsRdpConnectionOptions, WindowsRdpCredentialBundle, WindowsRdpHost,
    WindowsRdpHostError, WindowsRdpHostLifecycle, WindowsRdpParentWindow,
};

use super::composition::WindowsNativeComposition;
use super::{host_options, log_host_error, physical_viewport_size};
use crate::{cli::Config, native_overlay::NativeOverlay};

/// When the overlay is moved into the GPUI composition tree.
///
/// The distinction matters for one specific question: whether
/// `CreateSurfaceFromHwnd` captures pixels from a window that has never been on
/// screen. `Early` composes before the window is ever shown (what Navop does);
/// `Late` composes after login has completed, so the window has been visible and
/// its session has real content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComposeMode {
    Off,
    Early,
    Late,
}

pub(crate) fn compose_mode() -> ComposeMode {
    match std::env::var("SMOKE_RDP_COMPOSE").ok().as_deref() {
        Some("early") => ComposeMode::Early,
        Some("late") => ComposeMode::Late,
        _ => ComposeMode::Off,
    }
}

/// Whether to cloak the overlay at attach time instead of after it has been
/// shown and painted.
///
/// Only used to demonstrate causality: the immediate ordering is the one that
/// leaves the composition surface empty, and keeping it behind a switch lets
/// the fixed and broken orderings be measured on the same build.
fn cloak_immediately() -> bool {
    std::env::var("SMOKE_RDP_CLOAK_IMMEDIATE").is_ok_and(|value| value == "1" || value == "true")
}

pub(super) struct NativeSession {
    pub(super) host: WindowsRdpHost,
    pub(super) overlay: NativeOverlay,
    composition: Option<WindowsNativeComposition>,
    /// Set once the overlay is attached to the composition tree but DWM has not
    /// been told to stop drawing the child window yet.
    ///
    /// The cloak cannot be applied at attach time: see `finish_cloak`.
    pending_cloak: bool,
    /// Kept until composition actually happens so the `Late` mode can attach
    /// after login instead of at creation time.
    surface: Option<WindowCompositionSurface>,
}

pub(super) type Initialization = (Option<NativeSession>, String, Option<(i32, i32)>);

impl NativeSession {
    pub(super) fn prepare_host_close(&mut self) {
        self.release_composition();
        if self.host.lifecycle() != WindowsRdpHostLifecycle::Open {
            return;
        }
        if let Err(error) = self.host.set_visible(false) {
            log_host_error("close_set_visible", error);
        }
        if let Err(error) = self.host.disconnect() {
            log_host_error("close_disconnect", error);
        }
    }

    /// Moves the overlay window into the composition tree and takes it off
    /// screen.
    ///
    /// Ordering is deliberate: `WS_EX_LAYERED` (with its alpha) first, because
    /// that is what `CreateSurfaceFromHwnd` wraps; the attachment next, because
    /// cloaking an uncomposed window would take the session off screen with
    /// nothing to replace it. The cloak itself is *not* applied here — see
    /// `finish_cloak`.
    pub(super) fn compose(&mut self, stage: &str) -> Result<(), String> {
        if self.composition.is_some() {
            return Ok(());
        }
        let Some(surface) = self.surface.clone() else {
            return Ok(());
        };
        if let Err(error) = self.overlay.set_layered(true) {
            return Err(format!("{stage}: applying WS_EX_LAYERED failed: {error}"));
        }
        let mut composition = WindowsNativeComposition::new(surface, self.overlay.hwnd());
        if let Err(error) = composition.attach() {
            // A layered window that nothing composes is invisible rather than
            // merely uncomposed, so the style has to come back off.
            if let Err(restore) = self.overlay.set_layered(false) {
                println!("composition: warning could not drop WS_EX_LAYERED after failure: {restore}");
            }
            return Err(format!("{stage}: {error}"));
        }
        self.composition = Some(composition);
        println!(
            "composition: stage={stage} overlay_hwnd=0x{:016X} layered_applied=true cloak={}",
            self.overlay.hwnd(),
            if cloak_immediately() { "immediate" } else { "deferred" }
        );
        if cloak_immediately() {
            // Negative control for the ordering above: cloaking right here is the
            // behaviour that produced an empty composition surface.
            self.finish_cloak("compose_immediate")?;
        } else {
            self.pending_cloak = true;
        }
        Ok(())
    }

    /// Cloaks the overlay, once doing so is actually safe.
    ///
    /// This is the whole reason the cloak is not applied inside `compose`.
    /// `DWMWA_CLOAK` is what takes the child window off screen so that only the
    /// composition visual shows the session, but it is destructive when applied
    /// too early: cloaking a window that has never been shown and painted at its
    /// final size leaves the composition surface permanently empty. The window
    /// then contributes no pixels at all — the composed area shows whatever is
    /// behind it — and no later paint brings it back, not even the RDP frames
    /// themselves. Every DirectComposition call still reports success, which is
    /// why this fails silently.
    ///
    /// The window has to be on screen at its final size and painted once before
    /// the cloak lands. Both are true by the time the first bounds sync has been
    /// mirrored into the composition tree, so that is when this runs. Tests that
    /// cloaked at 1x1 and resized afterwards always produced an empty surface;
    /// painting first and cloaking afterwards always worked, and the result was
    /// not timing dependent.
    ///
    /// Callers own the decision of *when* this is safe; the method itself always
    /// cloaks.
    fn finish_cloak(&mut self, stage: &str) -> Result<(), String> {
        if let Err(error) = self.overlay.redraw() {
            return Err(format!("{stage}: painting the overlay at its final size failed: {error}"));
        }
        if let Err(error) = self.overlay.set_cloaked(true) {
            return Err(format!("{stage}: cloaking the composed overlay failed: {error}"));
        }
        self.pending_cloak = false;
        let cloaked = self
            .overlay
            .cloaked_state()
            .map_or_else(|error| format!("unreadable({error})"), |value| value.to_string());
        println!(
            "composition: stage={stage} overlay_hwnd=0x{:016X} layered_applied=true cloaked_state={cloaked}",
            self.overlay.hwnd()
        );
        Ok(())
    }

    /// Mirrors the overlay's placement into the composition tree.
    pub(super) fn sync_composition(&mut self, visible: bool) -> Result<(), String> {
        let bounds = self.overlay.last_bounds();
        match self.composition.as_mut() {
            Some(composition) if visible => composition.sync_bounds(bounds)?,
            Some(composition) => return composition.sync_visible(false),
            None => return Ok(()),
        }
        // The child window is on screen at its final size and the visual now has
        // the matching bounds, so the deferred cloak can finally be applied.
        if self.pending_cloak {
            self.finish_cloak("composition_sync")?;
        }
        Ok(())
    }

    pub(super) fn composition_active(&self) -> bool {
        self.composition.is_some()
    }

    /// Restores the plain child-window presentation.
    fn release_composition(&mut self) {
        self.composition = None;
        self.pending_cloak = false;
        if let Err(error) = self.overlay.set_cloaked(false) {
            println!("composition: warning could not uncloak while falling back: {error}");
        }
        if let Err(error) = self.overlay.set_layered(false) {
            println!("composition: warning could not drop WS_EX_LAYERED while falling back: {error}");
        }
    }
}

/// Enables GPUI window composition and creates the surface the native RDP
/// window is presented in.
///
/// Every failure here is non-fatal: the caller keeps the plain child-window
/// presentation.
fn enable_composition_surface(window: &Window) -> Option<WindowCompositionSurface> {
    match window
        .enable_window_composition()
        .and_then(|composition| composition.create_native_surface())
    {
        Ok(surface) => Some(surface),
        Err(error) => {
            eprintln!("composition: stage=enable_composition error={error:#}");
            None
        }
    }
}

pub(super) fn initialize(config: Config, window: &Window) -> Initialization {
    log_config(&config);
    let mode = compose_mode();
    println!("composition: mode={mode:?} compose_env={:?}", std::env::var("SMOKE_RDP_COMPOSE").ok());
    let credentials = build_credentials(&config);
    let connection_options = match build_connection_options(&config) {
        Ok(options) => options,
        Err(error) => return initialization_error("connection_options", error),
    };
    if let Err(error) = probe_host() {
        return (None, error, None);
    }
    let owner = match gpui_owner(window) {
        Ok(owner) => owner,
        Err(error) => return (None, error, None),
    };
    let surface = match mode {
        ComposeMode::Off => None,
        _ => enable_composition_surface(window),
    };
    if surface.is_none() && mode != ComposeMode::Off {
        eprintln!("composition: stage=surface_unavailable; falling back to a plain child window");
    }
    let session = match create_session(owner, surface, mode) {
        Ok(session) => session,
        Err(error) => return (None, error, None),
    };
    finish_initialization(session, credentials, connection_options, window)
}

fn log_config(config: &Config) {
    println!(
        "config: host_present={} port={} username_present={} domain_present={} password_env_present={} desktop={}x{} timeout_seconds={}",
        !config.host.is_empty(),
        config.port,
        config.username.is_some(),
        config.domain.is_some(),
        config.password.is_some(),
        config.width,
        config.height,
        config.timeout_seconds
    );
}

fn build_credentials(config: &Config) -> WindowsRdpCredentialBundle {
    let mut credentials = WindowsRdpCredentialBundle::new();
    if let Some(username) = config.username.clone() {
        credentials.set_username(username);
    }
    if let Some(domain) = config.domain.clone() {
        credentials.set_domain(domain);
    }
    if let Some(password) = config.password.clone() {
        credentials.set_server_password(password);
    }
    credentials
}

fn build_connection_options(
    config: &Config,
) -> Result<WindowsRdpConnectionOptions, WindowsRdpHostError> {
    WindowsRdpConnectionOptions::new(
        config.host.clone(),
        config.port,
        config.width,
        config.height,
        WindowsRdpColorDepth::Bpp32,
    )
}

fn probe_host() -> Result<(), String> {
    match WindowsRdpHost::probe() {
        Ok(capabilities) if capabilities.is_available() => {
            println!("probe: available=true capabilities={capabilities:?}");
            Ok(())
        }
        Ok(capabilities) => {
            eprintln!("ERROR: stage=probe error=native_boundary_unavailable");
            eprintln!("ERROR_DEBUG: stage=probe capabilities={capabilities:?}");
            Err("Windows native RDP boundary is unavailable; see console".to_owned())
        }
        Err(error) => {
            log_host_error("probe", error);
            Err("Windows native RDP probe failed; see console".to_owned())
        }
    }
}

fn gpui_owner(window: &Window) -> Result<usize, String> {
    let raw = HasWindowHandle::window_handle(window)
        .map_err(|error| {
            eprintln!("ERROR: stage=get_gpui_window_handle error={error}");
            "Failed to get the GPUI native window handle; see console".to_owned()
        })?
        .as_raw();
    let RawWindowHandle::Win32(handle) = raw else {
        eprintln!("ERROR: stage=get_gpui_window_handle error=handle_is_not_win32");
        return Err("GPUI did not expose a Win32 HWND; see console".to_owned());
    };
    let owner = handle.hwnd.get() as usize;
    println!("create: gpui_owner_hwnd=0x{owner:016X}");
    Ok(owner)
}

fn create_session(
    owner: usize,
    surface: Option<WindowCompositionSurface>,
    mode: ComposeMode,
) -> Result<NativeSession, String> {
    let overlay = NativeOverlay::create(owner).map_err(|error| {
        eprintln!("ERROR: stage=create_native_overlay error={error}");
        "Failed to create the child native RDP overlay; see console".to_owned()
    })?;
    println!("create: rdp_parent_hwnd=0x{:016X}", overlay.hwnd());
    let parent = unsafe { WindowsRdpParentWindow::from_raw(overlay.hwnd()) };
    let host =
        unsafe { WindowsRdpHost::create_with_parent(parent, host_options()) }.map_err(|error| {
            log_host_error("create_with_parent", error);
            "Windows native RDP host creation failed; see console".to_owned()
        })?;
    println!(
        "create: success generation={} lifecycle={:?}",
        host.generation(),
        host.lifecycle()
    );
    let mut session = NativeSession {
        host,
        overlay,
        composition: None,
        pending_cloak: false,
        surface,
    };
    if mode == ComposeMode::Early {
        if let Err(error) = session.compose("early") {
            eprintln!("ERROR: stage=compose_early error={error}");
        }
    }
    Ok(session)
}

fn finish_initialization(
    mut session: NativeSession,
    credentials: WindowsRdpCredentialBundle,
    connection_options: WindowsRdpConnectionOptions,
    window: &Window,
) -> Initialization {
    let bounds = physical_viewport_size(window);
    println!(
        "bounds: physical_width={} physical_height={} scale_factor={}",
        bounds.0,
        bounds.1,
        window.scale_factor()
    );
    if let Err(error) = session.overlay.synchronize((0, 0, bounds.0, bounds.1)) {
        return failed_after_overlay_error(session, "initial_overlay_bounds", error);
    }
    if session.composition_active() {
        if let Err(error) = session.sync_composition(true) {
            eprintln!("ERROR: stage=initial_composition_sync error={error}");
        }
    }
    if let Err(error) = configure_presentation(&mut session, bounds) {
        return failed_after_host_error(session, error.0, error.1);
    }
    if let Err(error) = connect_session(&mut session, &credentials, &connection_options) {
        return failed_after_host_error(session, error.0, error.1);
    }
    (
        Some(session),
        "RDP connect requested; waiting for native events".to_owned(),
        Some(bounds),
    )
}

fn configure_presentation(
    session: &mut NativeSession,
    bounds: (i32, i32),
) -> Result<(), (&'static str, WindowsRdpHostError)> {
    session
        .host
        .set_bounds(0, 0, bounds.0, bounds.1)
        .map_err(|error| ("set_bounds", error))?;
    session
        .host
        .set_visible(false)
        .map_err(|error| ("set_visible_before_connect", error))?;
    println!("presentation: host hidden before connect");
    Ok(())
}

fn connect_session(
    session: &mut NativeSession,
    credentials: &WindowsRdpCredentialBundle,
    options: &WindowsRdpConnectionOptions,
) -> Result<(), (&'static str, WindowsRdpHostError)> {
    session
        .host
        .apply_credentials(credentials)
        .map_err(|error| ("apply_credentials", error))?;
    println!("credentials: applied");
    session
        .host
        .connect(options)
        .map_err(|error| ("connect", error))?;
    println!("connect: synchronous call succeeded; waiting for events");
    Ok(())
}

fn initialization_error(stage: &str, error: WindowsRdpHostError) -> Initialization {
    log_host_error(stage, error);
    (
        None,
        "Invalid RDP connection options; see console".to_owned(),
        None,
    )
}

fn failed_after_host_error(
    mut session: NativeSession,
    stage: &str,
    error: WindowsRdpHostError,
) -> Initialization {
    log_host_error(stage, error);
    hide_after_failure(&mut session);
    finish_failure_cleanup(session, stage, "RDP initialization failed")
}

fn failed_after_overlay_error(
    mut session: NativeSession,
    stage: &str,
    error: String,
) -> Initialization {
    eprintln!("ERROR: stage={stage} error={error}");
    hide_after_failure(&mut session);
    finish_failure_cleanup(session, stage, "RDP presentation failed")
}

fn hide_after_failure(session: &mut NativeSession) {
    session.release_composition();
    if let Err(error) = session.overlay.hide() {
        eprintln!("ERROR: stage=failure_cleanup_hide_overlay error={error}");
    }
    if session.host.lifecycle() == WindowsRdpHostLifecycle::Open {
        if let Err(error) = session.host.set_visible(false) {
            log_host_error("failure_cleanup_set_visible", error);
        }
    }
}

fn finish_failure_cleanup(
    mut session: NativeSession,
    stage: &str,
    summary: &str,
) -> Initialization {
    match session.host.close() {
        Ok(()) => finish_overlay_cleanup(session, stage, summary),
        Err(error) => {
            log_host_error("failure_cleanup_close", error);
            (
                Some(session),
                format!("{summary} at {stage}; native cleanup needs another close attempt"),
                None,
            )
        }
    }
}

fn finish_overlay_cleanup(
    mut session: NativeSession,
    stage: &str,
    summary: &str,
) -> Initialization {
    match session.overlay.close() {
        Ok(()) => (None, format!("{summary} at {stage}; see console"), None),
        Err(error) => {
            eprintln!("ERROR: stage=failure_cleanup_close_overlay error={error}");
            (
                Some(session),
                format!("{summary} at {stage}; overlay cleanup needs another close attempt"),
                None,
            )
        }
    }
}
