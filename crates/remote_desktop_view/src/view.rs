use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{ActiveTheme, Icon, IconName};
use one_core::tab_container::{TabContent, TabContentEvent};
use remote_desktop::{
    RemoteDesktopConnectionOptions, RemoteDesktopInput, RemoteDesktopOutput, RemoteDesktopProtocol,
    RemoteDesktopRuntime, RemoteDesktopSize, RemoteKey, RemoteMouseButton, RemoteNamedKey,
    create_backend,
};

use crate::ime_guard::RemoteDesktopImeGuard;
use crate::keyboard::keystroke_to_remote_key_for_protocol;
use crate::modifiers::modifier_inputs;
use crate::pixels::rgba_to_render_image;
use crate::pointer::{LocalBounds, scale_filled_window_pointer_position};
use crate::shortcuts::{
    ClipboardShortcut, clipboard_shortcut_inputs, is_clipboard_platform_shortcut,
};

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(800);
const RESIZE_MIN_INTERVAL: Duration = Duration::from_millis(1200);
const RESIZE_DELTA_THRESHOLD: u16 = 16;
const CLIPBOARD_SYNC_INTERVAL: Duration = Duration::from_millis(500);
const RDP_DISPLAY_MIN_SIZE: f32 = 200.0;
const RDP_DISPLAY_MAX_SIZE: f32 = 8192.0;
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
    output_rx: Option<std::sync::mpsc::Receiver<RemoteDesktopOutput>>,
    focus_handle: FocusHandle,
    frame: Option<Arc<RenderImage>>,
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
}

impl RemoteDesktopView {
    pub fn new(config: RemoteDesktopViewConfig, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();

        cx.spawn(async move |this, cx| {
            loop {
                let _ = this.update(cx, |_, cx| cx.notify());
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
            }
        })
        .detach();

        Self {
            options: config.options,
            title: config.title,
            input_tx: None,
            output_rx: None,
            focus_handle,
            frame: None,
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
        }
    }

    fn start_runtime(&mut self, size: (u16, u16)) {
        if self.input_tx.is_some() {
            return;
        }
        let runtime = create_backend(self.options.clone())
            .start(RemoteDesktopSize {
                width: size.0,
                height: size.1,
            })
            .unwrap_or_else(failed_runtime);
        self.input_tx = Some(runtime.input_tx);
        self.output_rx = Some(runtime.output_rx);
        self.last_resize_size = Some(size);
        self.status = SharedString::from("Connecting");
    }

    fn drain_output(&mut self, cx: &mut Context<Self>) {
        let Some(output_rx) = self.output_rx.as_ref() else {
            return;
        };
        let mut outputs = Vec::new();
        while let Ok(output) = output_rx.try_recv() {
            outputs.push(output);
        }
        for output in outputs {
            match output {
                RemoteDesktopOutput::Connected { width, height, .. } => {
                    self.remote_size = Some((width, height));
                    self.status = SharedString::from("Connected");
                }
                RemoteDesktopOutput::Frame {
                    width,
                    height,
                    rgba,
                } => {
                    self.remote_size = Some((width, height));
                    match rgba_to_render_image(width, height, rgba) {
                        Ok(image) => self.frame = Some(Arc::new(image)),
                        Err(error) => self.status = SharedString::from(error.to_string()),
                    }
                }
                RemoteDesktopOutput::Status(message) => self.status = SharedString::from(message),
                RemoteDesktopOutput::ConnectionFailure(message)
                | RemoteDesktopOutput::Terminated(message) => {
                    self.handle_disconnect_status(message)
                }
                RemoteDesktopOutput::CursorDefault
                | RemoteDesktopOutput::CursorHidden
                | RemoteDesktopOutput::CursorPosition { .. } => {}
                RemoteDesktopOutput::ClipboardText { text } => {
                    if self.last_clipboard_text.as_deref() != Some(text.as_str()) {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        self.last_clipboard_text = Some(text);
                        self.last_clipboard_sync_at = Some(Instant::now());
                    }
                }
            }
        }
    }

    fn sync_local_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            return;
        }
        if self
            .last_clipboard_sync_at
            .is_some_and(|synced_at| synced_at.elapsed() < CLIPBOARD_SYNC_INTERVAL)
        {
            return;
        }
        self.last_clipboard_sync_at = Some(Instant::now());
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if self.last_clipboard_text.as_deref() == Some(text.as_str()) {
            return;
        }
        self.last_clipboard_text = Some(text.clone());
        self.send_input(RemoteDesktopInput::ClipboardText { text });
    }

    fn handle_disconnect_status(&mut self, message: String) {
        self.modifiers = Modifiers::default();
        self.status = SharedString::from(message);
    }

    fn request_reconnect(&mut self) {
        self.modifiers = Modifiers::default();
        self.status = SharedString::from("reconnecting RDP session");
        self.send_input(RemoteDesktopInput::Reconnect);
    }

    fn update_content_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.content_bounds = Some(bounds);
        let Some(size) = resize_dimensions_from_bounds(bounds) else {
            return;
        };
        self.start_runtime(size);
        if !is_meaningful_resize_delta(self.last_resize_size, size) {
            return;
        }
        if self.pending_resize_size == Some(size) {
            return;
        }
        self.pending_resize_size = Some(size);
        self.pending_resize_updated_at = Some(Instant::now());
    }

    fn flush_pending_resize(&mut self) {
        if self.remote_size.is_none() {
            return;
        }
        let Some(size) = self.pending_resize_size else {
            return;
        };
        let Some(updated_at) = self.pending_resize_updated_at else {
            return;
        };
        if updated_at.elapsed() < RESIZE_DEBOUNCE {
            return;
        }
        if self
            .last_resize_sent_at
            .is_some_and(|sent_at| sent_at.elapsed() < RESIZE_MIN_INTERVAL)
        {
            return;
        }
        self.pending_resize_size = None;
        self.pending_resize_updated_at = None;
        self.last_resize_size = Some(size);
        self.last_resize_sent_at = Some(Instant::now());
        self.send_input(RemoteDesktopInput::Resize {
            width: size.0,
            height: size.1,
        });
    }

    fn send_input(&self, input: RemoteDesktopInput) {
        if self.options.read_only
            && !matches!(
                input,
                RemoteDesktopInput::Resize { .. } | RemoteDesktopInput::Reconnect
            )
        {
            return;
        }

        if let Some(input_tx) = &self.input_tx {
            let _ = input_tx.send(input);
        }
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if is_clipboard_platform_shortcut(&event.keystroke) {
            cx.stop_propagation();
            return;
        }
        if let Some(key) =
            keystroke_to_remote_key_for_protocol(&event.keystroke, self.options.protocol)
        {
            self.send_input(RemoteDesktopInput::Key { key, pressed: true });
        }
        cx.stop_propagation();
    }

    fn handle_key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if is_clipboard_platform_shortcut(&event.keystroke) {
            cx.stop_propagation();
            return;
        }
        if let Some(key) =
            keystroke_to_remote_key_for_protocol(&event.keystroke, self.options.protocol)
        {
            self.send_input(RemoteDesktopInput::Key {
                key,
                pressed: false,
            });
        }
        cx.stop_propagation();
    }

    fn send_tab(&mut self, _: &SendTab, _window: &mut Window, cx: &mut Context<Self>) {
        self.send_key_press(RemoteKey::Named(RemoteNamedKey::Tab));
        cx.stop_propagation();
    }

    fn send_shift_tab(&mut self, _: &SendShiftTab, _window: &mut Window, cx: &mut Context<Self>) {
        self.send_input(RemoteDesktopInput::Key {
            key: RemoteKey::Named(RemoteNamedKey::Shift),
            pressed: true,
        });
        self.send_key_press(RemoteKey::Named(RemoteNamedKey::Tab));
        self.send_input(RemoteDesktopInput::Key {
            key: RemoteKey::Named(RemoteNamedKey::Shift),
            pressed: false,
        });
        cx.stop_propagation();
    }

    fn remote_copy(&mut self, _: &RemoteCopy, _window: &mut Window, cx: &mut Context<Self>) {
        self.send_clipboard_shortcut(ClipboardShortcut::Copy);
        cx.stop_propagation();
    }

    fn remote_paste(&mut self, _: &RemotePaste, _window: &mut Window, cx: &mut Context<Self>) {
        self.send_local_clipboard_to_remote(cx);
        self.send_clipboard_shortcut(ClipboardShortcut::Paste);
        cx.stop_propagation();
    }

    fn send_key_press(&self, key: RemoteKey) {
        self.send_input(RemoteDesktopInput::Key {
            key: key.clone(),
            pressed: true,
        });
        self.send_input(RemoteDesktopInput::Key {
            key,
            pressed: false,
        });
    }

    fn send_clipboard_shortcut(&self, shortcut: ClipboardShortcut) {
        for input in clipboard_shortcut_inputs(self.options.protocol, shortcut) {
            self.send_input(input);
        }
    }

    fn send_local_clipboard_to_remote(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        self.last_clipboard_text = Some(text.clone());
        self.last_clipboard_sync_at = Some(Instant::now());
        self.send_input(RemoteDesktopInput::ClipboardText { text });
    }

    fn handle_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.modifiers;
        self.modifiers = event.modifiers;

        if self.options.protocol == RemoteDesktopProtocol::Rdp {
            for input in modifier_inputs(previous, event.modifiers) {
                self.send_input(input);
            }
        }

        cx.stop_propagation();
    }

    fn send_pointer_move(&mut self, position: Point<Pixels>, window: &mut Window) {
        let Some((remote_width, remote_height)) = self.remote_size else {
            return;
        };
        let bounds = self.pointer_bounds(window);
        let Some((x, y)) = scale_filled_window_pointer_position(
            pixels_to_f32(position.x),
            pixels_to_f32(position.y),
            bounds,
            remote_width,
            remote_height,
        ) else {
            return;
        };
        self.send_input(RemoteDesktopInput::MouseMove { x, y });
    }

    fn send_mouse_button(&self, button: MouseButton, pressed: bool) {
        let Some(button) = map_mouse_button(button) else {
            return;
        };
        self.send_input(RemoteDesktopInput::MouseButton { button, pressed });
    }

    fn send_scroll(&self, event: &ScrollWheelEvent) {
        match event.delta {
            ScrollDelta::Lines(delta) => self.send_scroll_delta(delta.x, delta.y, 100.0),
            ScrollDelta::Pixels(delta) => {
                self.send_scroll_delta(pixels_to_f32(delta.x), pixels_to_f32(delta.y), 1.0)
            }
        }
    }

    fn send_scroll_delta(&self, x: f32, y: f32, multiplier: f32) {
        if x.abs() > 0.001 {
            self.send_input(RemoteDesktopInput::Wheel {
                vertical: false,
                units: (x * multiplier) as i16,
            });
        }
        if y.abs() > 0.001 {
            self.send_input(RemoteDesktopInput::Wheel {
                vertical: true,
                units: (y * multiplier) as i16,
            });
        }
    }

    fn pointer_bounds(&self, window: &mut Window) -> LocalBounds {
        self.content_bounds.map(bounds_to_local).unwrap_or_else(|| {
            let size = window.viewport_size();
            LocalBounds {
                left: 0.0,
                top: 0.0,
                width: pixels_to_f32(size.width),
                height: pixels_to_f32(size.height),
            }
        })
    }
}

fn map_mouse_button(button: MouseButton) -> Option<RemoteMouseButton> {
    match button {
        MouseButton::Left => Some(RemoteMouseButton::Left),
        MouseButton::Right => Some(RemoteMouseButton::Right),
        MouseButton::Middle => Some(RemoteMouseButton::Middle),
        MouseButton::Navigate(NavigationDirection::Back) => Some(RemoteMouseButton::X1),
        MouseButton::Navigate(NavigationDirection::Forward) => Some(RemoteMouseButton::X2),
    }
}

fn pixels_to_f32(pixels: Pixels) -> f32 {
    pixels.into()
}

fn bounds_to_local(bounds: Bounds<Pixels>) -> LocalBounds {
    LocalBounds {
        left: pixels_to_f32(bounds.left()),
        top: pixels_to_f32(bounds.top()),
        width: pixels_to_f32(bounds.size.width),
        height: pixels_to_f32(bounds.size.height),
    }
}

fn resize_dimensions_from_bounds(bounds: Bounds<Pixels>) -> Option<(u16, u16)> {
    let mut width = pixels_to_f32(bounds.size.width)
        .round()
        .clamp(RDP_DISPLAY_MIN_SIZE, RDP_DISPLAY_MAX_SIZE) as u16;
    if width % 2 != 0 {
        width = width.saturating_sub(1);
    }
    let height = pixels_to_f32(bounds.size.height)
        .round()
        .clamp(RDP_DISPLAY_MIN_SIZE, RDP_DISPLAY_MAX_SIZE) as u16;
    Some((width, height))
}

fn is_meaningful_resize_delta(previous: Option<(u16, u16)>, next: (u16, u16)) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    previous.0.abs_diff(next.0) >= RESIZE_DELTA_THRESHOLD
        || previous.1.abs_diff(next.1) >= RESIZE_DELTA_THRESHOLD
}

fn failed_runtime(error: anyhow::Error) -> RemoteDesktopRuntime {
    let (input_tx, _input_rx) = tokio::sync::mpsc::unbounded_channel();
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    let _ = output_tx.send(RemoteDesktopOutput::ConnectionFailure(error.to_string()));
    RemoteDesktopRuntime {
        input_tx,
        output_rx,
    }
}

impl Focusable for RemoteDesktopView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for RemoteDesktopView {}

impl TabContent for RemoteDesktopView {
    fn content_key(&self) -> &'static str {
        "RemoteDesktop"
    }

    fn title(&self, _cx: &App) -> SharedString {
        SharedString::from(remote_desktop_tab_title(&self.title, self.tab_index))
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::Monitor.color())
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Task<bool> {
        if let Some(input_tx) = &self.input_tx {
            let _ = input_tx.send(RemoteDesktopInput::Close);
        }
        Task::ready(true)
    }
}

impl Render for RemoteDesktopView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drain_output(cx);
        self.sync_local_clipboard(window, cx);
        self.flush_pending_resize();
        let view = cx.entity();
        let focus_handle = self.focus_handle.clone();
        let show_status_overlay = self.status.as_ref() != "Connected";

        let content = div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .key_context(REMOTE_DESKTOP_CONTEXT)
            .on_action(cx.listener(Self::send_tab))
            .on_action(cx.listener(Self::send_shift_tab))
            .on_action(cx.listener(Self::remote_copy))
            .on_action(cx.listener(Self::remote_paste))
            .capture_key_down(cx.listener(Self::handle_key_down))
            .capture_key_up(cx.listener(Self::handle_key_up))
            .on_modifiers_changed(cx.listener(Self::handle_modifiers_changed))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.send_pointer_move(event.position, window);
                cx.stop_propagation();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.send_pointer_move(event.position, window);
                    this.send_mouse_button(event.button, true);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.send_pointer_move(event.position, window);
                    this.send_mouse_button(event.button, true);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.send_pointer_move(event.position, window);
                    this.send_mouse_button(event.button, true);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.send_pointer_move(event.position, window);
                    this.send_mouse_button(event.button, false);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.send_pointer_move(event.position, window);
                    this.send_mouse_button(event.button, false);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.send_pointer_move(event.position, window);
                    this.send_mouse_button(event.button, false);
                    cx.stop_propagation();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                this.send_scroll(event);
                cx.stop_propagation();
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        window.handle_input(&focus_handle, RemoteDesktopImeGuard::new(bounds), cx);
                    },
                )
                .absolute()
                .size_full(),
            )
            .when_some(self.frame.clone(), |this, frame| {
                this.child(img(frame).size_full().object_fit(ObjectFit::Fill))
            })
            .when(self.frame.is_none(), |this| {
                this.child(
                    div()
                        .px_4()
                        .py_2()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.status.clone()),
                )
            });

        div()
            .size_full()
            .relative()
            .on_children_prepainted(move |bounds, _, cx| {
                if let Some(bounds) = bounds.first().copied() {
                    view.update(cx, |view, _| {
                        view.update_content_bounds(bounds);
                    });
                }
            })
            .child(content)
            .when(show_status_overlay, |this| {
                this.child(
                    div()
                        .id("remote-desktop-status-overlay")
                        .absolute()
                        .top_2()
                        .left_2()
                        .max_w(px(520.0))
                        .px_3()
                        .py_1()
                        .border_1()
                        .rounded_sm()
                        .bg(cx.theme().background)
                        .border_color(cx.theme().border)
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.request_reconnect();
                            cx.stop_propagation();
                        }))
                        .child(self.status.clone()),
                )
            })
    }
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
    use gpui::{Bounds, point, px, size};

    #[test]
    fn resize_dimensions_from_bounds_adjust_to_display_control_limits() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(1281.4), px(720.6)));

        assert_eq!(
            Some((1280, 721)),
            super::resize_dimensions_from_bounds(bounds)
        );

        let oversized = Bounds::new(point(px(0.0), px(0.0)), size(px(90000.0), px(0.0)));

        assert_eq!(
            Some((8192, 200)),
            super::resize_dimensions_from_bounds(oversized)
        );
    }

    #[test]
    fn resize_change_requires_meaningful_delta() {
        assert!(!super::is_meaningful_resize_delta(
            Some((1280, 720)),
            (1284, 726)
        ));
        assert!(super::is_meaningful_resize_delta(
            Some((1280, 720)),
            (1300, 726)
        ));
        assert!(super::is_meaningful_resize_delta(None, (1280, 720)));
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
}
