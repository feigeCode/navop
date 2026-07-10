use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::*;
use gpui_component::{ActiveTheme, Icon, IconName};
use one_core::tab_container::{TabContent, TabContentEvent};
use remote_desktop::{
    RemoteDesktopConnectionOptions, RemoteDesktopInput, RemoteDesktopOutput, RemoteDesktopProtocol,
    RemoteDesktopProviderVersionError, RemoteDesktopRuntime, RemoteDesktopSize, RemoteKey,
    RemoteMouseButton, RemoteNamedKey, create_backend,
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
    rendered_frames: RenderedFrameLifecycle<Arc<RenderImage>>,
    remote_size: Option<(u16, u16)>,
    content_bounds: Option<Bounds<Pixels>>,
    last_resize_size: Option<(u16, u16)>,
    pending_resize_size: Option<(u16, u16)>,
    pending_resize_updated_at: Option<Instant>,
    last_resize_sent_at: Option<Instant>,
    modifiers: Modifiers,
    last_clipboard_text: Option<String>,
    last_clipboard_sync_at: Option<Instant>,
    status: SharedString,
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
            if let Some(input_tx) = this.input_tx.take() {
                let _ = input_tx.send(RemoteDesktopInput::Close);
            }
            let frames = this
                .rendered_frames
                .take_all_distinct(this.latest_frame.take());
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
            rendered_frames: RenderedFrameLifecycle::default(),
            remote_size: None,
            content_bounds: None,
            last_resize_size: None,
            pending_resize_size: None,
            pending_resize_updated_at: None,
            last_resize_sent_at: None,
            modifiers: Modifiers::default(),
            last_clipboard_text: None,
            last_clipboard_sync_at: None,
            status: SharedString::from("Waiting for layout"),
            tab_index: config.tab_index,
            _output_poll_task: output_poll_task,
        }
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
}
