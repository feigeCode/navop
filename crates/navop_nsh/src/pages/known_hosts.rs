use nsh_base::theme;
use nsh_core::prelude::*;
use ssh::KnownHost;

use crate::{services::NavopNshServices, ui};

pub(crate) fn known_hosts() -> impl IntoElement {
    let services = NavopNshServices::current();
    let hosts = signal(load_hosts(services.as_deref()));
    let status = signal(String::new());
    let refresh = {
        let services = services.clone();
        move || {
            hosts.set(load_hosts(services.as_deref()));
            status.set(String::new());
        }
    };
    let scan = {
        let services = services.clone();
        move || {
            let Some(services) = services.as_ref() else {
                status.set("Host-key storage is unavailable".to_owned());
                return;
            };
            let Some(path) = services.known_hosts.openssh_known_hosts_path() else {
                status.set("OpenSSH known_hosts was not found".to_owned());
                return;
            };
            match services.known_hosts.import_system_known_hosts(path) {
                Ok(_) => {
                    hosts.set(load_hosts(Some(services)));
                    status.set("OpenSSH known_hosts imported".to_owned());
                }
                Err(error) => status.set(error),
            }
        }
    };

    view()
        .size_full()
        .child(ui::page_header(
            "Known Hosts",
            "Trusted SSH server keys managed by the shared host-key store",
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
                        .gap_2()
                        .child(ui::secondary_button("Scan OpenSSH").on_press(scan))
                        .child(ui::secondary_button("Refresh").on_press(refresh)),
                )
                .child(
                    text(move || status.get())
                        .text_sm()
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    scroll()
                        .flex_1()
                        .min_h_0()
                        .gap_2()
                        .children(hosts.with(|hosts| hosts.clone()).into_iter().map(host_row)),
                ),
        )
}

fn load_hosts(services: Option<&NavopNshServices>) -> Vec<KnownHost> {
    services
        .and_then(|services| services.known_hosts.list_known_hosts().ok())
        .unwrap_or_default()
}

fn host_row(host: KnownHost) -> impl IntoElement {
    let label = host.identity.host().to_owned();
    let port = host.identity.port();
    let identity = host.identity.to_string();
    ui::card().flex_row().items_center().gap_4().child(
        view()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                text(format!("{label}:{port}"))
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().foreground),
            )
            .child(
                text(format!("{} · {}", host.algorithm, host.fingerprint))
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
            )
            .child(
                text(identity)
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
            ),
    )
}
