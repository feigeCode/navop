use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_rdp_host::{
    WindowsRdpColorDepth, WindowsRdpConnectionOptions, WindowsRdpCredentialBundle, WindowsRdpHost,
    WindowsRdpHostError, WindowsRdpHostLifecycle, WindowsRdpParentWindow,
};

use super::{host_options, log_host_error, physical_viewport_size};
use crate::{cli::Config, native_overlay::NativeOverlay};

pub(super) struct NativeSession {
    pub(super) host: WindowsRdpHost,
    pub(super) overlay: NativeOverlay,
}

pub(super) type Initialization = (Option<NativeSession>, String, Option<(i32, i32)>);

impl NativeSession {
    pub(super) fn prepare_host_close(&mut self) {
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
}

pub(super) fn initialize(config: Config, window: &Window) -> Initialization {
    log_config(&config);
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
    let session = match create_session(owner) {
        Ok(session) => session,
        Err(error) => return (None, error, None),
    };
    finish_initialization(session, credentials, connection_options, window)
}

fn log_config(config: &Config) {
    println!(
        "config: host={} port={} username_present={} domain_present={} password_env_present={} desktop={}x{} timeout_seconds={}",
        config.host,
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
    let raw = window
        .window_handle()
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

fn create_session(owner: usize) -> Result<NativeSession, String> {
    let overlay = NativeOverlay::create(owner).map_err(|error| {
        eprintln!("ERROR: stage=create_native_overlay error={error}");
        "Failed to create the owned native RDP overlay; see console".to_owned()
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
    Ok(NativeSession { host, overlay })
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
    if let Err(error) = configure_presentation(&mut session, bounds) {
        return failed_after_host_error(session, error.0, error.1);
    }
    if let Err(error) = session.overlay.synchronize(0, 0, bounds.0, bounds.1) {
        return failed_after_overlay_error(session, "initial_overlay_bounds", error);
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
        .set_visible(true)
        .map_err(|error| ("set_visible", error))
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
    if let Err(error) = session.host.focus() {
        log_host_error("focus_best_effort", error);
    } else {
        println!("focus: success");
    }
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
