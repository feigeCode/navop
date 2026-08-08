use std::sync::{Arc, OnceLock, atomic::AtomicU64};
use std::time::{Duration, Instant};

use gpui::*;
use gpui_component::{ActiveTheme, Icon, IconName};
use one_core::tab_container::{TabContent, TabContentEvent};
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
// Task 0 freezes this contract before later tasks wire it into the view runtime.
#[allow(dead_code)]
mod presentation;
mod render;
mod resize;
mod surface;
// Task 6 freezes the GPUI/native-child presentation adapter before Task 8
// enables the production presentation factory.
#[allow(dead_code)]
mod windows_native;

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(800);
const RESIZE_MIN_INTERVAL: Duration = Duration::from_millis(1200);
const RESIZE_DELTA_THRESHOLD: u16 = 16;
const RDP_INITIAL_LAYOUT_DEBOUNCE: Duration = Duration::from_millis(150);
const REMOTE_DESKTOP_CONTEXT: &str = "RemoteDesktopView";
const REMOTE_DESKTOP_DIAGNOSTICS_ENV: &str = "NAVOP_REMOTE_DESKTOP_DIAGNOSTICS";

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
    [SendTab, SendShiftTab, RemoteCopy, RemotePaste]
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

fn preserve_presented_frame_during_session_reset(reason: SessionResetReason) -> bool {
    matches!(reason, SessionResetReason::Reconnecting)
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
    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    windows_native: Option<windows_native::WindowsNativeAdapter>,
}

impl RemoteDesktopView {
    pub fn new(
        config: RemoteDesktopViewConfig,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let manage_native_cursor = config.options.protocol == RemoteDesktopProtocol::Rdp;
        let focus_handle = cx.focus_handle();

        cx.on_release(move |this, cx| {
            close_runtime_once(&mut this.input_tx);
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
            #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
            windows_native: None,
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

    #[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
    pub(crate) fn attach_windows_native_presentation(
        &mut self,
        presentation: windows_native::WindowsNativeAdapter,
    ) {
        self.windows_native = Some(presentation);
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
