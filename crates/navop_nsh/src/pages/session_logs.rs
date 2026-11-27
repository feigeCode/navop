use std::path::PathBuf;
use std::time::Duration;

use chrono::{Local, TimeZone};
use nsh_base::theme;
use nsh_core::prelude::*;
use terminal::recording::{
    RecordingBackend, RecordingCompleteness, RecordingFileLimits, SessionLogEntry,
    load_session_log_favorites, scan_session_logs,
};

use crate::{services::NavopNshServices, ui};

#[derive(Clone, PartialEq)]
struct SessionLogRow {
    path: PathBuf,
    started_at: String,
    backend: &'static str,
    duration: Duration,
    complete: bool,
    favorite: bool,
    identity: String,
}

pub(crate) fn session_logs() -> impl IntoElement {
    let directory =
        NavopNshServices::current().and_then(|services| services.session_logs_dir.clone());
    let rows = signal(Vec::<SessionLogRow>::new());
    let status = signal(String::from("Loading session logs…"));
    let busy = signal(false);
    let refresh = {
        let directory = directory.clone();
        move || reload(directory.as_ref(), rows, status, busy)
    };
    refresh();

    view()
        .size_full()
        .child(ui::page_header(
            "Session Logs",
            "Terminal history stored in the shared Navop recording catalog",
        ))
        .child(
            view()
                .flex_1()
                .min_h_0()
                .p_6()
                .gap_3()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            text(move || status.get())
                                .flex_1()
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            ui::secondary_button("Refresh")
                                .disabled(move || busy.get())
                                .on_press(refresh.clone()),
                        ),
                )
                .child(
                    scroll()
                        .flex_1()
                        .min_h_0()
                        .gap_2()
                        .children(rows.with(|rows| rows.clone()).into_iter().map(log_row)),
                ),
        )
}

fn reload(
    directory: Option<&PathBuf>,
    rows: Signal<Vec<SessionLogRow>>,
    status: Signal<String>,
    busy: Signal<bool>,
) {
    let Some(directory) = directory.cloned() else {
        status.set("Session-log directory is unavailable".to_owned());
        return;
    };
    busy.set(true);
    status.set("Scanning session logs…".to_owned());
    spawn(async move {
        let scan_directory = directory.clone();
        let result = background(async move {
            let favorites = load_session_log_favorites(&scan_directory)?;
            scan_session_logs(&scan_directory, RecordingFileLimits::default(), &favorites)
        })
        .await;
        busy.set(false);
        match result {
            Ok(Ok(catalog)) => {
                let count = catalog.entries.len();
                let skipped = catalog.skipped.len();
                rows.set(catalog.entries.into_iter().map(to_row).collect());
                status.set(if skipped == 0 {
                    format!("{count} session log(s)")
                } else {
                    format!("{count} session log(s) · {skipped} skipped")
                });
            }
            Ok(Err(error)) => status.set(format!("Failed to scan session logs: {error}")),
            Err(error) => status.set(format!("Session-log task failed: {error}")),
        }
    });
}

fn to_row(entry: SessionLogEntry) -> SessionLogRow {
    let started_at = i64::try_from(entry.header.navop.started_at_unix_ms)
        .ok()
        .and_then(|millis| Local.timestamp_millis_opt(millis).single())
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| entry.header.navop.started_at_unix_ms.to_string());
    let session = entry.header.navop.session.as_ref();
    let identity = session
        .and_then(|session| {
            session.connection_name.clone().or_else(|| {
                match (&session.remote_user, &session.remote_host) {
                    (Some(user), Some(host)) => Some(format!("{user}@{host}")),
                    (None, Some(host)) => Some(host.clone()),
                    _ => session.local_host.clone(),
                }
            })
        })
        .unwrap_or_else(|| "Local session".to_owned());
    SessionLogRow {
        path: entry.path,
        started_at,
        backend: backend_label(entry.header.navop.backend),
        duration: entry.duration,
        complete: matches!(entry.completeness, RecordingCompleteness::Complete),
        favorite: entry.favorite,
        identity,
    }
}

fn backend_label(backend: RecordingBackend) -> &'static str {
    match backend {
        RecordingBackend::Local => "Local",
        RecordingBackend::Ssh => "SSH",
        RecordingBackend::Serial => "Serial",
        RecordingBackend::Telnet => "Telnet",
    }
}

fn log_row(row: SessionLogRow) -> impl IntoElement {
    let path = row.path.display().to_string();
    ui::card()
        .flex_row()
        .items_center()
        .gap_4()
        .child(
            text(if row.favorite { "★" } else { "•" })
                .text_size(px(16.0))
                .text_color(move || {
                    if row.favorite {
                        theme().chart_2
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
        .child(
            view()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    text(format!("{} · {}", row.backend, row.identity))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(format!(
                        "{} · {} · {}{}",
                        row.started_at,
                        duration_label(row.duration),
                        if row.complete { "complete" } else { "partial" },
                        if row.favorite { " · favorite" } else { "" }
                    ))
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(path)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
}

fn duration_label(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 3600 {
        format!(
            "{}h {:02}m {:02}s",
            seconds / 3600,
            (seconds % 3600) / 60,
            seconds % 60
        )
    } else if seconds >= 60 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_labels_cover_seconds_minutes_and_hours() {
        assert_eq!(duration_label(Duration::from_secs(9)), "9s");
        assert_eq!(duration_label(Duration::from_secs(61)), "1m 01s");
        assert_eq!(duration_label(Duration::from_secs(3661)), "1h 01m 01s");
    }
}
