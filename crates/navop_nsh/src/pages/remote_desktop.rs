use nsh_base::theme;
use nsh_core::prelude::*;
use one_core::storage::{ConnectionType, StoredConnection};

use crate::{services::NavopNshServices, ui};

#[derive(Clone, PartialEq)]
struct RemoteChoice {
    id: i64,
    name: String,
    protocol: &'static str,
}

pub(crate) fn remote_desktop() -> impl IntoElement {
    let connections = NavopNshServices::current()
        .map(|services| {
            services
                .resolved_connections
                .iter()
                .filter(|connection| {
                    matches!(
                        connection.connection_type,
                        ConnectionType::Rdp | ConnectionType::Vnc
                    )
                })
                .filter_map(remote_choice)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let requested = crate::route::take_connection_request(crate::route::Route::RemoteDesktop);
    let initial = requested
        .and_then(|id| {
            connections
                .iter()
                .find(|connection| connection.id == id)
                .cloned()
        })
        .or_else(|| connections.first().cloned());
    let selected = signal(initial.as_ref().map(|connection| connection.id));

    view()
        .size_full()
        .child(ui::page_header(
            "Remote Desktop",
            "RDP and VNC sessions using the existing GPUI remote desktop view",
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
                            text("CONNECTIONS")
                                .text_size(px(10.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().muted_foreground),
                        )
                        .children(
                            connections
                                .into_iter()
                                .map(move |connection| remote_button(connection, selected)),
                        ),
                )
                .when_some(selected.get(), |content, id| {
                    content.child(
                        native("navop.remote_desktop")
                            .prop("connection_id", serde_json::Value::from(id))
                            .flex_1()
                            .min_h_0(),
                    )
                })
                .when(selected.get().is_none(), |content| {
                    content.child(
                        ui::card().child(
                            text("No RDP or VNC connection is available. Create one in Connections first.")
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        ),
                    )
                }),
        )
}

fn remote_choice(connection: &StoredConnection) -> Option<RemoteChoice> {
    let protocol = match connection.connection_type {
        ConnectionType::Rdp => "RDP",
        ConnectionType::Vnc => "VNC",
        _ => return None,
    };
    Some(RemoteChoice {
        id: connection.id?,
        name: connection.name.clone(),
        protocol,
    })
}

fn remote_button(connection: RemoteChoice, selected: Signal<Option<i64>>) -> impl IntoElement {
    let id = connection.id;
    let label = format!("{} · {}", connection.protocol, connection.name);
    let accessibility_label = label.clone();
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::{RemoteDesktopParams, RemoteDesktopProtocol};

    fn remote(name: &str, protocol: RemoteDesktopProtocol, id: i64) -> StoredConnection {
        let mut connection = StoredConnection::new_remote_desktop(
            name.to_owned(),
            RemoteDesktopParams {
                protocol,
                host: "example.com".to_owned(),
                port: 3389,
                username: None,
                password: None,
                credential_reference: None,
                domain: None,
                read_only: false,
                audio_playback: false,
                proxy: None,
                backend_preference: Default::default(),
                rdp: None,
            },
            None,
        );
        connection.id = Some(id);
        connection
    }

    #[test]
    fn remote_choices_include_rdp_and_vnc_only() {
        let rdp = remote("Windows", RemoteDesktopProtocol::Rdp, 1);
        let vnc = remote("Linux", RemoteDesktopProtocol::Vnc, 2);
        assert_eq!(remote_choice(&rdp).unwrap().protocol, "RDP");
        assert_eq!(remote_choice(&vnc).unwrap().protocol, "VNC");
        let mut not_remote = rdp;
        not_remote.connection_type = ConnectionType::SshSftp;
        assert!(remote_choice(&not_remote).is_none());
    }
}
