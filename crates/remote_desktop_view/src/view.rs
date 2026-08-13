use std::sync::{Arc, OnceLock, atomic::AtomicU64};
use std::time::{Duration, Instant};

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
use std::sync::atomic::Ordering;

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

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(800);
const RESIZE_MIN_INTERVAL: Duration = Duration::from_millis(1200);
const RESIZE_DELTA_THRESHOLD: u16 = 16;
const RDP_INITIAL_LAYOUT_DEBOUNCE: Duration = Duration::from_millis(150);
const REMOTE_DESKTOP_CONTEXT: &str = "RemoteDesktopView";
const REMOTE_DESKTOP_DIAGNOSTICS_ENV: &str = "NAVOP_REMOTE_DESKTOP_DIAGNOSTICS";
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
const WINDOWS_NATIVE_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
const WINDOWS_NATIVE_EVENT_POLL_INTERVAL: Duration = Duration::from_millis(16);
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
const WINDOWS_NATIVE_FORCE_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
static NEXT_WINDOWS_NATIVE_RDP_GENERATION: AtomicU64 = AtomicU64::new(1);

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
enum WindowsNativeClosePoll {
    Pending,
    Closed,
    Failed,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowsNativeCloseRetryMode {
    WaitForConfirmation,
    ForceClose,
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
#[derive(Debug)]
enum WindowsNativePresentationCreateError {
    ProxyUnsupported,
    Adapter(windows_native::WindowsNativeAdapterCreateError),
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
fn detach_windows_native_cleanup(
    mut native: windows_native::WindowsNativeAdapter,
    registration: Option<windows_rdp_host::WindowsRdpRegistration>,
    cx: &App,
    reason: &'static str,
) {
    let generation = native.generation();
    let local_deadline = Instant::now() + WINDOWS_NATIVE_FORCE_CLOSE_TIMEOUT;
    cx.spawn(async move |cx| {
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
                    return;
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
                    return;
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
                return;
            }

            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
        }
    })
    .detach();
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
    windows_native_display: windows_native_display::WindowsNativeDisplayState,
}

impl RemoteDesktopView {
    pub fn new(
        config: RemoteDesktopViewConfig,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let manage_native_cursor = config.options.protocol == RemoteDesktopProtocol::Rdp;
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
                let focus_handle = match this.update(cx, |this, cx| {
                    let focus_handle = this.poll_windows_native_events();
                    this.flush_windows_native_display_settings(Instant::now());
                    cx.notify();
                    focus_handle
                }) {
                    Ok(focus_handle) => focus_handle,
                    Err(_) => break,
                };
                if let Some(focus_handle) = focus_handle {
                    let _ = native_event_window_handle.update(cx, |_, window, cx| {
                        window.focus(&focus_handle, cx);
                    });
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
            if let Some(mut native) = this.windows_native.take() {
                let registration = this.windows_native_registration.take();
                this.native_event_state.take();
                let focus_handle = this.focus_handle.clone();
                let _ = window_handle.update(cx, |_, window, cx| {
                    window.focus(&focus_handle, cx);
                });
                let mut focus_parent = || {};
                let destroyed = match native.force_close(&mut focus_parent) {
                    Ok(windows_native::NativeDestroyProgress::Destroyed) => true,
                    Ok(windows_native::NativeDestroyProgress::PendingCallbacks) => false,
                    Err(error) => {
                        tracing::warn!(
                            ?error,
                            "failed to force-close Windows native RDP during view release"
                        );
                        native.is_destroyed()
                    }
                };
                if destroyed {
                    if let Some(registration) = registration {
                        crate::windows_native_shutdown::record_windows_native_rdp_terminal(
                            registration,
                            windows_rdp_host::WindowsRdpTerminalOutcome::Destroyed,
                            cx,
                        );
                    } else {
                        tracing::error!(
                            "destroyed Windows native RDP adapter had no shutdown registration"
                        );
                    }
                } else {
                    if let Some(registration) = registration {
                        crate::windows_native_shutdown::mark_windows_native_rdp_detached(
                            registration,
                            cx,
                        );
                    }
                    detach_windows_native_cleanup(native, registration, cx, "view release");
                }
            } else if let Some(registration) = this.windows_native_registration.take() {
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
            tab_active: false,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native: None,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native_registration: None,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            native_event_state: None,
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native_display: Default::default(),
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
            self.windows_native.is_some()
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
            self.ensure_windows_native_presentation(window, cx);
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

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn ensure_windows_native_presentation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.options.backend_preference
            != one_core::storage::RemoteDesktopBackendPreference::Canvas
            && self
                .content_bounds
                .and_then(|bounds| resize::resize_dimensions(bounds, window.scale_factor()))
                .is_none()
        {
            return;
        }

        let proxy_configured = self.options.proxy.is_some();
        let creation = presentation::create_remote_desktop_presentation_with(
            presentation::current_remote_desktop_platform(),
            self.options.backend_preference,
            presentation_capability::current_windows_native_rdp_capability,
            || {
                if proxy_configured {
                    return Err(WindowsNativePresentationCreateError::ProxyUnsupported);
                }
                windows_native::WindowsNativeAdapter::create(
                    window,
                    next_windows_native_rdp_generation(),
                )
                .map_err(WindowsNativePresentationCreateError::Adapter)
            },
            |error| match error {
                WindowsNativePresentationCreateError::ProxyUnsupported => None,
                WindowsNativePresentationCreateError::Adapter(
                    windows_native::WindowsNativeAdapterCreateError::Host(error),
                ) => presentation::classify_windows_native_create_error(*error),
                WindowsNativePresentationCreateError::Adapter(
                    windows_native::WindowsNativeAdapterCreateError::WindowHandle(_)
                    | windows_native::WindowsNativeAdapterCreateError::ParentHandleNotWin32,
                ) => None,
            },
        );

        let mut native = match creation {
            Ok(presentation::RemoteDesktopPresentationCreation::Canvas { fallback_reason }) => {
                self.presentation_initialization =
                    presentation::RemoteDesktopPresentationInitialization::Canvas {
                        fallback_reason,
                    };
                return;
            }
            Ok(presentation::RemoteDesktopPresentationCreation::Native(native)) => native,
            Err(presentation::RemoteDesktopPresentationCreateError::Selection(error)) => {
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
                return;
            }
            Err(presentation::RemoteDesktopPresentationCreateError::NativeCreate(
                WindowsNativePresentationCreateError::ProxyUnsupported,
            )) => {
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
                return;
            }
            Err(presentation::RemoteDesktopPresentationCreateError::NativeCreate(
                WindowsNativePresentationCreateError::Adapter(error),
            )) => {
                tracing::warn!(
                    error = %error,
                    ?error,
                    "failed to create the Windows native RDP host"
                );
                self.fail_presentation_initialization(
                    presentation::RemoteDesktopPresentation::NativeWindows,
                    true,
                    Some(format!(
                        "Windows native RDP diagnostic\nstage=create\nerror={error}"
                    )),
                );
                return;
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
                self.fail_unregistered_windows_native_presentation(
                    native,
                    "shutdown-admission",
                    error,
                    cx,
                );
                return;
            }
        };

        let Some(bounds) = self.content_bounds else {
            self.fail_windows_native_presentation(
                native,
                registration,
                "layout",
                anyhow::anyhow!("native RDP layout disappeared before initialization"),
                cx,
            );
            return;
        };
        let Some(size) = resize::resize_dimensions(bounds, window.scale_factor()) else {
            self.fail_windows_native_presentation(
                native,
                registration,
                "layout",
                anyhow::anyhow!("native RDP layout became invalid during initialization"),
                cx,
            );
            return;
        };
        if let Err(error) =
            native.update_bounds(bounds, point(px(0.0), px(0.0)), window.scale_factor())
        {
            self.fail_windows_native_presentation(native, registration, "bounds", error, cx);
            return;
        }
        let (host, port) = match parse_destination(&self.options.destination) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                self.fail_windows_native_presentation(native, registration, "endpoint", error, cx);
                return;
            }
        };
        let connection_options = match windows_rdp_host::WindowsRdpConnectionOptions::new(
            host,
            port,
            u32::from(size.0),
            u32::from(size.1),
            windows_rdp_host::WindowsRdpColorDepth::Bpp32,
        ) {
            Ok(options) => options,
            Err(error) => {
                self.fail_windows_native_presentation(
                    native,
                    registration,
                    "connection-options",
                    error,
                    cx,
                );
                return;
            }
        };
        let mut credentials = windows_rdp_host::WindowsRdpCredentialBundle::new();
        if let Some(username) = self.options.username.as_ref() {
            credentials.set_username(username.clone());
        }
        if let Some(domain) = self.options.domain.as_ref() {
            credentials.set_domain(domain.clone());
        }
        if let Some(password) = self.options.password.as_ref() {
            credentials.set_server_password(password.clone());
        }
        if let Err(error) = native.apply_credentials(&credentials) {
            self.fail_windows_native_presentation(native, registration, "credentials", error, cx);
            return;
        }
        if let Err(error) = native.connect(&connection_options) {
            self.fail_windows_native_presentation(native, registration, "connect", error, cx);
            return;
        }

        self.attach_windows_native_presentation(native, registration, window, cx);
        self.presentation_initialization =
            presentation::RemoteDesktopPresentationInitialization::Native;
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
            if self.windows_native.is_some() {
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
    fn fail_windows_native_presentation(
        &mut self,
        mut native: windows_native::WindowsNativeAdapter,
        registration: windows_rdp_host::WindowsRdpRegistration,
        stage: &'static str,
        error: impl std::fmt::Debug,
        cx: &mut Context<Self>,
    ) {
        self.windows_native_display.reset();
        let failure_detail =
            format!("Windows native RDP diagnostic\nstage={stage}\nerror={error:?}");
        tracing::warn!(?error, stage, "failed to initialize Windows native RDP");
        let mut focus_parent = || {};
        let destroyed = match native.force_close(&mut focus_parent) {
            Ok(windows_native::NativeDestroyProgress::Destroyed) => true,
            Ok(windows_native::NativeDestroyProgress::PendingCallbacks) => false,
            Err(close_error) => {
                tracing::warn!(
                    ?close_error,
                    "failed to destroy Windows native RDP after initialization failure"
                );
                native.is_destroyed()
            }
        };
        let canvas_retry_available = destroyed;
        let needs_detached_cleanup = !destroyed;
        if destroyed {
            crate::windows_native_shutdown::record_windows_native_rdp_terminal(
                registration,
                windows_rdp_host::WindowsRdpTerminalOutcome::Destroyed,
                cx,
            );
        }
        if needs_detached_cleanup {
            crate::windows_native_shutdown::mark_windows_native_rdp_detached(registration, cx);
            detach_windows_native_cleanup(native, Some(registration), cx, stage);
        }
        self.fail_presentation_initialization(
            presentation::RemoteDesktopPresentation::NativeWindows,
            canvas_retry_available,
            Some(failure_detail),
        );
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn fail_unregistered_windows_native_presentation(
        &mut self,
        mut native: windows_native::WindowsNativeAdapter,
        stage: &'static str,
        error: impl std::fmt::Debug,
        cx: &mut Context<Self>,
    ) {
        self.windows_native_display.reset();
        let failure_detail =
            format!("Windows native RDP diagnostic\nstage={stage}\nerror={error:?}");
        tracing::warn!(
            ?error,
            stage,
            "Windows native RDP shutdown admission rejected the created host"
        );
        let mut focus_parent = || {};
        let destroyed = match native.force_close(&mut focus_parent) {
            Ok(windows_native::NativeDestroyProgress::Destroyed) => true,
            Ok(windows_native::NativeDestroyProgress::PendingCallbacks) => false,
            Err(close_error) => {
                tracing::warn!(
                    ?close_error,
                    "failed to destroy unregistered Windows native RDP host"
                );
                native.is_destroyed()
            }
        };
        if !destroyed {
            detach_windows_native_cleanup(native, None, cx, stage);
        }
        self.fail_presentation_initialization(
            presentation::RemoteDesktopPresentation::NativeWindows,
            destroyed,
            Some(failure_detail),
        );
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    pub(crate) fn attach_windows_native_presentation(
        &mut self,
        presentation: windows_native::WindowsNativeAdapter,
        registration: windows_rdp_host::WindowsRdpRegistration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        assert_eq!(
            registration.generation(),
            presentation.generation(),
            "Windows native RDP registration must match the attached adapter"
        );
        let generation = presentation.generation();
        self.windows_native_display.attach(generation);
        self.native_event_state = Some(native_events::NativeRdpEventState::new(generation));
        self.windows_native_registration = Some(registration);
        self.windows_native = Some(presentation);
        if let Some(bounds) = self.content_bounds {
            self.observe_windows_native_viewport(bounds, window.scale_factor());
        }
        if self.tab_active && self.activate_windows_native(false) {
            cx.defer_in(window, |this, _, _| {
                if this.tab_active {
                    this.focus_windows_native();
                }
            });
        }
    }

    pub(super) fn update_windows_native_bounds(
        &mut self,
        bounds: Bounds<Pixels>,
        display_scale_factor: f32,
    ) -> bool {
        #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
        if let Some(presentation) = self.windows_native.as_mut() {
            if let Err(error) =
                presentation.update_bounds(bounds, point(px(0.0), px(0.0)), display_scale_factor)
            {
                tracing::warn!(?error, "failed to update Windows native RDP bounds");
            }
            return true;
        }

        let _ = (bounds, display_scale_factor);
        false
    }

    pub(super) fn activate_windows_native(&mut self, focus_child: bool) -> bool {
        #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
        if let Some(presentation) = self.windows_native.as_mut() {
            if let Err(error) = presentation.activate(focus_child) {
                tracing::warn!(?error, "failed to activate Windows native RDP presentation");
            }
            return true;
        }

        let _ = focus_child;
        false
    }

    pub(super) fn focus_windows_native(&mut self) {
        #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
        if let Some(presentation) = self.windows_native.as_mut()
            && let Err(error) = presentation.focus()
        {
            tracing::warn!(?error, "failed to focus Windows native RDP presentation");
        }
    }

    pub(super) fn deactivate_windows_native(&mut self, mut focus_parent: impl FnMut()) -> bool {
        #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
        if let Some(presentation) = self.windows_native.as_mut() {
            if let Err(error) = presentation.deactivate(&mut focus_parent) {
                tracing::warn!(
                    ?error,
                    "failed to deactivate Windows native RDP presentation"
                );
            }
            return true;
        }

        let _ = &mut focus_parent;
        false
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn poll_windows_native_events(&mut self) -> Option<FocusHandle> {
        let effects = {
            let native = self.windows_native.as_ref()?;
            let event_state = self.native_event_state.as_mut()?;
            native.drain_events(event_state)
        };
        for effect in effects {
            self.apply_windows_native_ui_effect(effect);
        }
        let focus_release_pending = self
            .native_event_state
            .as_mut()
            .map(native_events::NativeRdpEventState::take_focus_release_pending)
            .unwrap_or(false);
        if focus_release_pending && self.tab_active {
            Some(self.focus_handle.clone())
        } else {
            None
        }
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn apply_windows_native_ui_effect(&mut self, effect: native_events::NativeRdpUiEffect) {
        use native_events::NativeRdpUiEffect;

        let diagnostic = native_events::diagnostic_text(&effect).map(SharedString::from);
        match effect {
            NativeRdpUiEffect::CloseConfirmed | NativeRdpUiEffect::FocusReleased => {}
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
                self.windows_native_display.reconnecting(generation);
            }
            NativeRdpUiEffect::Reconnected { generation } => {
                tracing::info!(generation, "Windows native RDP reconnected");
                self.mark_windows_native_connected();
                self.windows_native_display
                    .reconnected(generation, Instant::now());
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
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn mark_windows_native_connected(&mut self) {
        self.connected = true;
        self.failure_detail = None;
        self.status = SharedString::from(t!("RemoteDesktop.status_connected").to_string());
        if self.tab_active {
            self.activate_windows_native(false);
        }
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn show_windows_native_failure(&mut self, diagnostic: Option<SharedString>) {
        self.windows_native_display.reset();
        self.connected = false;
        self.status = SharedString::from(t!("RemoteDesktop.failure_generic").to_string());
        self.failure_detail = diagnostic;
        self.deactivate_windows_native(|| {});
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn windows_native_matches_registration(
        &self,
        registration: windows_rdp_host::WindowsRdpRegistration,
    ) -> bool {
        self.windows_native_registration == Some(registration)
            && self
                .windows_native
                .as_ref()
                .is_some_and(|native| native.generation() == registration.generation())
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn complete_destroyed_windows_native(
        &mut self,
        registration: windows_rdp_host::WindowsRdpRegistration,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.windows_native_matches_registration(registration) {
            return false;
        }
        debug_assert!(
            self.windows_native
                .as_ref()
                .is_some_and(windows_native::WindowsNativeAdapter::is_destroyed),
            "terminal completion requires confirmed native destruction"
        );
        self.windows_native_display.reset();
        self.windows_native.take();
        self.native_event_state.take();
        let stored_registration = self.windows_native_registration.take();
        debug_assert_eq!(stored_registration, Some(registration));
        crate::windows_native_shutdown::record_windows_native_rdp_terminal(
            registration,
            windows_rdp_host::WindowsRdpTerminalOutcome::Destroyed,
            cx,
        );
        true
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    pub(crate) fn force_close_windows_native_for_shutdown(
        &mut self,
        registration: windows_rdp_host::WindowsRdpRegistration,
        cx: &mut Context<Self>,
    ) {
        self.windows_native_display.reset();
        let _ = self.force_close_windows_native(registration, cx);
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    pub(crate) fn quarantine_windows_native_for_shutdown(
        &mut self,
        registration: windows_rdp_host::WindowsRdpRegistration,
        cx: &mut Context<Self>,
    ) -> bool {
        if !(self.windows_native_registration == Some(registration)
            && self
                .windows_native
                .as_ref()
                .is_some_and(|native| native.generation() == registration.generation()))
        {
            return false;
        }

        let native = self
            .windows_native
            .take()
            .expect("matching registration must own an adapter");
        self.windows_native_display.reset();
        self.native_event_state.take();
        let stored_registration = self.windows_native_registration.take();
        debug_assert_eq!(stored_registration, Some(registration));
        let _ = Box::leak(Box::new(native));
        crate::windows_native_shutdown::record_windows_native_rdp_terminal(
            registration,
            windows_rdp_host::WindowsRdpTerminalOutcome::TimedOutLeaked,
            cx,
        );
        true
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn poll_windows_native_close(
        &mut self,
        registration: windows_rdp_host::WindowsRdpRegistration,
        cx: &mut Context<Self>,
    ) -> WindowsNativeClosePoll {
        if self.windows_native_registration.is_none() && self.windows_native.is_none() {
            return WindowsNativeClosePoll::Closed;
        }
        if !self.windows_native_matches_registration(registration) {
            return WindowsNativeClosePoll::Failed;
        }
        let Some(native) = self.windows_native.as_mut() else {
            return WindowsNativeClosePoll::Failed;
        };
        let Some(event_state) = self.native_event_state.as_mut() else {
            return WindowsNativeClosePoll::Failed;
        };
        if !native.close_confirmed(event_state) {
            return WindowsNativeClosePoll::Pending;
        }

        match native.finish_destroy() {
            Ok(windows_native::NativeDestroyProgress::Destroyed) => {
                if self.complete_destroyed_windows_native(registration, cx) {
                    WindowsNativeClosePoll::Closed
                } else {
                    WindowsNativeClosePoll::Failed
                }
            }
            Ok(windows_native::NativeDestroyProgress::PendingCallbacks) => {
                WindowsNativeClosePoll::Pending
            }
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "failed to destroy Windows native RDP after close confirmation"
                );
                WindowsNativeClosePoll::Failed
            }
        }
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn finish_windows_native_close(
        &mut self,
        registration: windows_rdp_host::WindowsRdpRegistration,
        cx: &mut Context<Self>,
    ) -> WindowsNativeClosePoll {
        if self.windows_native_registration.is_none() && self.windows_native.is_none() {
            return WindowsNativeClosePoll::Closed;
        }
        if !self.windows_native_matches_registration(registration) {
            return WindowsNativeClosePoll::Failed;
        }
        let Some(native) = self.windows_native.as_mut() else {
            return WindowsNativeClosePoll::Failed;
        };

        match native.finish_destroy() {
            Ok(windows_native::NativeDestroyProgress::Destroyed) => {
                if self.complete_destroyed_windows_native(registration, cx) {
                    WindowsNativeClosePoll::Closed
                } else {
                    WindowsNativeClosePoll::Failed
                }
            }
            Ok(windows_native::NativeDestroyProgress::PendingCallbacks) => {
                WindowsNativeClosePoll::Pending
            }
            Err(error) => {
                tracing::warn!(?error, "failed to destroy Windows native RDP");
                WindowsNativeClosePoll::Failed
            }
        }
    }

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    fn force_close_windows_native(
        &mut self,
        registration: windows_rdp_host::WindowsRdpRegistration,
        cx: &mut Context<Self>,
    ) -> WindowsNativeClosePoll {
        self.windows_native_display.reset();
        if self.windows_native_registration.is_none() && self.windows_native.is_none() {
            return WindowsNativeClosePoll::Closed;
        }
        if !self.windows_native_matches_registration(registration) {
            return WindowsNativeClosePoll::Failed;
        }
        let Some(native) = self.windows_native.as_mut() else {
            return WindowsNativeClosePoll::Failed;
        };

        let mut focus_parent = || {};
        match native.force_close(&mut focus_parent) {
            Ok(windows_native::NativeDestroyProgress::Destroyed) => {
                if self.complete_destroyed_windows_native(registration, cx) {
                    WindowsNativeClosePoll::Closed
                } else {
                    WindowsNativeClosePoll::Failed
                }
            }
            Ok(windows_native::NativeDestroyProgress::PendingCallbacks) => {
                WindowsNativeClosePoll::Pending
            }
            Err(error) => {
                tracing::warn!(?error, "failed to force-close Windows native RDP");
                WindowsNativeClosePoll::Failed
            }
        }
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
