use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::*;
use gpui_component::{ActiveTheme, Icon, IconName};
use one_core::tab_container::{TabContent, TabContentEvent};
use remote_desktop::{
    RemoteDesktopCapabilities, RemoteDesktopConnectionOptions, RemoteDesktopInput,
    RemoteDesktopOutput, RemoteDesktopProtocol, RemoteDesktopProviderVersionError,
    RemoteDesktopRuntime, RemoteDesktopSize, RemoteKey, RemoteMouseButton, RemoteNamedKey,
    ResizeSupport, RgbaFramebuffer, create_backend,
};
use rust_i18n::t;

use crate::ime_guard::RemoteDesktopImeGuard;
use crate::keyboard::keystroke_to_remote_key_for_protocol;
use crate::modifiers::modifier_inputs;
use crate::pointer::{LocalBounds, scale_filled_window_pointer_position};
use crate::shortcuts::{
    ClipboardShortcut, clipboard_shortcut_inputs, is_clipboard_platform_shortcut,
};
use crate::view::frame_lifecycle::RenderedFrameLifecycle;

mod clipboard;
mod cursor;
mod frame_lifecycle;
mod frame_sync;
mod frames;
mod input;
mod notifications;
mod output;
mod render;
mod resize;

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(800);
const RESIZE_MIN_INTERVAL: Duration = Duration::from_millis(1200);
const RESIZE_DELTA_THRESHOLD: u16 = 16;
const RDP_INITIAL_LAYOUT_DEBOUNCE: Duration = Duration::from_millis(800);
const REMOTE_DESKTOP_CONTEXT: &str = "RemoteDesktopView";

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
    focus_handle: FocusHandle,
    latest_frame: Option<Arc<RenderImage>>,
    framebuffer: Option<RgbaFramebuffer>,
    rendered_frames: RenderedFrameLifecycle<Arc<RenderImage>>,
    pending_frame_drops: Vec<Arc<RenderImage>>,
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
    modifiers: Modifiers,
    last_clipboard_text: Option<String>,
    last_clipboard_files: Option<Vec<String>>,
    last_clipboard_sync_at: Option<Instant>,
    next_clipboard_transfer_id: u64,
    display_scale_factor: u32,
    status: SharedString,
    connected: bool,
    tab_index: Option<usize>,
    _output_poll_task: Task<()>,
}

impl RemoteDesktopView {
    pub fn new(
        config: RemoteDesktopViewConfig,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let output_poll_task = cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
            }
        });

        cx.on_release(move |this, cx| {
            close_runtime_once(&mut this.input_tx);
            let mut images = std::mem::take(&mut this.pending_frame_drops);
            images.extend(
                this.rendered_frames
                    .take_all_distinct(this.latest_frame.take()),
            );
            images.extend(this.cursor.release_all_images());
            let _ = window_handle.update(cx, move |_, window, _| {
                for image in images {
                    if let Err(error) = window.drop_image(image) {
                        tracing::warn!(?error, "failed to release remote desktop image");
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
            focus_handle,
            latest_frame: None,
            framebuffer: None,
            rendered_frames: RenderedFrameLifecycle::default(),
            pending_frame_drops: Vec::new(),
            cursor: cursor::RemoteCursorState::default(),
            frame_sync: frame_sync::FrameSyncTracker::default(),
            capabilities: None,
            remote_size: None,
            content_bounds: None,
            initial_size: resize::InitialSize::default(),
            last_resize_size: None,
            pending_resize_size: None,
            pending_resize_updated_at: None,
            last_resize_sent_at: None,
            modifiers: Modifiers::default(),
            last_clipboard_text: None,
            last_clipboard_files: None,
            last_clipboard_sync_at: None,
            next_clipboard_transfer_id: clipboard::FIRST_LOCAL_CLIPBOARD_TRANSFER_ID,
            display_scale_factor: 100,
            status: SharedString::from(t!("RemoteDesktop.status_waiting_layout").to_string()),
            connected: false,
            tab_index: config.tab_index,
            _output_poll_task: output_poll_task,
        }
    }
}

fn close_runtime_once(
    input_tx: &mut Option<tokio::sync::mpsc::UnboundedSender<RemoteDesktopInput>>,
) {
    if let Some(input_tx) = input_tx.take() {
        let _ = input_tx.send(RemoteDesktopInput::Close);
    }
}

fn failed_runtime(error: anyhow::Error) -> RemoteDesktopRuntime {
    let (input_tx, _input_rx) = tokio::sync::mpsc::unbounded_channel();
    let (output_tx, output_rx) = remote_desktop::output_mailbox::output_mailbox();
    let _ = output_tx.send(RemoteDesktopOutput::ConnectionFailure(
        remote_desktop_error_message(&error),
    ));
    RemoteDesktopRuntime {
        input_tx,
        output_rx,
    }
}

fn remote_desktop_error_message(error: &anyhow::Error) -> String {
    if let Some(error) = error.downcast_ref::<RemoteDesktopProviderVersionError>() {
        let key = if error.invalid {
            "RemoteDesktop.provider_version_invalid"
        } else {
            "RemoteDesktop.provider_version_too_old"
        };
        return t!(
            key,
            protocol = error.protocol.label(),
            installed = error.installed.as_str(),
            required = error.required.as_str()
        )
        .to_string();
    }
    error.to_string()
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
#[path = "view/view_tests.rs"]
mod tests;
