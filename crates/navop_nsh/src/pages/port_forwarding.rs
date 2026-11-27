use nsh_base::theme;
use nsh_core::prelude::*;
use one_core::storage::{ConnectionType, PortForwardingKind, StoredConnection};

use crate::{services::NavopNshServices, ui};

#[derive(Clone, PartialEq)]
struct ForwardChoice {
    id: i64,
    name: String,
    kind: PortForwardingKind,
    detail: String,
}

pub(crate) fn port_forwarding() -> impl IntoElement {
    let connections = NavopNshServices::current()
        .map(|services| {
            services
                .resolved_connections
                .iter()
                .filter_map(forward_choice)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let requested = crate::route::take_connection_request(crate::route::Route::PortForwarding);
    let initial = requested
        .and_then(|id| {
            connections
                .iter()
                .find(|connection| connection.id == id)
                .cloned()
        })
        .or_else(|| connections.first().cloned());
    let selected = signal(initial.map(|connection| connection.id));

    view()
        .size_full()
        .child(ui::page_header(
            "Port Forwarding",
            "Local, remote and dynamic SSH tunnels using the existing forwarding runtime",
        ))
        .child(
            view()
                .flex_1()
                .min_h_0()
                .p_4()
                .gap_3()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            text("TUNNELS")
                                .text_size(px(10.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().muted_foreground),
                        )
                        .children(
                            connections
                                .into_iter()
                                .map(move |connection| forward_button(connection, selected)),
                        ),
                )
                .when_some(selected.get(), |content, id| {
                    content.child(
                        native("navop.port_forwarding")
                            .prop("connection_id", serde_json::Value::from(id))
                            .flex_1()
                            .min_h_0(),
                    )
                })
                .when(selected.get().is_none(), |content| {
                    content.child(
                        ui::card().child(
                            text("No port-forwarding connection is available. Create one in Connections first.")
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        ),
                    )
                }),
        )
}

fn forward_choice(connection: &StoredConnection) -> Option<ForwardChoice> {
    if connection.connection_type != ConnectionType::PortForwarding {
        return None;
    }
    let id = connection.id?;
    let params = connection.to_port_forwarding_params().ok()?;
    let detail = match params.kind {
        PortForwardingKind::Local => format!(
            "{}:{} → {}:{}",
            params.bind_host, params.bind_port, params.target_host, params.target_port
        ),
        PortForwardingKind::Remote => format!(
            "{}:{} ← {}:{}",
            params.bind_host, params.bind_port, params.target_host, params.target_port
        ),
        PortForwardingKind::Dynamic => {
            format!("SOCKS {}:{}", params.bind_host, params.bind_port)
        }
    };
    Some(ForwardChoice {
        id,
        name: connection.name.clone(),
        kind: params.kind,
        detail,
    })
}

fn forward_button(connection: ForwardChoice, selected: Signal<Option<i64>>) -> impl IntoElement {
    let id = connection.id;
    let label = format!("{} · {}", connection.kind.label(), connection.name);
    let accessibility_label = label.clone();
    let detail = connection.detail;
    view()
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == Some(id) {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == Some(id) {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .label(accessibility_label)
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == Some(id),
            ..AccessibilityState::default()
        })
        .on_press(move || selected.set(Some(id)))
        .child(text(label).text_xs().text_color(move || theme().foreground))
        .child(
            text(detail)
                .text_size(px(9.0))
                .text_color(move || theme().muted_foreground),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forward(kind: PortForwardingKind) -> StoredConnection {
        let mut connection = StoredConnection::new_port_forwarding(
            "Tunnel".to_owned(),
            one_core::storage::PortForwardingParams {
                ssh_connection_id: 7,
                kind,
                bind_host: "127.0.0.1".to_owned(),
                bind_port: 8080,
                target_host: "db.internal".to_owned(),
                target_port: 5432,
            },
            None,
        );
        connection.id = Some(1);
        connection
    }

    #[test]
    fn forwarding_choices_describe_their_mode() {
        let local = forward_choice(&forward(PortForwardingKind::Local)).unwrap();
        assert!(local.detail.contains("→"));
        let remote = forward_choice(&forward(PortForwardingKind::Remote)).unwrap();
        assert!(remote.detail.contains("←"));
        let dynamic = forward_choice(&forward(PortForwardingKind::Dynamic)).unwrap();
        assert!(dynamic.detail.starts_with("SOCKS"));
    }
}
