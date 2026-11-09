//! 终端侧边栏连接笔记面板

use crate::theme::TerminalColors;
use gpui::{
    App, AppContext as _, Context, IntoElement, ParentElement, Render, Styled, Subscription,
    Window, div,
};
use gpui_component::{
    ActiveTheme, Disableable, StyledExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState, EditorStyleOverrides, InputEvent},
    v_flex,
};
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::storage::GlobalStorageState;
use one_core::storage::models::StoredConnection;
use one_core::storage::traits::Repository;
use rust_i18n::t;

pub struct ConnectionNotesPanel {
    connection: StoredConnection,
    input: gpui::Entity<EditorState>,
    saved_note: String,
    last_error: Option<String>,
    colors: TerminalColors,
    _input_subscription: Subscription,
}

impl ConnectionNotesPanel {
    pub fn new(
        connection: StoredConnection,
        colors: TerminalColors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let note = connection.remark.clone().unwrap_or_default();
        let input = cx.new(|cx| {
            EditorState::new(window, cx)
                .placeholder(t!("ConnectionNotes.placeholder").to_string())
                .line_number(false)
                .soft_wrap(true)
                .default_value(note.clone())
        });
        let _input_subscription = cx.subscribe(&input, |this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.last_error = None;
                cx.notify();
            }
        });

        Self {
            connection,
            input,
            saved_note: note,
            last_error: None,
            colors,
            _input_subscription,
        }
    }

    fn current_note(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn save_note(&mut self, cx: &mut Context<Self>) {
        let note = self.current_note(cx);
        let Some(storage) = cx
            .try_global::<GlobalStorageState>()
            .map(|state| state.storage.clone())
        else {
            self.last_error = Some(t!("ConnectionNotes.storage_unavailable").to_string());
            cx.notify();
            return;
        };
        let Some(repository) = storage.get::<one_core::storage::ConnectionRepository>() else {
            self.last_error = Some(t!("ConnectionNotes.storage_unavailable").to_string());
            cx.notify();
            return;
        };

        let mut connection = connection_with_note(self.connection.clone(), &note);
        match repository.update(&mut connection) {
            Ok(()) => {
                self.connection = connection.clone();
                self.saved_note = connection.remark.clone().unwrap_or_default();
                self.last_error = None;
                if let Some(notifier) = get_notifier(cx) {
                    notifier.update(cx, |_, cx| {
                        cx.emit(ConnectionDataEvent::ConnectionUpdated {
                            connection: connection.clone(),
                        });
                    });
                }
            }
            Err(error) => {
                self.last_error =
                    Some(t!("ConnectionNotes.save_failed", error = error.to_string()).to_string());
            }
        }
        cx.notify();
    }

    fn render_status(&self, dirty: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        if let Some(error) = &self.last_error {
            return div()
                .text_xs()
                .text_color(cx.theme().danger)
                .child(error.clone())
                .into_any_element();
        }

        if dirty {
            div()
                .text_xs()
                .text_color(cx.theme().warning)
                .child(t!("ConnectionNotes.unsaved").to_string())
                .into_any_element()
        } else {
            div()
                .text_xs()
                .text_color(self.colors.muted_foreground)
                .child(t!("ConnectionNotes.saved").to_string())
                .into_any_element()
        }
    }
}

impl Render for ConnectionNotesPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let note = self.current_note(cx);
        let dirty = note != self.saved_note;
        let connection_name = self.connection.name.clone();

        v_flex()
            .size_full()
            .p_3()
            .gap_3()
            .bg(self.colors.background)
            .text_color(self.colors.foreground)
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().font_semibold().child(connection_name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(self.colors.muted_foreground)
                            .child(t!("ConnectionNotes.description").to_string()),
                    ),
            )
            .child(
                div()
                    .rounded_md()
                    .bg(cx.theme().warning.opacity(0.1))
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(t!("ConnectionNotes.security_hint").to_string()),
            )
            .child(
                div().flex_1().min_h_0().child(
                    Editor::new(&self.input)
                        .w_full()
                        .h_full()
                        .font_family(cx.theme().font_family.clone())
                        .text_size(cx.theme().font_size)
                        .text_color(self.colors.foreground)
                        .bg(self.colors.muted)
                        .border_color(self.colors.border)
                        .appearance(true)
                        .bordered(true)
                        .editor_style(connection_notes_editor_style(&self.colors)),
                ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(self.render_status(dirty, cx))
                    .child(
                        Button::new("connection-notes-save")
                            .label(t!("ConnectionNotes.save").to_string())
                            .primary()
                            .disabled(!dirty)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_note(cx);
                            })),
                    ),
            )
    }
}

fn connection_notes_editor_style(colors: &TerminalColors) -> EditorStyleOverrides {
    EditorStyleOverrides {
        foreground: Some(colors.foreground),
        muted_foreground: Some(colors.muted_foreground),
        background: Some(colors.muted),
        border: Some(colors.border),
        selection: Some(colors.accent.opacity(0.35)),
        caret: Some(colors.foreground),
        editor_gutter_background: Some(colors.muted),
        ..Default::default()
    }
}

fn connection_with_note(mut connection: StoredConnection, note: &str) -> StoredConnection {
    connection.remark = (!note.trim().is_empty()).then(|| note.to_string());
    connection
}

#[cfg(test)]
mod tests {
    use super::{connection_notes_editor_style, connection_with_note};
    use crate::theme::TerminalColors;
    use gpui::rgb;
    use one_core::storage::models::{ExtensionConnectionParams, StoredConnection};
    use std::collections::BTreeMap;

    fn sample_connection() -> StoredConnection {
        StoredConnection::new_extension(
            "Production".to_string(),
            ExtensionConnectionParams::new(
                "example.extension",
                "production",
                serde_json::Map::new(),
                BTreeMap::new(),
            )
            .expect("extension params should be valid"),
            None,
        )
    }

    #[test]
    fn connection_notes_persist_through_the_existing_remark_field() {
        let saved = connection_with_note(sample_connection(), "Deploy user: ops\nRun: make deploy");

        assert_eq!(
            saved.remark.as_deref(),
            Some("Deploy user: ops\nRun: make deploy")
        );
    }

    #[test]
    fn whitespace_only_connection_notes_clear_the_remark() {
        let saved = connection_with_note(sample_connection(), "  \n\t ");

        assert_eq!(saved.remark, None);
    }

    #[test]
    fn connection_notes_editor_uses_terminal_theme_surface_colors() {
        let colors = TerminalColors {
            background: rgb(0x111827).into(),
            foreground: rgb(0xf9fafb).into(),
            muted: rgb(0x1f2937).into(),
            muted_foreground: rgb(0x9ca3af).into(),
            border: rgb(0x374151).into(),
            accent: rgb(0x2563eb).into(),
            accent_foreground: rgb(0xffffff).into(),
        };

        let style = connection_notes_editor_style(&colors);

        assert_eq!(style.background, Some(colors.muted));
        assert_eq!(style.foreground, Some(colors.foreground));
        assert_eq!(style.border, Some(colors.border));
        assert_eq!(style.caret, Some(colors.foreground));
    }

    #[test]
    fn connection_notes_panel_reuses_connection_persistence() {
        let source = include_str!("connection_notes_panel.rs");

        assert!(source.contains("Editor::new(&self.input)"));
        assert!(source.contains("EditorStyleOverrides"));
        assert!(source.contains("background: Some(self.colors.muted)"));
        assert!(source.contains("ConnectionRepository"));
        assert!(source.contains("ConnectionDataEvent::ConnectionUpdated"));
    }
}
