use nsh_base::theme;
use nsh_core::prelude::*;
use one_core::storage::{ConnectionType, StoredConnection};
use terminal_view::HostMonitorSession;

use crate::{services::NavopNshServices, ui};

#[derive(Clone, PartialEq)]
struct MonitorMetrics {
    os: String,
    cpu: String,
    memory: String,
    disk: String,
    network: String,
    processes: String,
    cores: String,
    uptime: String,
}

pub(crate) fn monitor() -> impl IntoElement {
    let connections = NavopNshServices::current()
        .map(|services| {
            services
                .resolved_connections
                .clone()
                .into_iter()
                .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let count = connections.len();

    view()
        .size_full()
        .child(ui::page_header(
            "Host Monitor",
            format!("One-shot SSH resource sampling for {count} saved host(s)"),
        ))
        .child(
            scroll().flex_1().child(
                view()
                    .p_6()
                    .gap_3()
                    .when(connections.is_empty(), |page| {
                        page.child(
                            ui::card().child(
                                text("No SSH connections are available for monitoring")
                                    .text_sm()
                                    .text_color(move || theme().muted_foreground),
                            ),
                        )
                    })
                    .children(connections.into_iter().map(monitor_row)),
            ),
        )
}

fn monitor_row(connection: StoredConnection) -> impl IntoElement {
    let address = connection
        .to_ssh_params()
        .map(|params| format!("{}@{}:{}", params.username, params.host, params.port))
        .unwrap_or_else(|_| "Invalid SSH connection".to_owned());
    let title = connection.name.clone();
    let status = signal(String::from("Idle · press Sample"));
    let busy = signal(false);
    let metrics = signal(None::<MonitorMetrics>);
    let sample = {
        let connection = connection.clone();
        move || {
            let connection = connection.clone();
            busy.set(true);
            status.set("Connecting and sampling…".to_owned());
            spawn(async move {
                let result = background(async move {
                    let params = connection.to_ssh_params()?;
                    if params.prompts_for_username()
                        || (params.prompts_for_password() && params.auth_method.contains_password())
                    {
                        anyhow::bail!(
                            "This host requires interactive credentials; unlock them before monitoring"
                        );
                    }
                    let keyboard_interactive = params.keyboard_interactive_enabled();
                    let config =
                        terminal::terminal::ssh_terminal_config_from_connection(&connection)?;
                    let mut session = HostMonitorSession::new(config, keyboard_interactive);
                    let snapshot = session.sample().await;
                    session.disconnect().await;
                    snapshot
                })
                .await;
                busy.set(false);
                match result {
                    Ok(Ok(snapshot)) => {
                        status.set("Sampled successfully".to_owned());
                        metrics.set(Some(MonitorMetrics {
                            os: snapshot
                                .os_name
                                .clone()
                                .unwrap_or_else(|| "Unknown OS".to_owned()),
                            cpu: snapshot
                                .cpu_percent
                                .map_or_else(|| "n/a".to_owned(), |value| format!("{value:.1}%")),
                            memory: snapshot
                                .memory_percent()
                                .map_or_else(|| "n/a".to_owned(), |value| format!("{value:.1}%")),
                            disk: snapshot
                                .disks
                                .iter()
                                .max_by(|left, right| left.percent.total_cmp(&right.percent))
                                .map_or_else(
                                    || "n/a".to_owned(),
                                    |disk| format!("{:.1}% {}", disk.percent, disk.mount),
                                ),
                            network: match (
                                snapshot.network_rx_bytes_per_sec,
                                snapshot.network_tx_bytes_per_sec,
                            ) {
                                (Some(rx), Some(tx)) => {
                                    format!("↓ {} · ↑ {}", rate(rx), rate(tx))
                                }
                                _ => "n/a".to_owned(),
                            },
                            processes: snapshot
                                .process_count
                                .map_or_else(|| "n/a".to_owned(), |value| value.to_string()),
                            cores: snapshot
                                .cpu_core_count()
                                .map_or_else(|| "n/a".to_owned(), |value| value.to_string()),
                            uptime: snapshot.uptime_seconds.map_or_else(
                                || "n/a".to_owned(),
                                |seconds| duration_label(seconds),
                            ),
                        }));
                    }
                    Ok(Err(error)) => status.set(format!("Sampling failed: {error:#}")),
                    Err(error) => status.set(format!("Sampling task failed: {error}")),
                }
            });
        }
    };

    ui::card()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    view()
                        .flex_1()
                        .min_w_0()
                        .child(
                            text(title)
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(address)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            text(move || status.get())
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(
                    ui::primary_button("Sample")
                        .disabled(move || busy.get())
                        .on_press(sample),
                ),
        )
        .when_some(metrics.get(), |card, metrics| {
            card.child(view().flex_row().flex_wrap().gap_2().children([
                metric("OS", metrics.os),
                metric("CPU", metrics.cpu),
                metric("Memory", metrics.memory),
                metric("Disk", metrics.disk),
                metric("Network", metrics.network),
                metric("Processes", metrics.processes),
                metric("Cores", metrics.cores),
                metric("Uptime", metrics.uptime),
            ]))
        })
}

fn metric(label_text: &'static str, value: String) -> impl IntoElement {
    view()
        .min_w(px(130.0))
        .flex_1()
        .p_3()
        .gap_1()
        .rounded(px(8.0))
        .bg(move || theme().muted)
        .child(
            text(label_text)
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(value)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
}

fn rate(bytes_per_second: f64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    if bytes_per_second >= GIB {
        format!("{:.1} GiB/s", bytes_per_second / GIB)
    } else if bytes_per_second >= MIB {
        format!("{:.1} MiB/s", bytes_per_second / MIB)
    } else if bytes_per_second >= KIB {
        format!("{:.1} KiB/s", bytes_per_second / KIB)
    } else {
        format!("{bytes_per_second:.0} B/s")
    }
}

fn duration_label(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else {
        format!("{hours}h {minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_and_uptime_are_human_readable() {
        assert_eq!(rate(512.0), "512 B/s");
        assert_eq!(rate(1536.0), "1.5 KiB/s");
        assert_eq!(duration_label(90_061.0), "1d 1h 1m");
    }
}
