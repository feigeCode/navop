use gpui::{
    App, Entity, FocusHandle, InteractiveElement, IntoElement, Keystroke, ParentElement, Styled,
    Window, div,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    kbd::Kbd,
    v_flex,
};
use notes::NotesShortcutDescriptor;
use one_core::settings::AppSettings;
use rust_i18n::t;

use super::notes_shortcut_capture::{
    NotesShortcutCapture, NotesShortcutCaptureState, clear_capture, reset_shortcut,
};
use super::notes_shortcut_labels::command_label;

struct NotesShortcutRow {
    command_id: String,
    label: String,
    current_keys: Vec<String>,
}

pub fn search_texts() -> Vec<String> {
    let mut texts = vec![t!("Settings.Shortcuts.notes_editor").to_string()];
    for descriptor in notes::shortcut_descriptors() {
        texts.push(command_label(&descriptor));
        texts.push(descriptor.command_id);
        texts.extend(descriptor.default_keys);
    }
    texts
}

pub fn render_group(window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let state = window.use_keyed_state("notes-shortcut-capture", cx, |_, cx| {
        NotesShortcutCaptureState::new(cx)
    });
    let mut list = v_flex().gap_1().pl_2();
    for descriptor in notes::shortcut_descriptors() {
        list = list.child(render_row(row_model(descriptor, cx), state.clone(), cx));
    }
    v_flex()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .child(t!("Settings.Shortcuts.notes_editor").to_string()),
        )
        .child(list)
        .into_any_element()
}

fn row_model(descriptor: NotesShortcutDescriptor, cx: &App) -> NotesShortcutRow {
    let current_keys = AppSettings::global(cx)
        .custom_keybindings
        .get(&descriptor.command_id)
        .filter(|keys| !keys.is_empty())
        .cloned()
        .unwrap_or_else(|| descriptor.default_keys.clone());
    NotesShortcutRow {
        label: command_label(&descriptor),
        command_id: descriptor.command_id,
        current_keys,
    }
}

fn render_row(
    row: NotesShortcutRow,
    state: Entity<NotesShortcutCaptureState>,
    cx: &mut App,
) -> gpui::AnyElement {
    let value = render_editor(&row, state, cx);
    h_flex()
        .group("notes-shortcut-row")
        .items_center()
        .justify_between()
        .gap_3()
        .py_1()
        .child(
            div()
                .flex_1()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(row.label),
        )
        .child(value)
        .into_any_element()
}

fn render_editor(
    row: &NotesShortcutRow,
    state: Entity<NotesShortcutCaptureState>,
    cx: &mut App,
) -> gpui::AnyElement {
    let editing = state.read(cx).active_command_id.as_deref() == Some(&row.command_id);
    if editing {
        return render_capture(row.command_id.clone(), state, cx);
    }
    render_current_shortcut(row, state, cx)
}

fn render_capture(
    command_id: String,
    state: Entity<NotesShortcutCaptureState>,
    cx: &mut App,
) -> gpui::AnyElement {
    let invalid = state.read(cx).invalid_capture;
    let focus_handle = state.read(cx).focus_handle.clone();
    h_flex()
        .gap_2()
        .track_focus(&focus_handle)
        .on_key_down({
            let capture = NotesShortcutCapture::new(command_id, state.clone());
            move |event, window, cx| {
                capture.handle(event, window, cx);
            }
        })
        .child(
            div()
                .px_2()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(if invalid {
                    cx.theme().danger
                } else {
                    cx.theme().border
                })
                .text_sm()
                .child(capture_label(invalid)),
        )
        .child(cancel_capture_button(state))
        .into_any_element()
}

fn render_current_shortcut(
    row: &NotesShortcutRow,
    state: Entity<NotesShortcutCaptureState>,
    cx: &mut App,
) -> gpui::AnyElement {
    let command_id = row.command_id.clone();
    let reset_id = row.command_id.clone();
    let focus_handle = state.read(cx).focus_handle.clone();
    h_flex()
        .gap_2()
        .child(render_shortcut_values(&row.current_keys, cx))
        .child(
            h_flex()
                .gap_1()
                .invisible()
                .group_hover("notes-shortcut-row", |this| this.visible())
                .child(edit_button(command_id, state.clone(), focus_handle))
                .child(reset_button(reset_id)),
        )
        .into_any_element()
}

fn edit_button(
    command_id: String,
    state: Entity<NotesShortcutCaptureState>,
    focus_handle: FocusHandle,
) -> Button {
    Button::new(format!("edit-notes-shortcut-{command_id}"))
        .icon(IconName::Edit)
        .ghost()
        .xsmall()
        .tooltip(t!("Common.edit").to_string())
        .on_click(move |_, window, cx| {
            state.update(cx, |state, cx| {
                state.active_command_id = Some(command_id.clone());
                state.invalid_capture = false;
                cx.notify();
            });
            focus_handle.focus(window, cx);
        })
}

fn reset_button(command_id: String) -> Button {
    Button::new(format!("reset-notes-shortcut-{command_id}"))
        .icon(IconName::Refresh)
        .ghost()
        .xsmall()
        .tooltip(t!("Settings.Shortcuts.reset").to_string())
        .on_click(move |_, _, cx| reset_shortcut(&command_id, cx))
}

fn cancel_capture_button(state: Entity<NotesShortcutCaptureState>) -> Button {
    Button::new("cancel-notes-shortcut-capture")
        .label(t!("Common.cancel").to_string())
        .ghost()
        .xsmall()
        .on_click(move |_, _, cx| clear_capture(&state, cx))
}

fn render_shortcut_values(keys: &[String], _cx: &App) -> gpui::AnyElement {
    h_flex()
        .gap_1()
        .flex_wrap()
        .justify_end()
        .children(keys.iter().map(|key| match Keystroke::parse(key) {
            Ok(keystroke) => Kbd::new(keystroke).into_any_element(),
            Err(_) => div().text_sm().child(key.clone()).into_any_element(),
        }))
        .into_any_element()
}

fn capture_label(invalid: bool) -> String {
    if invalid {
        t!("Settings.Shortcuts.invalid_hotkey").to_string()
    } else {
        t!("Settings.Shortcuts.press_shortcut").to_string()
    }
}
