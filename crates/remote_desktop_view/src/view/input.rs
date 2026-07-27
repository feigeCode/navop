use super::clipboard::{
    LocalClipboardContent, allocate_local_clipboard_transfer_id, classify_local_clipboard,
    clipboard_files_supported, clipboard_text_supported,
};
use super::*;

impl RemoteDesktopView {
    pub(super) fn send_input(&self, input: RemoteDesktopInput) {
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

    pub(super) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn handle_key_up(
        &mut self,
        event: &KeyUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn send_tab(&mut self, _: &SendTab, _: &mut Window, cx: &mut Context<Self>) {
        self.send_key_press(RemoteKey::Named(RemoteNamedKey::Tab));
        cx.stop_propagation();
    }

    pub(super) fn send_shift_tab(
        &mut self,
        _: &SendShiftTab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn remote_copy(&mut self, _: &RemoteCopy, _: &mut Window, cx: &mut Context<Self>) {
        self.send_clipboard_shortcut(ClipboardShortcut::Copy);
        cx.stop_propagation();
    }

    pub(super) fn remote_paste(
        &mut self,
        _: &RemotePaste,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.send_local_clipboard_to_remote(window, cx) {
            self.send_clipboard_shortcut(ClipboardShortcut::Paste);
        }
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

    fn send_local_clipboard_to_remote(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return true;
        };
        self.last_clipboard_sync_at = Some(Instant::now());
        match classify_local_clipboard(&item) {
            LocalClipboardContent::Files(paths) => {
                self.last_clipboard_files = Some(paths.clone());
                self.last_clipboard_text = None;
                if !clipboard_files_supported(self.options.protocol) {
                    return false;
                }
                let transfer_id =
                    allocate_local_clipboard_transfer_id(&mut self.next_clipboard_transfer_id);
                self.send_input(RemoteDesktopInput::ClipboardFiles { transfer_id, paths });
                true
            }
            LocalClipboardContent::Text(text) => {
                self.last_clipboard_text = Some(text.clone());
                self.last_clipboard_files = None;
                if !clipboard_text_supported(self.options.protocol, &text) {
                    self.notify_vnc_clipboard_ascii_warning(window, cx);
                    return false;
                }
                self.send_input(RemoteDesktopInput::ClipboardText { text });
                true
            }
            LocalClipboardContent::Other => true,
        }
    }

    pub(super) fn handle_modifiers_changed(
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

    pub(super) fn send_pointer_move(&mut self, position: Point<Pixels>, window: &mut Window) {
        self.cursor.refresh_native_cursor();
        let Some((remote_width, remote_height)) = self.remote_size else {
            return;
        };
        let Some((x, y)) = scale_filled_window_pointer_position(
            pixels_to_f32(position.x),
            pixels_to_f32(position.y),
            self.pointer_bounds(window),
            remote_width,
            remote_height,
        ) else {
            return;
        };
        if should_track_local_cursor_position(
            self.options.protocol,
            self.connected,
            self.options.read_only,
        ) {
            self.cursor.set_position(x, y);
        }
        self.send_input(RemoteDesktopInput::MouseMove { x, y });
    }

    pub(super) fn send_mouse_button(&self, button: MouseButton, pressed: bool) {
        let Some(button) = map_mouse_button(button) else {
            return;
        };
        self.send_input(RemoteDesktopInput::MouseButton { button, pressed });
    }

    pub(super) fn send_scroll(&self, event: &ScrollWheelEvent) {
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

fn should_track_local_cursor_position(
    protocol: RemoteDesktopProtocol,
    connected: bool,
    read_only: bool,
) -> bool {
    protocol == RemoteDesktopProtocol::Vnc && connected && !read_only
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

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
