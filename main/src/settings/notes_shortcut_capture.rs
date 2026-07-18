use gpui::{App, Entity, FocusHandle, KeyDownEvent, Keystroke, Window};
use one_core::settings::AppSettings;

#[derive(Clone)]
pub(super) struct NotesShortcutCaptureState {
    pub active_command_id: Option<String>,
    pub invalid_capture: bool,
    pub focus_handle: FocusHandle,
}

impl NotesShortcutCaptureState {
    pub fn new(cx: &mut App) -> Self {
        Self {
            active_command_id: None,
            invalid_capture: false,
            focus_handle: cx.focus_handle(),
        }
    }
}

pub(super) struct NotesShortcutCapture {
    command_id: String,
    state: Entity<NotesShortcutCaptureState>,
}

impl NotesShortcutCapture {
    pub fn new(command_id: String, state: Entity<NotesShortcutCaptureState>) -> Self {
        Self { command_id, state }
    }

    pub fn handle(&self, event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
        window.prevent_default();
        cx.stop_propagation();
        if event.keystroke.key == "escape" {
            clear_capture(&self.state, cx);
            return;
        }
        let Some(spec) = shortcut_spec(&event.keystroke) else {
            return;
        };
        if Keystroke::parse(&spec).is_err() {
            mark_invalid(&self.state, cx);
            return;
        }
        save_shortcut(&self.command_id, spec, cx);
        clear_capture(&self.state, cx);
    }
}

pub(super) fn reset_shortcut(command_id: &str, cx: &mut App) {
    AppSettings::update_and_save(cx, |settings| {
        settings.custom_keybindings.remove(command_id);
    });
    crate::onetcli_app::refresh_keybindings(cx);
}

pub(super) fn clear_capture(state: &Entity<NotesShortcutCaptureState>, cx: &mut App) {
    state.update(cx, |state, cx| {
        state.active_command_id = None;
        state.invalid_capture = false;
        cx.notify();
    });
}

fn save_shortcut(command_id: &str, spec: String, cx: &mut App) {
    AppSettings::update_and_save(cx, |settings| {
        settings
            .custom_keybindings
            .insert(command_id.to_string(), vec![spec]);
    });
    crate::onetcli_app::refresh_keybindings(cx);
}

fn mark_invalid(state: &Entity<NotesShortcutCaptureState>, cx: &mut App) {
    state.update(cx, |state, cx| {
        state.invalid_capture = true;
        cx.notify();
    });
}

fn shortcut_spec(keystroke: &Keystroke) -> Option<String> {
    let key = keystroke.key.as_str();
    if matches!(key, "ctrl" | "control" | "alt" | "shift" | "cmd" | "win") {
        return None;
    }
    let mut tokens = Vec::with_capacity(5);
    if keystroke.modifiers.control {
        tokens.push("ctrl");
    }
    if keystroke.modifiers.alt {
        tokens.push("alt");
    }
    if keystroke.modifiers.shift {
        tokens.push("shift");
    }
    if keystroke.modifiers.platform {
        tokens.push("cmd");
    }
    tokens.push(key);
    Some(tokens.join("-"))
}
