use gpui::{FontWeight, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use rust_i18n::t;
use std::path::PathBuf;
use terminal::recording::{RecordingBackend, RecordingCompleteness, SessionLogEntry};

use super::{
    SessionLogsPage,
    actions::FavoriteChange,
    model::{format_duration, format_started_at, local_identity, remote_identity},
};

struct RowActions {
    recording_id: String,
    favorite: bool,
    path: PathBuf,
}

impl SessionLogsPage {
    pub(super) fn render_entry(
        &self,
        entry: SessionLogEntry,
        cx: &gpui::Context<Self>,
    ) -> impl IntoElement {
        let path_text = entry.path.to_string_lossy().to_string();
        let actions = RowActions {
            recording_id: entry.header.navop.recording_id.clone(),
            favorite: entry.favorite,
            path: entry.path.clone(),
        };
        v_flex()
            .w_full()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .p_3()
            .child(self.render_entry_top(&entry, actions, cx))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(path_text),
            )
    }

    fn render_entry_top(
        &self,
        entry: &SessionLogEntry,
        actions: RowActions,
        cx: &gpui::Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_3()
            .child(backend_icon(entry.header.navop.backend, cx))
            .child(entry_summary(entry, cx))
            .child(self.render_entry_actions(actions, cx))
    }

    fn render_entry_actions(
        &self,
        actions: RowActions,
        cx: &gpui::Context<Self>,
    ) -> impl IntoElement {
        let replay_path = actions.path.clone();
        let export_path = actions.path;
        h_flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .child(self.favorite_button(actions.recording_id.clone(), actions.favorite, cx))
            .child(view_log_button(
                actions.recording_id.clone(),
                replay_path,
                cx,
            ))
            .child(export_button(actions.recording_id, export_path, cx))
    }

    fn favorite_button(
        &self,
        recording_id: String,
        favorite: bool,
        cx: &gpui::Context<Self>,
    ) -> Button {
        Button::new(button_id("session-log-favorite", &recording_id))
            .icon(if favorite {
                IconName::StarFill
            } else {
                IconName::Star
            })
            .small()
            .ghost()
            .disabled(self.favorite_saving)
            .tooltip(if favorite {
                t!("SessionLogs.unfavorite").to_string()
            } else {
                t!("SessionLogs.favorite").to_string()
            })
            .on_click(cx.listener(move |page, _, window, cx| {
                page.toggle_favorite(
                    FavoriteChange {
                        recording_id: recording_id.clone(),
                        favorite: !favorite,
                    },
                    window,
                    cx,
                );
            }))
    }
}

fn entry_summary(entry: &SessionLogEntry, cx: &gpui::Context<SessionLogsPage>) -> impl IntoElement {
    let (status, status_color) = completeness(entry, cx);
    v_flex()
        .min_w_0()
        .flex_1()
        .gap_1()
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .font_weight(FontWeight::MEDIUM)
                        .child(entry_title(entry)),
                )
                .child(status_chip(status, status_color, cx)),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(entry_details(entry)),
        )
}

fn view_log_button(
    recording_id: String,
    path: PathBuf,
    cx: &gpui::Context<SessionLogsPage>,
) -> Button {
    Button::new(button_id("session-log-view", &recording_id))
        .icon(IconName::Eye)
        .small()
        .ghost()
        .tooltip(t!("SessionLogs.view_log").to_string())
        .on_click(cx.listener(move |page, _, window, cx| {
            page.view_log(path.clone(), window, cx);
        }))
}

fn export_button(
    recording_id: String,
    path: PathBuf,
    cx: &gpui::Context<SessionLogsPage>,
) -> Button {
    Button::new(button_id("session-log-export", &recording_id))
        .icon(IconName::Upload)
        .small()
        .ghost()
        .tooltip(t!("SessionLogs.export_text").to_string())
        .on_click(cx.listener(move |page, _, window, cx| {
            page.request_text_export(path.clone(), window, cx);
        }))
}

fn entry_title(entry: &SessionLogEntry) -> String {
    entry
        .header
        .navop
        .session
        .as_ref()
        .and_then(|session| session.connection_name.clone())
        .unwrap_or_else(|| backend_name(entry.header.navop.backend))
}

fn entry_details(entry: &SessionLogEntry) -> String {
    let mut parts = vec![
        format_started_at(entry.header.navop.started_at_unix_ms),
        format_duration(entry.duration),
        backend_name(entry.header.navop.backend),
    ];
    parts.extend(
        local_identity(entry).map(|value| format!("{}: {value}", t!("SessionLogs.local_identity"))),
    );
    parts.extend(remote_detail(entry));
    parts.join(" · ")
}

fn remote_detail(entry: &SessionLogEntry) -> Option<String> {
    match entry.header.navop.backend {
        RecordingBackend::Ssh => remote_identity(entry)
            .map(|value| format!("{}: {value}", t!("SessionLogs.remote_identity"))),
        RecordingBackend::Serial => {
            serial_port(entry).map(|value| format!("{}: {value}", t!("SessionLogs.serial_port")))
        }
        RecordingBackend::Telnet => remote_identity(entry)
            .map(|value| format!("{}: {value}", t!("SessionLogs.remote_identity"))),
        RecordingBackend::Local => None,
    }
}

fn serial_port(entry: &SessionLogEntry) -> Option<String> {
    entry
        .header
        .navop
        .session
        .as_ref()
        .and_then(|session| session.serial_port.clone())
}

fn backend_name(backend: RecordingBackend) -> String {
    match backend {
        RecordingBackend::Local => t!("SessionLogs.local").to_string(),
        RecordingBackend::Ssh => t!("SessionLogs.ssh").to_string(),
        RecordingBackend::Serial => t!("SessionLogs.serial").to_string(),
        RecordingBackend::Telnet => t!("SessionLogs.telnet").to_string(),
    }
}

fn backend_icon(backend: RecordingBackend, cx: &gpui::App) -> impl IntoElement {
    let icon = match backend {
        RecordingBackend::Local => IconName::Terminal,
        RecordingBackend::Ssh => IconName::SquareTerminal,
        RecordingBackend::Serial => IconName::SerialPort,
        RecordingBackend::Telnet => IconName::SquareTerminal,
    };
    div()
        .flex_shrink_0()
        .w(px(36.0))
        .h(px(36.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .bg(cx.theme().muted)
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(icon))
}

fn completeness(entry: &SessionLogEntry, cx: &gpui::App) -> (String, gpui::Hsla) {
    match entry.completeness {
        RecordingCompleteness::Complete => {
            (t!("SessionLogs.complete").to_string(), cx.theme().success)
        }
        RecordingCompleteness::Partial { .. } => {
            (t!("SessionLogs.partial").to_string(), cx.theme().warning)
        }
    }
}

fn status_chip(label: String, color: gpui::Hsla, cx: &gpui::App) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .rounded_full()
        .border_1()
        .border_color(color)
        .px_2()
        .py_0p5()
        .text_xs()
        .text_color(color)
        .bg(cx.theme().background)
        .child(label)
}

fn button_id(prefix: &str, recording_id: &str) -> gpui::SharedString {
    format!("{prefix}-{recording_id}").into()
}
