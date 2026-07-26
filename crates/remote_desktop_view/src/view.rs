use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::*;
use gpui_component::{ActiveTheme, Icon, IconName};
use one_core::tab_container::{TabContent, TabContentEvent};
use remote_desktop::{
    RemoteDesktopConnectionOptions, RemoteDesktopFrameRect, RemoteDesktopInput,
    RemoteDesktopOutput, RemoteDesktopProtocol, RemoteDesktopProviderVersionError,
    RemoteDesktopRuntime, RemoteDesktopSize, RemoteKey, RemoteMouseButton, RemoteNamedKey,
    RgbaFramebuffer, create_backend,
};
use rust_i18n::t;

use crate::ime_guard::RemoteDesktopImeGuard;
use crate::keyboard::keystroke_to_remote_key_for_protocol;
use crate::modifiers::modifier_inputs;
use crate::pixels::{bgra_to_render_image, rgba_to_render_image};
use crate::pointer::{LocalBounds, scale_filled_window_pointer_position};
use crate::shortcuts::{
    ClipboardShortcut, clipboard_shortcut_inputs, is_clipboard_platform_shortcut,
};
use crate::view::frame_lifecycle::RenderedFrameLifecycle;

mod frame_lifecycle;
mod input;
mod output;
mod render;
mod resize;

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(800);
const RESIZE_MIN_INTERVAL: Duration = Duration::from_millis(1200);
const RESIZE_DELTA_THRESHOLD: u16 = 16;
const RDP_INITIAL_LAYOUT_DEBOUNCE: Duration = Duration::from_millis(800);
const CLIPBOARD_SYNC_INTERVAL: Duration = Duration::from_millis(500);
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
            let mut frames = std::mem::take(&mut this.pending_frame_drops);
            frames.extend(
                this.rendered_frames
                    .take_all_distinct(this.latest_frame.take()),
            );
            let _ = window_handle.update(cx, move |_, window, _| {
                for frame in frames {
                    if let Err(error) = window.drop_image(frame) {
                        tracing::warn!(?error, "failed to release remote desktop frame");
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
mod tests {
    use remote_desktop::{RemoteDesktopProtocol, RemoteDesktopProviderVersionError};

    use super::close_runtime_once;

    #[test]
    fn closes_runtime_only_once() {
        let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut input_tx = Some(input_tx);

        close_runtime_once(&mut input_tx);
        close_runtime_once(&mut input_tx);

        assert_eq!(
            Some(remote_desktop::RemoteDesktopInput::Close),
            input_rx.blocking_recv()
        );
        assert!(input_rx.try_recv().is_err());
    }

    #[test]
    fn tab_title_uses_connection_name_and_duplicate_index() {
        assert_eq!(
            "prod-rdp",
            super::remote_desktop_tab_title("prod-rdp", None)
        );
        assert_eq!(
            "prod-rdp(2)",
            super::remote_desktop_tab_title("prod-rdp", Some(2))
        );
    }

    #[test]
    fn provider_version_error_message_is_localized_after_context() {
        let error = anyhow::Error::new(RemoteDesktopProviderVersionError {
            protocol: RemoteDesktopProtocol::Vnc,
            installed: "0.1.0".to_string(),
            required: "0.1.1".to_string(),
            invalid: false,
        })
        .context("VNC remote desktop provider");

        assert_eq!(
            "VNC provider version 0.1.0 is too old. Please update the provider to 0.1.1 or newer.",
            super::remote_desktop_error_message(&error)
        );
    }

    #[test]
    fn rendered_frame_cannot_expand_its_tab_container() {
        let source = include_str!("view/render.rs");

        let content_start = source
            .find("let content = div()")
            .expect("remote desktop content");
        let root_start = source[content_start..]
            .find("\n        div()\n            .size_full()\n            .min_w_0()")
            .map(|offset| content_start + offset)
            .expect("remote desktop root");
        let content = &source[content_start..root_start];
        let root = &source[root_start..];

        assert!(content.contains(".size_full()"));
        assert!(content.contains(".min_w_0()"));
        assert!(content.contains(".min_h_0()"));
        assert!(content.contains(".overflow_hidden()"));
        let frame_start = content.find("img(frame)").expect("rendered frame");
        let frame_end = content[frame_start..]
            .find(".object_fit(ObjectFit::Fill)")
            .map(|offset| frame_start + offset)
            .expect("rendered frame fit");
        let frame = &content[frame_start..frame_end];
        assert!(frame.contains(".size_full()"));
        assert!(frame.contains(".min_w_0()"));
        assert!(frame.contains(".min_h_0()"));

        let status = &content[content
            .find(".when(rendered_frame.is_none()")
            .expect("empty-frame status")..];
        assert!(status.contains(".min_w_0()"));
        assert!(status.contains(".max_w_full()"));
        assert!(status.contains(".overflow_hidden()"));

        assert!(root.contains(".size_full()"));
        assert!(root.contains(".min_w_0()"));
        assert!(root.contains(".min_h_0()"));
        assert!(root.contains(".overflow_hidden()"));
    }

    #[test]
    fn reconnect_status_overlay_has_a_full_size_layout_boundary() {
        let source = include_str!("view/render.rs");
        let overlay_start = source
            .find(".when(show_status_overlay")
            .expect("reconnect overlay");
        let overlay = &source[overlay_start..];
        let badge_start = overlay
            .find(".id(\"remote-desktop-status-overlay\")")
            .expect("reconnect status badge");
        let boundary = &overlay[..badge_start];
        let badge = &overlay[badge_start..];

        assert!(boundary.contains(".absolute()"));
        assert!(boundary.contains(".inset_0()"));
        assert!(boundary.contains(".min_w_0()"));
        assert!(boundary.contains(".min_h_0()"));
        assert!(boundary.contains(".flex()"));
        assert!(boundary.contains(".overflow_hidden()"));
        assert!(boundary.contains(".p_2()"));
        assert!(badge.contains(".min_w_0()"));
        assert!(badge.contains(".max_w(px(520.0))"));
        assert!(badge.contains(".flex_shrink(1.0)"));
        assert!(badge.contains(".overflow_hidden()"));
    }
}
