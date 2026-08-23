use std::sync::{Arc, OnceLock, atomic::AtomicU64};
use std::time::{Duration, Instant};

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
use std::sync::atomic::Ordering;
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
use std::{cell::RefCell, rc::Rc};

use gpui::*;
use gpui_component::{ActiveTheme, Icon, IconName};
use one_core::tab_container::{TabContent, TabContentEvent};
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
use remote_desktop::parse_destination;
use remote_desktop::{
    RemoteDesktopCapabilities, RemoteDesktopConnectionOptions, RemoteDesktopFailure,
    RemoteDesktopInput, RemoteDesktopOutput, RemoteDesktopProtocol,
    RemoteDesktopProviderVersionError, RemoteDesktopRuntime, RemoteDesktopSize, RemoteKey,
    RemoteMouseButton, RemoteNamedKey, ResizeSupport, create_backend,
};
use rust_i18n::t;

use crate::keyboard::keystroke_to_remote_key_for_protocol;
use crate::modifiers::{RdpKeyboardState, keyboard_state_inputs};
use crate::pointer::{LocalBounds, scale_filled_window_pointer_position};
use crate::shortcuts::{
    ClipboardShortcut, clipboard_shortcut_inputs, is_clipboard_platform_shortcut,
};
use crate::view::frame_lifecycle::RenderedFrameLifecycle;

mod clipboard;
#[cfg(target_os = "macos")]
mod clipboard_macos;
#[cfg(any(target_os = "windows", test))]
mod clipboard_windows;
mod cursor;
mod frame_lifecycle;
mod frame_sync;
mod frames;
mod input;
// Task 5 freezes the owner-thread event reducer before Task 6 creates and
// presents the native child window.
#[cfg(feature = "windows-native-rdp")]
#[allow(dead_code)]
mod native_events;
mod notifications;
mod output;
// The full selection taxonomy is exercised by cross-platform contract tests,
// while several variants are only constructed by the Windows production path.
#[allow(dead_code)]
mod presentation;
// Capability probing is compiled for contract tests on every platform but is
// only consumed by the production presentation factory on Windows.
#[allow(dead_code)]
mod presentation_capability;
mod render;
mod resize;
mod surface;
// Pure lifecycle/bounds tests run cross-platform; the production adapter and
// sink are only constructed by the Windows native-RDP build.
#[allow(dead_code)]
mod windows_native;
#[allow(dead_code)]
mod windows_native_display;
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
mod windows_native_display_integration;
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
mod windows_native_overlay;
#[cfg(feature = "windows-native-rdp")]
mod windows_native_policy;

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(800);
const RESIZE_MIN_INTERVAL: Duration = Duration::from_millis(1200);
const RESIZE_DELTA_THRESHOLD: u16 = 16;
const RDP_INITIAL_LAYOUT_DEBOUNCE: Duration = Duration::from_millis(150);
const REMOTE_DESKTOP_CONTEXT: &str = "RemoteDesktopView";
const REMOTE_DESKTOP_DIAGNOSTICS_ENV: &str = "NAVOP_REMOTE_DESKTOP_DIAGNOSTICS";
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
const WINDOWS_NATIVE_EVENT_POLL_INTERVAL: Duration = Duration::from_millis(16);
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
const WINDOWS_NATIVE_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
const WINDOWS_NATIVE_FORCE_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
static NEXT_WINDOWS_NATIVE_RDP_GENERATION: AtomicU64 = AtomicU64::new(1);

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowsNativeFocusTarget {
    Parent,
    NativeChild,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WindowsNativeOperationToken {
    generation: u64,
    serial: u64,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
struct WindowsNativeOperation {
    token: WindowsNativeOperationToken,
    registration: windows_rdp_host::WindowsRdpRegistration,
    native: windows_native::WindowsNativeAdapter,
    event_state: native_events::NativeRdpEventState,
    bounds: Option<(Bounds<Pixels>, f32)>,
    tab_active: bool,
    allow_activation: bool,
    lifecycle_dirty: bool,
    focus_requested: bool,
    display_request: Option<windows_native_display::WindowsNativeDisplayRequest>,
    was_presentation_ready: bool,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
pub(crate) struct WindowsNativeCloseOperation {
    registration: windows_rdp_host::WindowsRdpRegistration,
    native: windows_native::WindowsNativeAdapter,
    event_state: native_events::NativeRdpEventState,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
impl WindowsNativeCloseOperation {
    /// Consumes the close operation and returns the adapter for intentional
    /// leak-quarantine when a shutdown deadline hits. Leaking performs no COM
    /// calls; pending callbacks must never observe a dropped host during
    /// teardown.
    pub(crate) fn into_leaked_adapter(self) -> windows_native::WindowsNativeAdapter {
        self.native
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
pub(crate) enum WindowsNativeCloseTake {
    Pending,
    Closed,
    Failed,
    Ready(WindowsNativeCloseOperation),
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
enum WindowsNativeDisplayCompletion {
    Succeeded(windows_native_display::WindowsNativeDisplayRequest),
    Failed {
        request: windows_native_display::WindowsNativeDisplayRequest,
        completed_at: Instant,
    },
    Suspended(windows_native_display::WindowsNativeDisplayRequest),
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
#[derive(Clone, Copy, Debug)]
struct WindowsNativeReadinessSnapshot {
    generation: u64,
    requested_visible: bool,
    activation_pending: bool,
    can_present: bool,
    presentation_ready: bool,
    transitioned: bool,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
struct WindowsNativeOperationResult {
    operation: WindowsNativeOperation,
    effects: Vec<native_events::NativeRdpUiEffect>,
    requested_focus: Option<WindowsNativeFocusTarget>,
    display_completion: Option<WindowsNativeDisplayCompletion>,
    readiness: WindowsNativeReadinessSnapshot,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
struct WindowsNativeOperationCommit {
    focus_handle: Option<FocusHandle>,
    notifications: Vec<native_events::NativeRdpNotificationRequest>,
    entity_id: EntityId,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
enum WindowsNativeOperationCommitResult {
    Attached(WindowsNativeOperationCommit),
    Rejected(WindowsNativeOperation),
}

#[cfg(target_os = "macos")]
const REMOTE_COPY_SHORTCUT: &str = "cmd-c";
#[cfg(not(target_os = "macos"))]
const REMOTE_COPY_SHORTCUT: &str = "ctrl-shift-c";
#[cfg(target_os = "macos")]
const REMOTE_PASTE_SHORTCUT: &str = "cmd-v";
#[cfg(not(target_os = "macos"))]
const REMOTE_PASTE_SHORTCUT: &str = "ctrl-shift-v";

actions!(
    remote_desktop_view,
    [SendTab, SendShiftTab, RemoteCopy, RemotePaste, UseCanvas]
);

fn remote_desktop_tab_title(title: &str, tab_index: Option<usize>) -> String {
    if let Some(index) = tab_index {
        format!("{title}({index})")
    } else {
        title.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionResetReason {
    Reconnecting,
    ConnectionFailure,
    Terminated,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowsNativeCloseRetryMode {
    WaitForConfirmation,
    ForceClose,
}

/// Pure-Rust snapshot of everything the Windows native RDP COM stages need.
///
/// Built while the App borrow is held (Phase 1) so that every subsequent
/// ActiveX/COM call (host creation, bounds update, credentials, connect) can
/// run without any App borrow: those calls pump Win32 messages, and the pump
/// can dispatch pending GPUI foreground tasks that re-borrow the App.
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
struct WindowsNativePresentationInputs {
    owner: Result<usize, windows_native::WindowsNativeAdapterCreateError>,
    generation: u64,
    bounds: Bounds<Pixels>,
    scale_factor: f32,
    options: RemoteDesktopConnectionOptions,
    desktop_size: (u32, u32),
}

/// Failure modes of the borrow-free COM preparation stage (Phase 2).
///
/// These all happen before the host is registered for shutdown drain, so
/// cleanup uses the unregistered failure path.
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
enum WindowsNativePrepareFailure {
    Create(windows_native::WindowsNativeAdapterCreateError),
    Bounds {
        native: windows_native::WindowsNativeAdapter,
        error: anyhow::Error,
    },
    Endpoint {
        native: windows_native::WindowsNativeAdapter,
        error: anyhow::Error,
    },
    SharedFoldersUnsupported {
        native: windows_native::WindowsNativeAdapter,
        count: usize,
    },
    ConnectionOptions {
        native: windows_native::WindowsNativeAdapter,
        error: windows_rdp_host::WindowsRdpHostError,
    },
    Credentials {
        native: windows_native::WindowsNativeAdapter,
        error: windows_rdp_host::WindowsRdpHostError,
    },
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
type WindowsNativePreparedConnection = Result<
    (
        windows_native::WindowsNativeAdapter,
        windows_rdp_host::WindowsRdpConnectionOptions,
    ),
    WindowsNativePrepareFailure,
>;

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
struct WindowsNativeInitializationCleanup {
    native: windows_native::WindowsNativeAdapter,
    registration: Option<windows_rdp_host::WindowsRdpRegistration>,
    reason: &'static str,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
enum WindowsNativeAdmission {
    Connect {
        native: windows_native::WindowsNativeAdapter,
        registration: windows_rdp_host::WindowsRdpRegistration,
        connection_options: windows_rdp_host::WindowsRdpConnectionOptions,
    },
    Cleanup(WindowsNativeInitializationCleanup),
    Complete,
}

/// Phase 2 of the Windows native RDP initialization: creates the ActiveX
/// host and prepares the connection WITHOUT holding any App borrow.
///
/// Every call in here — `create_with_owner` (CreateWindowEx,
/// `AtlAxCreateControl`, `CoCreateInstance`), `update_bounds`,
/// `apply_credentials` — pumps Win32 messages. If the App borrow were held
/// across any of them, the pump could dispatch a pending GPUI foreground
/// task that re-borrows the App and panics with "RefCell already borrowed".
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
fn prepare_windows_native_connection(
    inputs: WindowsNativePresentationInputs,
) -> WindowsNativePreparedConnection {
    let WindowsNativePresentationInputs {
        owner,
        generation,
        bounds,
        scale_factor,
        options,
        desktop_size,
    } = inputs;

    let mut native = windows_native::WindowsNativeAdapter::create_with_owner(
        owner.map_err(WindowsNativePrepareFailure::Create)?,
        generation,
    )
    .map_err(WindowsNativePrepareFailure::Create)?;
    if let Err(error) = native.update_bounds(bounds, point(px(0.0), px(0.0)), scale_factor) {
        return Err(WindowsNativePrepareFailure::Bounds { native, error });
    }
    let (host, port) = match parse_destination(&options.destination) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            return Err(WindowsNativePrepareFailure::Endpoint { native, error });
        }
    };
    let policy = windows_native_policy::connection_policy(&options.rdp);
    tracing::info!(
        desktop_width = desktop_size.0,
        desktop_height = desktop_size.1,
        ?policy,
        shared_folder_count = options.rdp.resources.shared_folders.len(),
        "native RDP: stage=connection-policy"
    );
    let shared_folder_count = options.rdp.resources.shared_folders.len();
    if shared_folder_count != 0 {
        return Err(WindowsNativePrepareFailure::SharedFoldersUnsupported {
            native,
            count: shared_folder_count,
        });
    }
    let connection_options = match windows_rdp_host::WindowsRdpConnectionOptions::new(
        host,
        port,
        desktop_size.0,
        desktop_size.1,
        windows_rdp_host::WindowsRdpColorDepth::Bpp32,
    ) {
        Ok(options) => options.with_policy(policy),
        Err(error) => {
            return Err(WindowsNativePrepareFailure::ConnectionOptions { native, error });
        }
    };
    let mut credentials = windows_rdp_host::WindowsRdpCredentialBundle::new();
    if let Some(username) = options.username.as_ref() {
        credentials.set_username(username.clone());
    }
    if let Some(domain) = options.domain.as_ref() {
        credentials.set_domain(domain.clone());
    }
    if let Some(password) = options.password.as_ref() {
        credentials.set_server_password(password.clone());
    }
    windows_native_policy::apply_gateway_credentials(&mut credentials, &options.rdp);
    if let Err(error) = native.apply_credentials(&credentials) {
        return Err(WindowsNativePrepareFailure::Credentials { native, error });
    }

    Ok((native, connection_options))
}

fn preserve_presented_frame_during_session_reset(reason: SessionResetReason) -> bool {
    matches!(reason, SessionResetReason::Reconnecting)
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
fn next_windows_native_rdp_generation() -> u64 {
    NEXT_WINDOWS_NATIVE_RDP_GENERATION.fetch_add(1, Ordering::Relaxed)
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
fn record_detached_windows_native_terminal(
    registration: windows_rdp_host::WindowsRdpRegistration,
    outcome: windows_rdp_host::WindowsRdpTerminalOutcome,
    generation: u64,
    reason: &'static str,
    cx: &gpui::AsyncApp,
) {
    if crate::windows_native_shutdown::record_windows_native_rdp_terminal_async(
        registration,
        outcome,
        cx,
    )
    .was_rejected()
    {
        tracing::error!(
            generation,
            reason,
            ?outcome,
            "Windows native RDP detached cleanup terminal dispatcher became unavailable"
        );
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
fn reset_windows_native_presentation_schedule(
    this: &WeakEntity<RemoteDesktopView>,
    generation: u64,
    cx: &mut gpui::AsyncApp,
) {
    let _ = this.update(cx, |this, cx| {
        if this.windows_native_initialization_generation == Some(generation) {
            this.windows_native_initialization_generation = None;
            cx.notify();
        }
    });
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
async fn cleanup_rejected_windows_native_preparation(
    prepared: WindowsNativePreparedConnection,
    cx: &mut gpui::AsyncApp,
) {
    let cleanup = match prepared {
        Ok((native, _)) => Some(WindowsNativeInitializationCleanup {
            native,
            registration: None,
            reason: "admission-dispatch",
        }),
        Err(WindowsNativePrepareFailure::Create(error)) => {
            tracing::warn!(
                error = %error,
                ?error,
                "Windows native RDP preparation completed after its view disappeared"
            );
            None
        }
        Err(WindowsNativePrepareFailure::Bounds { native, error }) => {
            tracing::warn!(
                ?error,
                "Windows native RDP bounds preparation failed after its view disappeared"
            );
            Some(WindowsNativeInitializationCleanup {
                native,
                registration: None,
                reason: "bounds",
            })
        }
        Err(WindowsNativePrepareFailure::Endpoint { native, error }) => {
            tracing::warn!(
                ?error,
                "Windows native RDP endpoint preparation failed after its view disappeared"
            );
            Some(WindowsNativeInitializationCleanup {
                native,
                registration: None,
                reason: "endpoint",
            })
        }
        Err(WindowsNativePrepareFailure::SharedFoldersUnsupported { native, count }) => {
            tracing::warn!(
                shared_folder_count = count,
                "Windows native RDP shared-folder preparation was rejected after its view disappeared"
            );
            Some(WindowsNativeInitializationCleanup {
                native,
                registration: None,
                reason: "shared-folders",
            })
        }
        Err(WindowsNativePrepareFailure::ConnectionOptions { native, error }) => {
            tracing::warn!(
                ?error,
                "Windows native RDP option preparation failed after its view disappeared"
            );
            Some(WindowsNativeInitializationCleanup {
                native,
                registration: None,
                reason: "connection-options",
            })
        }
        Err(WindowsNativePrepareFailure::Credentials { native, error }) => {
            tracing::warn!(
                ?error,
                "Windows native RDP credential preparation failed after its view disappeared"
            );
            Some(WindowsNativeInitializationCleanup {
                native,
                registration: None,
                reason: "credentials",
            })
        }
    };
    if let Some(cleanup) = cleanup {
        cleanup_windows_native_initialization(
            cleanup.native,
            cleanup.registration,
            cleanup.reason,
            cx,
        )
        .await;
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
async fn cleanup_windows_native_initialization(
    mut native: windows_native::WindowsNativeAdapter,
    registration: Option<windows_rdp_host::WindowsRdpRegistration>,
    reason: &'static str,
    cx: &mut gpui::AsyncApp,
) -> bool {
    let generation = native.generation();
    let local_deadline = Instant::now() + WINDOWS_NATIVE_FORCE_CLOSE_TIMEOUT;
    loop {
        let mut focus_parent = || {};
        match native.force_close(&mut focus_parent) {
            Ok(windows_native::NativeDestroyProgress::Destroyed) => {
                if let Some(registration) = registration {
                    record_detached_windows_native_terminal(
                        registration,
                        windows_rdp_host::WindowsRdpTerminalOutcome::Destroyed,
                        generation,
                        reason,
                        cx,
                    );
                }
                return true;
            }
            Ok(windows_native::NativeDestroyProgress::PendingCallbacks) => {}
            Err(_) if native.is_destroyed() => {
                if let Some(registration) = registration {
                    record_detached_windows_native_terminal(
                        registration,
                        windows_rdp_host::WindowsRdpTerminalOutcome::Destroyed,
                        generation,
                        reason,
                        cx,
                    );
                }
                return true;
            }
            Err(error) => {
                tracing::warn!(
                    ?error,
                    generation,
                    reason,
                    "failed to retry detached Windows native RDP cleanup"
                );
            }
        }

        let deadline =
            crate::windows_native_shutdown::detached_cleanup_deadline(local_deadline, cx);
        if Instant::now() >= deadline {
            tracing::error!(
                generation,
                reason,
                "leaking Windows native RDP adapter after detached cleanup timed out"
            );
            let _ = Box::leak(Box::new(native));
            if let Some(registration) = registration {
                record_detached_windows_native_terminal(
                    registration,
                    windows_rdp_host::WindowsRdpTerminalOutcome::TimedOutLeaked,
                    generation,
                    reason,
                    cx,
                );
            }
            return false;
        }

        cx.background_executor()
            .timer(Duration::from_millis(16))
            .await;
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
fn detach_windows_native_cleanup(
    native: windows_native::WindowsNativeAdapter,
    registration: Option<windows_rdp_host::WindowsRdpRegistration>,
    cx: &App,
    reason: &'static str,
) {
    cx.spawn(async move |cx| {
        cleanup_windows_native_initialization(native, registration, reason, cx).await;
    })
    .detach();
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
async fn cleanup_rejected_windows_native_operation(
    operation: WindowsNativeOperation,
    reason: &'static str,
    cx: &mut gpui::AsyncApp,
) {
    let WindowsNativeOperation {
        native,
        registration,
        ..
    } = operation;
    let detached =
        crate::windows_native_shutdown::mark_windows_native_rdp_detached_async(registration, cx);
    if detached.was_rejected() {
        tracing::error!(
            token = registration.token(),
            generation = registration.generation(),
            reason,
            "Windows native RDP runtime ownership transfer was rejected"
        );
    }
    cleanup_windows_native_initialization(native, Some(registration), reason, cx).await;
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
fn execute_windows_native_operation(
    mut operation: WindowsNativeOperation,
) -> WindowsNativeOperationResult {
    use native_events::NativeRdpUiEffect;

    let effects = operation.native.drain_events(&mut operation.event_state);
    let mut requested_focus = None;
    let mut allow_activation = operation.allow_activation;

    for effect in &effects {
        match effect {
            NativeRdpUiEffect::FocusReleased => {
                requested_focus = Some(WindowsNativeFocusTarget::Parent);
            }
            NativeRdpUiEffect::LoginComplete { .. } => {
                operation.native.mark_login_complete();
                allow_activation = operation.tab_active;
            }
            NativeRdpUiEffect::Reconnected { .. } => {
                operation.native.mark_login_complete();
                allow_activation = operation.tab_active;
                if operation.tab_active {
                    requested_focus = Some(WindowsNativeFocusTarget::NativeChild);
                }
            }
            NativeRdpUiEffect::Reconnecting { .. } => {
                allow_activation = false;
                let mut focus_parent = false;
                if let Err(error) = operation.native.begin_reconnect(&mut || {
                    focus_parent = true;
                }) {
                    tracing::warn!(
                        ?error,
                        "failed to reset Windows native RDP presentation for reconnect"
                    );
                }
                if focus_parent {
                    requested_focus = Some(WindowsNativeFocusTarget::Parent);
                }
            }
            NativeRdpUiEffect::FatalError { .. }
            | NativeRdpUiEffect::LogonError { .. }
            | NativeRdpUiEffect::Disconnected { .. } => {
                allow_activation = false;
                let mut focus_parent = false;
                if let Err(error) = operation.native.deactivate(&mut || {
                    focus_parent = true;
                }) {
                    tracing::warn!(
                        ?error,
                        "failed to deactivate Windows native RDP presentation after failure"
                    );
                }
                if focus_parent {
                    requested_focus = Some(WindowsNativeFocusTarget::Parent);
                }
            }
            NativeRdpUiEffect::CloseConfirmed
            | NativeRdpUiEffect::Connecting { .. }
            | NativeRdpUiEffect::Connected { .. }
            | NativeRdpUiEffect::Warning { .. }
            | NativeRdpUiEffect::Unknown { .. } => {}
        }
    }

    if operation.event_state.take_focus_release_pending() && requested_focus.is_none() {
        requested_focus = Some(WindowsNativeFocusTarget::Parent);
    }

    if let Some((bounds, scale_factor)) = operation.bounds
        && let Err(error) =
            operation
                .native
                .update_bounds(bounds, point(px(0.0), px(0.0)), scale_factor)
    {
        tracing::warn!(
            ?error,
            ?bounds,
            scale_factor,
            "failed to update Windows native RDP bounds"
        );
    }

    if operation.lifecycle_dirty && !operation.tab_active {
        let mut focus_parent = false;
        if let Err(error) = operation.native.deactivate(&mut || {
            focus_parent = true;
        }) {
            tracing::warn!(?error, "failed to deactivate Windows native RDP presentation");
        }
        if focus_parent {
            requested_focus = Some(WindowsNativeFocusTarget::Parent);
        }
    } else if operation.lifecycle_dirty && operation.tab_active && allow_activation {
        if let Err(error) = operation.native.activate(false) {
            tracing::warn!(?error, "failed to activate Windows native RDP presentation");
        }
    }

    let now_presentable = operation.native.refresh_native_readiness();
    if now_presentable && allow_activation && operation.native.activation_pending()
        && let Err(error) = operation.native.activate(false)
    {
        tracing::trace!(
            ?error,
            "deferred Windows native RDP activation is still blocked"
        );
    }

    if operation.tab_active
        && allow_activation
        && (operation.focus_requested
            || requested_focus == Some(WindowsNativeFocusTarget::NativeChild))
    {
        match operation.native.focus() {
            Ok(()) => requested_focus = None,
            Err(error) => {
                tracing::trace!(
                    ?error,
                    "deferred Windows native RDP focus is still blocked"
                );
            }
        }
    }
    if !operation.tab_active {
        requested_focus = None;
    }

    let completed_at = Instant::now();
    let display_completion = operation.display_request.map(|request| {
        windows_native_display_integration::log_display_request(request);
        if operation.native.generation() != request.generation || !operation.native.is_open() {
            windows_native_display_integration::log_display_target_unavailable(request);
            return WindowsNativeDisplayCompletion::Suspended(request);
        }
        let result = windows_rdp_host::WindowsRdpSessionDisplaySettings::viewport(
            request.settings.width,
            request.settings.height,
            request.settings.desktop_scale_factor,
        )
        .and_then(|settings| operation.native.update_session_display_settings(settings));
        match result {
            Ok(()) => {
                windows_native_display_integration::log_display_success(request);
                WindowsNativeDisplayCompletion::Succeeded(request)
            }
            Err(error) => {
                windows_native_display_integration::log_display_failure(request, error);
                WindowsNativeDisplayCompletion::Failed {
                    request,
                    completed_at,
                }
            }
        }
    });

    let readiness = WindowsNativeReadinessSnapshot {
        generation: operation.native.generation(),
        requested_visible: operation.native.requested_visible(),
        activation_pending: operation.native.activation_pending(),
        can_present: operation.native.can_present(),
        presentation_ready: operation.native.presentation_ready(),
        transitioned: operation.native.presentation_ready() && !operation.was_presentation_ready,
    };

    WindowsNativeOperationResult {
        operation,
        effects,
        requested_focus,
        display_completion,
        readiness,
    }
}

pub struct RemoteDesktopViewConfig {
    pub options: RemoteDesktopConnectionOptions,
    pub title: String,
    pub tab_index: Option<usize>,
}

pub struct RemoteDesktopView {
    options: RemoteDesktopConnectionOptions,
    title: String,
    input_tx: Option<tokio::sync::mpsc::UnboundedSender<RemoteDesktopInput>>,
    output_rx: Option<remote_desktop::output_mailbox::OutputMailboxReceiver>,
    presentation_tx: Option<tokio::sync::mpsc::UnboundedSender<presentation::PresentationCommand>>,
    presentation_queue: presentation::PresentationQueue,
    presentation_in_flight: bool,
    presentation_pacer: presentation::PresentationPacer,
    latest_presentation_frame_ticket: Arc<AtomicU64>,
    focus_handle: FocusHandle,
    latest_frame: Option<Arc<surface::RemoteDesktopSurface>>,
    rendered_frames: RenderedFrameLifecycle<Arc<surface::RemoteDesktopSurface>>,
    retired_textures: surface::RetiredTextureQueue,
    cursor: cursor::RemoteCursorState,
    frame_sync: frame_sync::FrameSyncTracker,
    capabilities: Option<RemoteDesktopCapabilities>,
    remote_size: Option<(u16, u16)>,
    content_bounds: Option<Bounds<Pixels>>,
    initial_size: resize::InitialSize,
    last_resize_size: Option<(u16, u16)>,
    pending_resize_size: Option<(u16, u16)>,
    pending_resize_updated_at: Option<Instant>,
    last_resize_sent_at: Option<Instant>,
    keyboard_state: RdpKeyboardState,
    last_clipboard_text: Option<String>,
    last_clipboard_files: Option<Vec<String>>,
    last_clipboard_sync_at: Option<Instant>,
    next_clipboard_transfer_id: u64,
    display_scale_factor: u32,
    status: SharedString,
    failure_detail: Option<SharedString>,
    connected: bool,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    native_login_complete: bool,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    windows_native_scale_factor: Option<f32>,
    tab_index: Option<usize>,
    startup_started_at: Instant,
    runtime_started_at: Option<Instant>,
    startup_connected_logged: bool,
    startup_frame_logged: bool,
    _initial_layout_task: Option<Task<()>>,
    _output_ready_task: Option<Task<()>>,
    _presentation_task: Option<Task<()>>,
    _presentation_pacing_task: Option<Task<()>>,
    presentation_initialization: presentation::RemoteDesktopPresentationInitialization,
    tab_active: bool,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    windows_native: Option<windows_native::WindowsNativeAdapter>,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    windows_native_registration: Option<windows_rdp_host::WindowsRdpRegistration>,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    native_event_state: Option<native_events::NativeRdpEventState>,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    pending_windows_native_notifications: Vec<native_events::NativeRdpNotificationRequest>,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    windows_native_initialization_generation: Option<u64>,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    windows_native_display: windows_native_display::WindowsNativeDisplayState,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    windows_native_operation_in_flight: Option<WindowsNativeOperationToken>,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    next_windows_native_operation_serial: u64,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    pending_windows_native_bounds: Option<(Bounds<Pixels>, f32)>,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    windows_native_lifecycle_dirty: bool,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    windows_native_focus_requested: bool,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    windows_native_close_requested: bool,
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    windows_native_close_in_flight: Option<windows_rdp_host::WindowsRdpRegistration>,
}

impl RemoteDesktopView {
    pub fn new(
        config: RemoteDesktopViewConfig,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let manage_native_cursor = config.options.protocol == RemoteDesktopProtocol::Rdp;
        // Standalone windows (tab_index: None, e.g. the fullscreen RDP popup)
        // have no tab container to drive TabContent::on_activate, so they must
        // start as the active presentation and request native focus directly.
        // Otherwise the native overlay stays hidden and the window renders
        // only the GPUI background.
        let standalone_window = config.tab_index.is_none();
        let presentation_initialization = if config.options.protocol == RemoteDesktopProtocol::Vnc {
            presentation::RemoteDesktopPresentationInitialization::Canvas {
                fallback_reason: None,
            }
        } else {
            presentation::RemoteDesktopPresentationInitialization::Pending
        };
        let focus_handle = cx.focus_handle();
        #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
        let native_event_window_handle = window_handle.clone();
        #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
        let output_poll_task = cx.spawn(async move |this, cx| {
            loop {
                if this
                    .update(cx, |this, _| this.windows_native_maintenance_is_pending())
                    .unwrap_or(false)
                {
                    let operation = match this.update(cx, |this, _| {
                        this.take_windows_native_operation()
                    }) {
                        Ok(Some(operation)) => operation,
                        Ok(None) => {
                            cx.background_executor()
                                .timer(WINDOWS_NATIVE_EVENT_POLL_INTERVAL)
                                .await;
                            continue;
                        }
                        Err(_) => break,
                    };
                    let result = execute_windows_native_operation(operation);
                    let result_slot = Rc::new(RefCell::new(Some(result)));
                    let result_for_commit = result_slot.clone();
                    let commit = this.update(cx, |this, cx| {
                        result_for_commit
                            .borrow_mut()
                            .take()
                            .map(|result| this.commit_windows_native_operation(result, cx))
                    });
                    let commit = match commit {
                        Ok(Some(commit)) => commit,
                        Ok(None) => {
                            tracing::error!(
                                "Windows native RDP maintenance commit lost its owner payload"
                            );
                            continue;
                        }
                        Err(_) => {
                            if let Some(result) = result_slot.borrow_mut().take() {
                                cleanup_rejected_windows_native_operation(
                                    result.operation,
                                    "maintenance-dispatch",
                                    cx,
                                )
                                .await;
                            }
                            break;
                        }
                    };
                    let commit = match commit {
                        WindowsNativeOperationCommitResult::Attached(commit) => commit,
                        WindowsNativeOperationCommitResult::Rejected(operation) => {
                            cleanup_rejected_windows_native_operation(
                                operation,
                                "maintenance-stale",
                                cx,
                            )
                            .await;
                            continue;
                        }
                    };
                    if commit.focus_handle.is_some() || !commit.notifications.is_empty() {
                        let _ = native_event_window_handle.update(cx, |_, window, cx| {
                            if let Some(focus_handle) = commit.focus_handle {
                                window.focus(&focus_handle, cx);
                            }
                            for request in commit.notifications {
                                notifications::defer_windows_native_rdp_notification(
                                    request,
                                    commit.entity_id,
                                    window,
                                    cx,
                                );
                            }
                        });
                    }
                }
                cx.background_executor()
                    .timer(WINDOWS_NATIVE_EVENT_POLL_INTERVAL)
                    .await;
            }
        });
        #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
        output_poll_task.detach();

        cx.on_release(move |this, cx| {
            close_runtime_once(&mut this.input_tx);
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            {
                this.windows_native_display.reset();
            }
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            if let Some(native) = this.windows_native.take() {
                let registration = this.windows_native_registration.take();
                this.native_event_state.take();
                let focus_handle = this.focus_handle.clone();
                let _ = window_handle.update(cx, |_, window, cx| {
                    window.focus(&focus_handle, cx);
                });
                // `force_close` pumps Win32/COM messages. The release hook
                // still owns the entity/App borrow, so defer every native
                // close attempt until after the hook returns.
                if let Some(registration) = registration {
                    crate::windows_native_shutdown::mark_windows_native_rdp_detached(
                        registration,
                        cx,
                    );
                    detach_windows_native_cleanup(native, Some(registration), cx, "view release");
                } else {
                    detach_windows_native_cleanup(native, None, cx, "view release");
                }
            } else if let Some(registration) = this.windows_native_registration.take() {
                // The adapter is not in the entity. If an operation owns it,
                // that operation's rejected-commit cleanup performs the real
                // teardown and records the terminal outcome; recording
                // `OwnerLost` here would produce a second terminal outcome.
                if this.windows_native_operation_in_flight.is_some() {
                    tracing::warn!(
                        token = registration.token(),
                        generation = registration.generation(),
                        "Windows native RDP view released while a maintenance operation owns the adapter"
                    );
                } else if this.windows_native_close_in_flight.is_some() {
                    tracing::warn!(
                        token = registration.token(),
                        generation = registration.generation(),
                        "Windows native RDP view released while a close operation owns the adapter"
                    );
                } else {
                    tracing::error!(
                        token = registration.token(),
                        generation = registration.generation(),
                        "Windows native RDP view released with a registration but no adapter"
                    );
                    crate::windows_native_shutdown::record_windows_native_rdp_terminal(
                        registration,
                        windows_rdp_host::WindowsRdpTerminalOutcome::OwnerLost,
                        cx,
                    );
                }
            }

            this.output_rx.take();
            this.presentation_tx.take();
            this.presentation_queue.clear();
            this.presentation_in_flight = false;
            this.reset_presentation_pacing();
            this._initial_layout_task.take();
            this._output_ready_task.take();
            this._presentation_task.take();
            this.retired_textures.retire_all(
                this.rendered_frames
                    .take_all_distinct(this.latest_frame.take()),
            );
            let textures = this.retired_textures.take_all();
            let cursor_images = this.cursor.release_all_images();
            let _ = window_handle.update(cx, move |_, window, _| {
                for texture in textures {
                    if let Err(error) = window.drop_dynamic_texture(texture) {
                        tracing::warn!(?error, "failed to release remote desktop texture");
                    }
                }
                for image in cursor_images {
                    if let Err(error) = window.drop_image(image) {
                        tracing::warn!(?error, "failed to release remote desktop cursor");
                    }
                }
            });
        })
        .detach();

        Self {
            options: config.options,
            title: config.title,
            input_tx: None,
            output_rx: None,
            presentation_tx: None,
            presentation_queue: presentation::PresentationQueue::default(),
            presentation_in_flight: false,
            presentation_pacer: presentation::PresentationPacer::default(),
            latest_presentation_frame_ticket: Arc::new(AtomicU64::new(0)),
            focus_handle,
            latest_frame: None,
            rendered_frames: RenderedFrameLifecycle::default(),
            retired_textures: surface::RetiredTextureQueue::default(),
            cursor: cursor::RemoteCursorState::new(manage_native_cursor),
            frame_sync: frame_sync::FrameSyncTracker::default(),
            capabilities: None,
            remote_size: None,
            content_bounds: None,
            initial_size: resize::InitialSize::default(),
            last_resize_size: None,
            pending_resize_size: None,
            pending_resize_updated_at: None,
            last_resize_sent_at: None,
            keyboard_state: RdpKeyboardState::default(),
            last_clipboard_text: None,
            last_clipboard_files: None,
            last_clipboard_sync_at: None,
            next_clipboard_transfer_id: clipboard::FIRST_LOCAL_CLIPBOARD_TRANSFER_ID,
            display_scale_factor: 100,
            status: SharedString::from(t!("RemoteDesktop.status_waiting_layout").to_string()),
            failure_detail: None,
            connected: false,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            native_login_complete: false,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native_scale_factor: None,
            tab_index: config.tab_index,
            startup_started_at: Instant::now(),
            runtime_started_at: None,
            startup_connected_logged: false,
            startup_frame_logged: false,
            _initial_layout_task: None,
            _output_ready_task: None,
            _presentation_task: None,
            _presentation_pacing_task: None,
            presentation_initialization,
            tab_active: standalone_window,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native: None,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native_registration: None,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            native_event_state: None,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            pending_windows_native_notifications: Vec::new(),
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native_initialization_generation: None,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native_display: Default::default(),
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native_operation_in_flight: None,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            next_windows_native_operation_serial: 1,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            pending_windows_native_bounds: None,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native_lifecycle_dirty: false,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native_focus_requested: standalone_window,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native_close_requested: false,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native_close_in_flight: None,
        }
    }

    fn cancel_presentation_pacing(&mut self) {
        self.presentation_pacer.invalidate_timer();
        self._presentation_pacing_task.take();
    }

    fn reset_presentation_pacing(&mut self) {
        self.presentation_pacer.reset();
        self._presentation_pacing_task.take();
    }

    pub(super) fn uses_windows_native_presentation(&self) -> bool {
        #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
        {
            // Stable product-state check: maintenance and close operations
            // temporarily take the adapter out of the entity, which must not
            // flip rendering back to the canvas path mid-session.
            matches!(
                self.presentation_initialization,
                presentation::RemoteDesktopPresentationInitialization::Native
            )
        }
        #[cfg(not(all(feature = "windows-native-rdp", target_os = "windows")))]
        {
            false
        }
    }

    pub(super) fn ensure_presentation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(
            self.presentation_initialization,
            presentation::RemoteDesktopPresentationInitialization::Pending
        ) {
            return;
        }
        if self.options.protocol != RemoteDesktopProtocol::Rdp {
            self.presentation_initialization =
                presentation::RemoteDesktopPresentationInitialization::Canvas {
                    fallback_reason: None,
                };
            return;
        }

        #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
        {
            self.schedule_windows_native_presentation(window, cx);
        }

        #[cfg(not(all(feature = "windows-native-rdp", target_os = "windows")))]
        {
            let _ = (window, cx);
            let creation = presentation::create_remote_desktop_presentation_with(
                presentation::current_remote_desktop_platform(),
                self.options.backend_preference,
                presentation_capability::current_windows_native_rdp_capability,
                || Ok::<(), std::convert::Infallible>(()),
                |error| match *error {},
            );
            match creation {
                Ok(presentation::RemoteDesktopPresentationCreation::Canvas { fallback_reason }) => {
                    self.presentation_initialization =
                        presentation::RemoteDesktopPresentationInitialization::Canvas {
                            fallback_reason,
                        };
                }
                Ok(presentation::RemoteDesktopPresentationCreation::Native(())) => {
                    unreachable!("native Windows presentation is unavailable in this build")
                }
                Err(error) => {
                    tracing::warn!(?error, "failed to select the remote desktop presentation");
                    self.fail_presentation_initialization(
                        presentation::RemoteDesktopPresentation::NativeWindows,
                        true,
                        Some(format!(
                            "Windows native RDP diagnostic\nstage=selection\nerror={error:?}"
                        )),
                    );
                }
            }
        }
    }

    /// Defers the Windows native RDP initialization out of `render` into five
    /// borrow-scoped phases.
    ///
    /// Every ActiveX/COM call in the initialization sequence pumps Win32
    /// messages (`CreateWindowEx`, `AtlAxCreateControl`, `CoCreateInstance`,
    /// bounds updates, credentials, `Connect`). The pump dispatches pending
    /// GPUI foreground tasks, which re-borrow the App. Running any of those
    /// calls while holding the App borrow therefore panics with "RefCell
    /// already borrowed" (and starves `on_request_frame`, leaving the window
    /// blank). The phases alternate accordingly:
    ///
    /// * Phase 1 (App borrow held): pure-Rust snapshot of the inputs.
    /// * Phase 2 (no borrow): ActiveX host creation and preparation.
    /// * Phase 3 (App borrow held): shutdown registration and failure
    ///   routing. Registration stays ahead of `Connect` so app quit still
    ///   drains a connecting session.
    /// * Phase 4 (no borrow): `Connect`.
    /// * Phase 5 (App borrow held): attach only Rust ownership/state to the
    ///   view. Native activation remains outside this borrowed phase.
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn schedule_windows_native_presentation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.windows_native_initialization_generation.is_some() {
            return;
        }
        // Same layout precondition as the deferred initialization below.
        if self.options.backend_preference
            != one_core::storage::RemoteDesktopBackendPreference::Canvas
            && self
                .content_bounds
                .and_then(|bounds| resize::resize_dimensions(bounds, window.scale_factor()))
                .is_none()
        {
            return;
        }

        let initialization_generation = next_windows_native_rdp_generation();
        self.windows_native_initialization_generation = Some(initialization_generation);
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            // Phase 1: snapshot the inputs with the App borrow held. `None`
            // means a fallback, failure, or retry path already ran.
            let inputs = match window_handle.update(cx, |_, window, app| {
                this.update(app, |this, _cx| {
                    this.prepare_windows_native_presentation(window, initialization_generation)
                })
            }) {
                Ok(Ok(inputs)) => inputs,
                Ok(Err(_)) | Err(_) => {
                    reset_windows_native_presentation_schedule(
                        &this,
                        initialization_generation,
                        cx,
                    );
                    return;
                }
            };
            let Some(inputs) = inputs else {
                reset_windows_native_presentation_schedule(&this, initialization_generation, cx);
                return;
            };

            // Phase 2: create and prepare the native host WITHOUT holding the
            // App borrow; see `prepare_windows_native_connection`.
            let prepared = prepare_windows_native_connection(inputs);

            // Phase 3: retain the move-only preparation result outside the
            // entity update. If dispatch is rejected, the payload remains in
            // this slot and can be closed without an App/entity borrow.
            let prepared_slot = Rc::new(RefCell::new(Some(prepared)));
            let prepared_for_admission = prepared_slot.clone();
            let admission = this.update(cx, |this, cx| {
                let prepared = prepared_for_admission.borrow_mut().take();
                prepared.map(|prepared| this.admit_windows_native_presentation(prepared, cx))
            });
            let admission = match admission {
                Ok(Some(admission)) => admission,
                Ok(None) => {
                    tracing::error!(
                        "Windows native RDP admission callback ran without its preparation payload"
                    );
                    reset_windows_native_presentation_schedule(
                        &this,
                        initialization_generation,
                        cx,
                    );
                    return;
                }
                Err(_) => {
                    if let Some(prepared) = prepared_slot.borrow_mut().take() {
                        cleanup_rejected_windows_native_preparation(prepared, cx).await;
                    }
                    reset_windows_native_presentation_schedule(
                        &this,
                        initialization_generation,
                        cx,
                    );
                    return;
                }
            };

            let (mut native, registration, connection_options) = match admission {
                WindowsNativeAdmission::Connect {
                    native,
                    registration,
                    connection_options,
                } => (native, registration, connection_options),
                WindowsNativeAdmission::Cleanup(cleanup) => {
                    let destroyed = cleanup_windows_native_initialization(
                        cleanup.native,
                        cleanup.registration,
                        cleanup.reason,
                        cx,
                    )
                    .await;
                    let _ = this.update(cx, |this, cx| {
                        this.complete_windows_native_initialization_cleanup(destroyed, cx);
                    });
                    reset_windows_native_presentation_schedule(
                        &this,
                        initialization_generation,
                        cx,
                    );
                    return;
                }
                WindowsNativeAdmission::Complete => {
                    reset_windows_native_presentation_schedule(
                        &this,
                        initialization_generation,
                        cx,
                    );
                    return;
                }
            };

            // Phase 4: connect WITHOUT holding the App borrow. The ActiveX
            // `Connect` call pumps COM messages that can dispatch pending
            // GPUI foreground tasks; the borrow must be released around it.
            if let Err(error) = native.connect(&connection_options) {
                let failure_detail =
                    format!("Windows native RDP diagnostic\nstage=connect\nerror={error:?}");
                let _ = this.update(cx, |this, cx| {
                    this.fail_windows_native_presentation(failure_detail);
                    cx.notify();
                });
                let detached_dispatch =
                    crate::windows_native_shutdown::mark_windows_native_rdp_detached_async(
                        registration,
                        cx,
                    );
                if detached_dispatch.was_rejected() {
                    tracing::error!(
                        token = registration.token(),
                        generation = registration.generation(),
                        "Windows native RDP connect failure ownership transfer was rejected"
                    );
                }
                let destroyed = cleanup_windows_native_initialization(
                    native,
                    Some(registration),
                    "connect",
                    cx,
                )
                .await;
                let _ = this.update(cx, |this, cx| {
                    this.complete_windows_native_initialization_cleanup(destroyed, cx);
                });
                reset_windows_native_presentation_schedule(&this, initialization_generation, cx);
                return;
            }

            // Phase 5: retain the connected host until both the Window and
            // entity update closures actually execute. A closed window,
            // released view, stale initialization state, or generation
            // mismatch returns the payload to the borrow-free cleanup path.
            let attach_slot = Rc::new(RefCell::new(Some((native, registration))));
            let attach_for_window = attach_slot.clone();
            let attach_result = window_handle.update(cx, |_, window, app| {
                let attach_for_entity = attach_for_window.clone();
                let scale_factor = window.scale_factor();
                this.update(app, |this, _cx| {
                    attach_for_entity
                        .borrow_mut()
                        .take()
                        .map(|(native, registration)| {
                            this.attach_windows_native_presentation(
                                native,
                                registration,
                                scale_factor,
                            )
                        })
                })
            });
            let rejected = match attach_result {
                Ok(Ok(Some(Ok(())))) => None,
                Ok(Ok(Some(Err(rejected)))) => Some(rejected),
                Ok(Ok(None)) => {
                    tracing::error!(
                        "Windows native RDP attach callback ran without its connected host"
                    );
                    attach_slot.borrow_mut().take()
                }
                Ok(Err(_)) | Err(_) => attach_slot.borrow_mut().take(),
            };
            if let Some((native, registration)) = rejected {
                let detached_dispatch =
                    crate::windows_native_shutdown::mark_windows_native_rdp_detached_async(
                        registration,
                        cx,
                    );
                if detached_dispatch.was_rejected() {
                    tracing::error!(
                        token = registration.token(),
                        generation = registration.generation(),
                        "Windows native RDP attach rejection ownership transfer was rejected"
                    );
                }
                cleanup_windows_native_initialization(native, Some(registration), "attach", cx)
                    .await;
            }
            reset_windows_native_presentation_schedule(&this, initialization_generation, cx);
        })
        .detach();
    }

    /// Phase 1 of the Windows native RDP initialization: gathers everything
    /// the borrow-free COM stages need, using only pure-Rust work (no COM,
    /// no window creation, no message pumping).
    ///
    /// Returns `None` when a terminal path already ran (selection fallback,
    /// proxy rejection, failure) or when initialization is no longer pending.
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn prepare_windows_native_presentation(
        &mut self,
        window: &mut Window,
        initialization_generation: u64,
    ) -> Option<WindowsNativePresentationInputs> {
        if self.windows_native_initialization_generation != Some(initialization_generation)
            || !matches!(
                self.presentation_initialization,
                presentation::RemoteDesktopPresentationInitialization::Pending
            )
        {
            return None;
        }
        if self.options.backend_preference
            != one_core::storage::RemoteDesktopBackendPreference::Canvas
            && self
                .content_bounds
                .and_then(|bounds| resize::resize_dimensions(bounds, window.scale_factor()))
                .is_none()
        {
            return None;
        }

        // Selection is pure Rust: platform, preference, and capability probe
        // results, with no COM calls.
        let selection = match presentation::select_remote_desktop_presentation(
            presentation::current_remote_desktop_platform(),
            self.options.backend_preference,
            presentation_capability::current_windows_native_rdp_capability(),
        ) {
            Ok(selection) => selection,
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "failed to select the Windows native RDP presentation"
                );
                self.fail_presentation_initialization(
                    presentation::RemoteDesktopPresentation::NativeWindows,
                    true,
                    Some(format!(
                        "Windows native RDP diagnostic\nstage=selection\nerror={error:?}"
                    )),
                );
                return None;
            }
        };
        if matches!(
            selection.presentation,
            presentation::RemoteDesktopPresentation::Canvas
        ) {
            self.presentation_initialization =
                presentation::RemoteDesktopPresentationInitialization::Canvas {
                    fallback_reason: selection.fallback_reason,
                };
            return None;
        }

        // Windows native RDP cannot tunnel through SOCKS/HTTP proxies; fail
        // closed rather than leaking the connection around the proxy.
        if self.options.proxy.is_some() {
            tracing::warn!("Windows native RDP cannot use the configured SOCKS/HTTP proxy");
            self.fail_presentation_initialization(
                presentation::RemoteDesktopPresentation::NativeWindows,
                true,
                Some(
                    "Windows native RDP diagnostic\nstage=proxy\nerror=SOCKS/HTTP proxy is unsupported by Windows native RDP"
                        .to_owned(),
                ),
            );
            self.status =
                SharedString::from(t!("RemoteDesktop.native_proxy_unsupported").to_string());
            return None;
        }

        let Some(bounds) = self.content_bounds else {
            // Layout disappeared between scheduling and now; leave the state
            // pending so the next render reschedules.
            return None;
        };
        let Some(size) = resize::resize_dimensions(bounds, window.scale_factor()) else {
            return None;
        };
        Some(WindowsNativePresentationInputs {
            owner: windows_native::WindowsNativeAdapter::parent_window_owner(window),
            generation: initialization_generation,
            bounds,
            scale_factor: window.scale_factor(),
            desktop_size: windows_native_policy::initial_desktop_size(&self.options.rdp, size),
            options: self.options.clone(),
        })
    }

    /// Phase 3 of the Windows native RDP initialization: registers the
    /// prepared host for shutdown drain (before `Connect` runs) and routes
    /// preparation failures to their terminal paths.
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn admit_windows_native_presentation(
        &mut self,
        prepared: WindowsNativePreparedConnection,
        cx: &mut Context<Self>,
    ) -> WindowsNativeAdmission {
        let (native, connection_options) = match prepared {
            Ok(prepared) => prepared,
            Err(WindowsNativePrepareFailure::Create(error)) => {
                // Format the diagnostic before consuming `error` by value for
                // the Canvas fallback classification below.
                let failure_detail =
                    format!("Windows native RDP diagnostic\nstage=create\nerror={error}");
                tracing::warn!(
                    error = %error,
                    ?error,
                    "failed to create the Windows native RDP host"
                );
                let fallback_reason = match error {
                    windows_native::WindowsNativeAdapterCreateError::Host(host_error)
                        if self.options.backend_preference
                            == one_core::storage::RemoteDesktopBackendPreference::Auto =>
                    {
                        presentation::classify_windows_native_create_error(host_error)
                    }
                    _ => None,
                };
                match fallback_reason {
                    Some(reason) => {
                        self.presentation_initialization =
                            presentation::RemoteDesktopPresentationInitialization::Canvas {
                                fallback_reason: Some(reason),
                            };
                    }
                    None => {
                        self.fail_presentation_initialization(
                            presentation::RemoteDesktopPresentation::NativeWindows,
                            true,
                            Some(failure_detail),
                        );
                    }
                }
                return WindowsNativeAdmission::Complete;
            }
            Err(WindowsNativePrepareFailure::Bounds { native, error }) => {
                return WindowsNativeAdmission::Cleanup(
                    self.fail_unregistered_windows_native_presentation(native, "bounds", error),
                );
            }
            Err(WindowsNativePrepareFailure::Endpoint { native, error }) => {
                return WindowsNativeAdmission::Cleanup(
                    self.fail_unregistered_windows_native_presentation(native, "endpoint", error),
                );
            }
            Err(WindowsNativePrepareFailure::SharedFoldersUnsupported { native, count }) => {
                tracing::warn!(
                    shared_folder_count = count,
                    "Windows native RDP does not transport shared-folder redirection"
                );
                if self.options.backend_preference
                    == one_core::storage::RemoteDesktopBackendPreference::Auto
                {
                    self.presentation_initialization =
                        presentation::RemoteDesktopPresentationInitialization::Canvas {
                            fallback_reason: Some(
                                presentation::WindowsNativeRdpUnavailableReason::SharedFoldersUnsupported,
                            ),
                        };
                } else {
                    self.fail_windows_native_presentation(format!(
                        "Windows native RDP diagnostic\nstage=shared-folders\nerror={count} requested shared folders are unsupported"
                    ));
                    self.status = SharedString::from(
                        t!("RemoteDesktop.native_shared_folders_unsupported").to_string(),
                    );
                }
                return WindowsNativeAdmission::Cleanup(WindowsNativeInitializationCleanup {
                    native,
                    registration: None,
                    reason: "shared-folders",
                });
            }
            Err(WindowsNativePrepareFailure::ConnectionOptions { native, error }) => {
                return WindowsNativeAdmission::Cleanup(
                    self.fail_unregistered_windows_native_presentation(
                        native,
                        "connection-options",
                        error,
                    ),
                );
            }
            Err(WindowsNativePrepareFailure::Credentials { native, error }) => {
                return WindowsNativeAdmission::Cleanup(
                    self.fail_unregistered_windows_native_presentation(
                        native,
                        "credentials",
                        error,
                    ),
                );
            }
        };

        let registration = match crate::windows_native_shutdown::register_windows_native_rdp(
            cx.entity().downgrade(),
            native.generation(),
            cx,
        ) {
            Ok(registration) => registration,
            Err(error @ windows_rdp_host::WindowsRdpRegistrationError::AdmissionClosed)
            | Err(error @ windows_rdp_host::WindowsRdpRegistrationError::TokenExhausted) => {
                return WindowsNativeAdmission::Cleanup(
                    self.fail_unregistered_windows_native_presentation(
                        native,
                        "shutdown-admission",
                        error,
                    ),
                );
            }
        };

        // Phase 4 releases the App borrow before `Connect`. Phase 5 re-borrows
        // only to install Rust ownership/state and must not call the native
        // host or pump Win32 messages.
        WindowsNativeAdmission::Connect {
            native,
            registration,
            connection_options,
        }
    }

    fn fail_presentation_initialization(
        &mut self,
        attempted_presentation: presentation::RemoteDesktopPresentation,
        canvas_retry_available: bool,
        failure_detail: Option<String>,
    ) {
        self.presentation_initialization =
            presentation::RemoteDesktopPresentationInitialization::Failed {
                attempted_presentation,
                canvas_retry_available,
            };
        self.status = SharedString::from(t!("RemoteDesktop.failure_generic").to_string());
        self.failure_detail = failure_detail.map(SharedString::from);
    }

    fn use_canvas(&mut self, _: &UseCanvas, _window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .presentation_initialization
            .allows_explicit_canvas_retry()
        {
            return;
        }

        #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
        {
            if self.windows_native.is_some() || self.windows_native_close_in_flight.is_some() {
                tracing::warn!(
                    "refusing Canvas retry while a Windows native RDP child is still attached"
                );
                return;
            }
            self.windows_native_display.reset();
        }

        close_runtime_once(&mut self.input_tx);
        self.output_rx = None;
        self.presentation_initialization =
            presentation::RemoteDesktopPresentationInitialization::Canvas {
                fallback_reason: None,
            };
        self.status = SharedString::from(t!("RemoteDesktop.status_waiting_layout").to_string());
        self.failure_detail = None;
        cx.notify();
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn fail_windows_native_presentation(&mut self, failure_detail: String) {
        self.windows_native_display.reset();
        self.fail_presentation_initialization(
            presentation::RemoteDesktopPresentation::NativeWindows,
            false,
            Some(failure_detail),
        );
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn fail_unregistered_windows_native_presentation(
        &mut self,
        native: windows_native::WindowsNativeAdapter,
        stage: &'static str,
        error: impl std::fmt::Debug,
    ) -> WindowsNativeInitializationCleanup {
        let failure_detail =
            format!("Windows native RDP diagnostic\nstage={stage}\nerror={error:?}");
        tracing::warn!(
            ?error,
            stage,
            "Windows native RDP shutdown admission rejected the created host"
        );
        self.fail_windows_native_presentation(failure_detail);
        WindowsNativeInitializationCleanup {
            native,
            registration: None,
            reason: stage,
        }
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn complete_windows_native_initialization_cleanup(
        &mut self,
        destroyed: bool,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            &self.presentation_initialization,
            presentation::RemoteDesktopPresentationInitialization::Failed {
                attempted_presentation: presentation::RemoteDesktopPresentation::NativeWindows,
                ..
            }
        ) {
            self.presentation_initialization =
                presentation::RemoteDesktopPresentationInitialization::Failed {
                    attempted_presentation: presentation::RemoteDesktopPresentation::NativeWindows,
                    canvas_retry_available: destroyed,
                };
            cx.notify();
        }
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn windows_native_maintenance_is_pending(&self) -> bool {
        !self.windows_native_close_requested
            && self.windows_native_operation_in_flight.is_none()
            && self.windows_native.is_some()
            && self.windows_native_registration.is_some()
            && self.native_event_state.is_some()
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn take_windows_native_operation(&mut self) -> Option<WindowsNativeOperation> {
        if self.windows_native_close_requested || self.windows_native_operation_in_flight.is_some() {
            return None;
        }
        let registration = self.windows_native_registration?;
        let generation = registration.generation();
        if !self
            .windows_native
            .as_ref()
            .is_some_and(|native| native.generation() == generation)
        {
            return None;
        }
        let event_state = self.native_event_state.take()?;
        let native = self
            .windows_native
            .take()
            .expect("validated Windows native RDP owner");
        let was_presentation_ready = native.presentation_ready();
        let token = WindowsNativeOperationToken {
            generation,
            serial: self.next_windows_native_operation_serial,
        };
        self.next_windows_native_operation_serial =
            self.next_windows_native_operation_serial.wrapping_add(1).max(1);
        self.windows_native_operation_in_flight = Some(token);

        Some(WindowsNativeOperation {
            token,
            registration,
            native,
            event_state,
            bounds: self.pending_windows_native_bounds.take(),
            tab_active: self.tab_active,
            allow_activation: self.tab_active && self.connected && self.native_login_complete,
            lifecycle_dirty: std::mem::take(&mut self.windows_native_lifecycle_dirty),
            focus_requested: std::mem::take(&mut self.windows_native_focus_requested),
            display_request: self
                .windows_native_display
                .take_request(Instant::now()),
            was_presentation_ready,
        })
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn commit_windows_native_operation(
        &mut self,
        result: WindowsNativeOperationResult,
        cx: &mut Context<Self>,
    ) -> WindowsNativeOperationCommitResult {
        let WindowsNativeOperationResult {
            operation,
            effects,
            requested_focus,
            display_completion,
            readiness,
        } = result;
        let applied_scale_factor = operation.bounds.map(|(_, scale_factor)| scale_factor);
        let owner_matches = self.windows_native_operation_in_flight == Some(operation.token)
            && self.windows_native_registration == Some(operation.registration)
            && operation.registration.generation() == operation.token.generation
            && readiness.generation == operation.token.generation
            && self.windows_native.is_none()
            && self.native_event_state.is_none();
        if !owner_matches {
            if self.windows_native_operation_in_flight == Some(operation.token) {
                self.windows_native_operation_in_flight = None;
            }
            return WindowsNativeOperationCommitResult::Rejected(operation);
        }

        for effect in effects {
            if let Some(request) = native_events::notification_request(&effect) {
                self.pending_windows_native_notifications.push(request);
            }
            self.apply_windows_native_ui_effect(effect);
        }
        match display_completion {
            Some(WindowsNativeDisplayCompletion::Succeeded(request)) => {
                self.windows_native_display.request_succeeded(request);
            }
            Some(WindowsNativeDisplayCompletion::Failed {
                request,
                completed_at,
            }) => {
                self.windows_native_display
                    .request_failed(request, completed_at);
            }
            Some(WindowsNativeDisplayCompletion::Suspended(request)) => {
                if request.generation == operation.token.generation {
                    self.windows_native_display.suspend();
                }
            }
            None => {}
        }

        let generation = operation.token.generation;
        self.windows_native_operation_in_flight = None;
        self.native_event_state = Some(operation.event_state);
        self.windows_native = Some(operation.native);
        if let Some(scale_factor) = applied_scale_factor {
            self.windows_native_scale_factor = Some(scale_factor);
        }

        if readiness.transitioned {
            tracing::info!(generation, "Windows native RDP presentation ready");
        }
        tracing::trace!(
            generation = readiness.generation,
            requested_visible = readiness.requested_visible,
            activation_pending = readiness.activation_pending,
            can_present = readiness.can_present,
            presentation_ready = readiness.presentation_ready,
            native_login_complete = self.native_login_complete,
            tab_active = self.tab_active,
            "Windows native RDP presentation readiness"
        );

        let notifications = std::mem::take(&mut self.pending_windows_native_notifications)
            .into_iter()
            .filter(|request| request.generation() == generation)
            .collect();
        let focus_handle = if self.tab_active
            && requested_focus == Some(WindowsNativeFocusTarget::Parent)
        {
            Some(self.focus_handle.clone())
        } else {
            None
        };
        cx.notify();
        WindowsNativeOperationCommitResult::Attached(WindowsNativeOperationCommit {
            focus_handle,
            notifications,
            entity_id: cx.entity_id(),
        })
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    pub(crate) fn attach_windows_native_presentation(
        &mut self,
        presentation: windows_native::WindowsNativeAdapter,
        registration: windows_rdp_host::WindowsRdpRegistration,
        scale_factor: f32,
    ) -> Result<
        (),
        (
            windows_native::WindowsNativeAdapter,
            windows_rdp_host::WindowsRdpRegistration,
        ),
    > {
        if self.windows_native_initialization_generation != Some(presentation.generation())
            || !matches!(
                self.presentation_initialization,
                presentation::RemoteDesktopPresentationInitialization::Pending
            )
            || registration.generation() != presentation.generation()
            || self.windows_native.is_some()
            || self.windows_native_registration.is_some()
            || self.native_event_state.is_some()
            || self.windows_native_operation_in_flight.is_some()
            || self.windows_native_close_in_flight.is_some()
        {
            return Err((presentation, registration));
        }
        let generation = presentation.generation();
        self.windows_native_display.attach(generation);
        self.native_event_state = Some(native_events::NativeRdpEventState::new(generation));
        self.pending_windows_native_notifications.clear();
        self.windows_native_registration = Some(registration);
        self.windows_native = Some(presentation);
        if let Some(bounds) = self.content_bounds {
            self.observe_windows_native_viewport(bounds, scale_factor);
            self.pending_windows_native_bounds = Some((bounds, scale_factor));
        }
        // Do not activate, focus, resize, or otherwise enter the native
        // COM/Win32 path while the entity is borrowed. LoginComplete and
        // Reconnected perform presentation activation after attachment.
        self.presentation_initialization =
            presentation::RemoteDesktopPresentationInitialization::Native;
        Ok(())
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn apply_windows_native_ui_effect(
        &mut self,
        effect: native_events::NativeRdpUiEffect,
    ) -> Option<WindowsNativeFocusTarget> {
        use native_events::NativeRdpUiEffect;

        let diagnostic = native_events::diagnostic_text(&effect).map(SharedString::from);
        match effect {
            NativeRdpUiEffect::CloseConfirmed => {}
            NativeRdpUiEffect::FocusReleased => {
                return Some(WindowsNativeFocusTarget::Parent);
            }
            NativeRdpUiEffect::Connecting { generation } => {
                tracing::info!(generation, "Windows native RDP is connecting");
                self.connected = false;
                self.failure_detail = None;
                self.status = SharedString::from(t!("RemoteDesktop.status_connecting").to_string());
            }
            NativeRdpUiEffect::Connected { generation } => {
                tracing::info!(generation, "Windows native RDP connected");
                self.mark_windows_native_connected();
            }
            NativeRdpUiEffect::LoginComplete { generation } => {
                tracing::info!(generation, "Windows native RDP login completed");
                self.mark_windows_native_connected();
                self.native_login_complete = true;
                self.windows_native_display
                    .login_complete(generation, Instant::now());
            }
            NativeRdpUiEffect::Reconnecting {
                generation,
                attempt,
                max_attempts,
            } => {
                tracing::warn!(
                    generation,
                    attempt,
                    ?max_attempts,
                    "Windows native RDP is reconnecting"
                );
                self.connected = false;
                self.failure_detail = None;
                self.status = SharedString::from(t!("RemoteDesktop.status_connecting").to_string());
                self.native_login_complete = false;
                self.windows_native_display.reconnecting(generation);
            }
            NativeRdpUiEffect::Reconnected { generation } => {
                tracing::info!(generation, "Windows native RDP reconnected");
                self.mark_windows_native_connected();
                self.native_login_complete = true;
                self.windows_native_display
                    .reconnected(generation, Instant::now());
                if self.tab_active {
                    return Some(WindowsNativeFocusTarget::NativeChild);
                }
            }
            NativeRdpUiEffect::Warning {
                generation,
                warning,
            } => {
                tracing::warn!(
                    generation,
                    kind = ?warning.kind(),
                    code = warning.code(),
                    "Windows native RDP warning"
                );
            }
            NativeRdpUiEffect::FatalError { generation, error } => {
                tracing::error!(
                    generation,
                    kind = ?error.kind(),
                    code = error.code(),
                    "Windows native RDP fatal error"
                );
                self.show_windows_native_failure(diagnostic);
            }
            NativeRdpUiEffect::LogonError { generation, error } => {
                tracing::error!(
                    generation,
                    kind = ?error.kind(),
                    code = error.code(),
                    "Windows native RDP logon error"
                );
                self.show_windows_native_failure(diagnostic);
            }
            NativeRdpUiEffect::Disconnected { generation, reason } => {
                self.windows_native_display.reset();
                tracing::warn!(
                    generation,
                    category = ?reason.category(),
                    disconnect_code = reason.disconnect_code(),
                    extended_code = ?reason.extended_code(),
                    "Windows native RDP disconnected"
                );
                if reason.category()
                    != windows_rdp_host::WindowsRdpDiagnosticCategory::UserInitiated
                {
                    self.show_windows_native_failure(diagnostic);
                }
            }
            NativeRdpUiEffect::Unknown { event } => {
                tracing::warn!(
                    generation = event.generation,
                    kind = event.kind,
                    code = event.code,
                    payload_len = event.payload.len(),
                    "unknown or malformed Windows native RDP event"
                );
            }
        }
        None
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn mark_windows_native_connected(&mut self) {
        self.connected = true;
        self.failure_detail = None;
        self.status = SharedString::from(t!("RemoteDesktop.status_connected").to_string());
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn show_windows_native_failure(&mut self, diagnostic: Option<SharedString>) {
        self.windows_native_display.reset();
        self.connected = false;
        self.native_login_complete = false;
        self.status = SharedString::from(t!("RemoteDesktop.failure_generic").to_string());
        self.failure_detail = diagnostic;
    }

    /// Borrow-held phase of the close state machine: only pure-Rust ownership
    /// transfer and state checks. Never calls into the native adapter, whose
    /// COM/Win32 calls pump messages and must run borrow-free.
    ///
    /// Atomically moves `windows_native`, `native_event_state` and
    /// `windows_native_registration` into the returned operation once no
    /// maintenance operation temporarily owns the adapter.
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    pub(crate) fn take_windows_native_close_operation(
        &mut self,
        registration: windows_rdp_host::WindowsRdpRegistration,
    ) -> WindowsNativeCloseTake {
        if self.windows_native.is_none()
            && self.native_event_state.is_none()
            && self.windows_native_registration.is_none()
        {
            return WindowsNativeCloseTake::Closed;
        }
        // Compare the complete registration (token + generation): an old close
        // task must never consume a newer owner that reused a generation.
        if self.windows_native_registration != Some(registration) {
            return WindowsNativeCloseTake::Failed;
        }

        self.windows_native_close_requested = true;
        self.windows_native_close_in_flight = Some(registration);
        if self.windows_native_operation_in_flight.is_some() {
            // A maintenance operation owns the adapter right now; its commit
            // returns ownership, after which a retry of this take succeeds.
            return WindowsNativeCloseTake::Pending;
        }
        let Some(native) = self.windows_native.take() else {
            return WindowsNativeCloseTake::Failed;
        };
        let Some(event_state) = self.native_event_state.take() else {
            self.windows_native = Some(native);
            return WindowsNativeCloseTake::Failed;
        };
        if native.generation() != registration.generation()
            || event_state.generation() != registration.generation()
        {
            tracing::error!(
                token = registration.token(),
                registration_generation = registration.generation(),
                adapter_generation = native.generation(),
                event_state_generation = event_state.generation(),
                "Windows native RDP close take found inconsistent owner generations"
            );
            self.windows_native = Some(native);
            self.native_event_state = Some(event_state);
            return WindowsNativeCloseTake::Failed;
        }

        self.windows_native_registration = None;
        self.windows_native_display.reset();
        self.windows_native_lifecycle_dirty = false;
        self.windows_native_focus_requested = false;
        self.pending_windows_native_bounds = None;
        WindowsNativeCloseTake::Ready(WindowsNativeCloseOperation {
            registration,
            native,
            event_state,
        })
    }

    /// Pure-Rust commit after a close operation reached its terminal outcome:
    /// clears the matching in-flight marker so a fresh attach is admitted again.
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    pub(crate) fn finish_windows_native_close_in_view(
        this: &WeakEntity<RemoteDesktopView>,
        registration: windows_rdp_host::WindowsRdpRegistration,
        cx: &mut gpui::AsyncApp,
    ) {
        let _ = this.update(cx, |this, cx| {
            if this.windows_native_close_in_flight == Some(registration) {
                this.windows_native_close_in_flight = None;
            }
            cx.notify();
        });
    }

}

/// Borrow-free close runner: owns the adapter for the whole close and is the
/// only place (besides the initialization/detached cleanup runners) allowed to
/// call `begin_close` / `close_confirmed` / `finish_destroy` / `force_close`.
///
/// `hard_deadline` caps the total close budget (the shutdown drain passes its
/// own deadline). On timeout the adapter is intentionally leaked so pending COM
/// callbacks never run into a dropped host, and the registration is terminally
/// recorded as `TimedOutLeaked`.
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
pub(crate) async fn close_windows_native_operation(
    this: &WeakEntity<RemoteDesktopView>,
    mut operation: WindowsNativeCloseOperation,
    initial_mode: WindowsNativeCloseRetryMode,
    hard_deadline: Instant,
    cx: &mut gpui::AsyncApp,
) -> bool {
    let registration = operation.registration;
    let generation = registration.generation();
    let started_at = Instant::now();
    let graceful_deadline = match initial_mode {
        WindowsNativeCloseRetryMode::WaitForConfirmation => {
            started_at + WINDOWS_NATIVE_CLOSE_TIMEOUT
        }
        WindowsNativeCloseRetryMode::ForceClose => hard_deadline,
    };
    let hard_deadline = hard_deadline.min(match initial_mode {
        WindowsNativeCloseRetryMode::WaitForConfirmation => {
            graceful_deadline + WINDOWS_NATIVE_FORCE_CLOSE_TIMEOUT
        }
        WindowsNativeCloseRetryMode::ForceClose => hard_deadline,
    });
    let mut mode = initial_mode;

    loop {
        let now = Instant::now();
        if now >= hard_deadline {
            tracing::error!(
                generation,
                "timed out waiting for Windows native RDP callback quiescence"
            );
            let _ = Box::leak(Box::new(operation.native));
            record_detached_windows_native_terminal(
                registration,
                windows_rdp_host::WindowsRdpTerminalOutcome::TimedOutLeaked,
                generation,
                "close-timeout",
                cx,
            );
            RemoteDesktopView::finish_windows_native_close_in_view(this, registration, cx);
            return false;
        }
        if matches!(mode, WindowsNativeCloseRetryMode::WaitForConfirmation)
            && now >= graceful_deadline
        {
            mode = WindowsNativeCloseRetryMode::ForceClose;
        }

        // Feed native events (including CloseConfirmed) into the state machine
        // while the maintenance loop is stopped by the close request.
        operation.native.drain_events(&mut operation.event_state);

        let mut switch_to_force = false;
        let progress = match mode {
            WindowsNativeCloseRetryMode::WaitForConfirmation => {
                match operation.native.begin_close(&mut || {}) {
                    Ok(windows_native::NativeCloseProgress::Ready) => {
                        operation.native.finish_destroy()
                    }
                    Ok(windows_native::NativeCloseProgress::WaitingForEvents {
                        generation: close_generation,
                    }) => {
                        if close_generation != generation {
                            tracing::error!(
                                token = registration.token(),
                                registration_generation = generation,
                                close_generation,
                                "Windows native RDP close generation changed unexpectedly"
                            );
                            switch_to_force = true;
                            Ok(windows_native::NativeDestroyProgress::PendingCallbacks)
                        } else if operation
                            .native
                            .close_confirmed(&mut operation.event_state)
                        {
                            operation.native.finish_destroy()
                        } else {
                            Ok(windows_native::NativeDestroyProgress::PendingCallbacks)
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            ?error,
                            "failed to request graceful Windows native RDP close"
                        );
                        switch_to_force = true;
                        Ok(windows_native::NativeDestroyProgress::PendingCallbacks)
                    }
                }
            }
            WindowsNativeCloseRetryMode::ForceClose => {
                operation.native.force_close(&mut || {})
            }
        };
        if switch_to_force {
            mode = WindowsNativeCloseRetryMode::ForceClose;
        }

        let destroyed = match progress {
            Ok(windows_native::NativeDestroyProgress::Destroyed) => true,
            Ok(windows_native::NativeDestroyProgress::PendingCallbacks) => false,
            Err(error) if operation.native.is_destroyed() => {
                tracing::warn!(
                    ?error,
                    generation,
                    "Windows native RDP close completed through an error path"
                );
                true
            }
            Err(error) => {
                tracing::warn!(
                    ?error,
                    generation,
                    "failed to destroy Windows native RDP; retrying the close"
                );
                if matches!(mode, WindowsNativeCloseRetryMode::WaitForConfirmation) {
                    mode = WindowsNativeCloseRetryMode::ForceClose;
                }
                false
            }
        };
        if destroyed {
            record_detached_windows_native_terminal(
                registration,
                windows_rdp_host::WindowsRdpTerminalOutcome::Destroyed,
                generation,
                "close",
                cx,
            );
            RemoteDesktopView::finish_windows_native_close_in_view(this, registration, cx);
            return true;
        }

        let deadline = crate::windows_native_shutdown::detached_cleanup_deadline(
            hard_deadline,
            cx,
        );
        if Instant::now() >= deadline {
            continue;
        }
        cx.background_executor()
            .timer(WINDOWS_NATIVE_EVENT_POLL_INTERVAL)
            .await;
    }
}

fn remote_desktop_diagnostics_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os(REMOTE_DESKTOP_DIAGNOSTICS_ENV).is_some())
}

fn close_runtime_once(
    input_tx: &mut Option<tokio::sync::mpsc::UnboundedSender<RemoteDesktopInput>>,
) {
    if let Some(input_tx) = input_tx.take() {
        let _ = input_tx.send(RemoteDesktopInput::Close);
    }
}

fn failed_runtime(error: anyhow::Error) -> RemoteDesktopRuntime {
    tracing::warn!(?error, "failed to create remote desktop backend");
    let (input_tx, _input_rx) = tokio::sync::mpsc::unbounded_channel();
    let (output_tx, output_rx) = remote_desktop::output_mailbox::output_mailbox();
    let _ = output_tx.send(RemoteDesktopOutput::ConnectionFailure(
        remote_desktop_failure(&error),
    ));
    RemoteDesktopRuntime {
        input_tx,
        output_rx,
    }
}

fn remote_desktop_failure(error: &anyhow::Error) -> RemoteDesktopFailure {
    if let Some(error) = error.downcast_ref::<RemoteDesktopProviderVersionError>() {
        return RemoteDesktopFailure::ProviderVersion {
            protocol: error.protocol,
            installed: error.installed.clone(),
            required: error.required.clone(),
            invalid: error.invalid,
        };
    }
    RemoteDesktopFailure::ConnectionFailed
}

pub fn init(cx: &mut App) {
    crate::windows_native_shutdown::init(cx);
    cx.bind_keys([
        KeyBinding::new("tab", SendTab, Some(REMOTE_DESKTOP_CONTEXT)),
        KeyBinding::new("shift-tab", SendShiftTab, Some(REMOTE_DESKTOP_CONTEXT)),
        KeyBinding::new(
            REMOTE_COPY_SHORTCUT,
            RemoteCopy,
            Some(REMOTE_DESKTOP_CONTEXT),
        ),
        KeyBinding::new(
            REMOTE_PASTE_SHORTCUT,
            RemotePaste,
            Some(REMOTE_DESKTOP_CONTEXT),
        ),
    ]);
}

pub fn refresh_keybindings(_cx: &mut App) {}

#[cfg(test)]
#[path = "view/render_contract_tests.rs"]
mod render_contract_tests;

#[cfg(test)]
#[path = "view/presentation_tests.rs"]
mod presentation_tests;

#[cfg(test)]
#[path = "view/view_tests.rs"]
mod tests;
