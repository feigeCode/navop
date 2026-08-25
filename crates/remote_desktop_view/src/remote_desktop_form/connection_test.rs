use std::time::Duration;

use connection_form::credential::resolve_connection_for_runtime;
use gpui::prelude::FluentBuilder;
use gpui::{
    AppContext, AsyncApp, Context, IntoElement, ParentElement, Styled, WeakEntity, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Colorize as _, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement,
};
use one_core::{
    gpui_tokio::Tokio,
    storage::{RemoteDesktopBackendPreference, RemoteDesktopProtocol, StoredConnection},
};
use rust_i18n::t;

use super::RemoteDesktopFormWindow;

const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ConnectionTestResult {
    Success,
    /// The native RDP backend only probes TCP reachability; credentials are
    /// validated when the actual session starts.
    NativeReachable,
    Failure(String),
}

#[derive(Default)]
pub(super) struct ConnectionTestState {
    generation: u64,
    is_testing: bool,
    result: Option<ConnectionTestResult>,
}

impl ConnectionTestState {
    fn begin(&mut self) -> Option<u64> {
        if self.is_testing {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.is_testing = true;
        self.result = None;
        Some(self.generation)
    }

    fn complete(&mut self, generation: u64, result: ConnectionTestResult) -> bool {
        if generation != self.generation || !self.is_testing {
            return false;
        }
        self.is_testing = false;
        self.result = Some(result);
        true
    }

    fn fail_validation(&mut self, reason: String) {
        self.is_testing = false;
        self.result = Some(ConnectionTestResult::Failure(reason));
    }

    pub(super) fn is_testing(&self) -> bool {
        self.is_testing
    }

    pub(super) fn result(&self) -> Option<&ConnectionTestResult> {
        self.result.as_ref()
    }
}

impl RemoteDesktopFormWindow {
    pub(super) fn on_test_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection_test.is_testing() {
            return;
        }
        let params = match self.build_params(cx) {
            Ok(params) => params,
            Err(reason) => {
                self.connection_test.fail_validation(reason);
                cx.notify();
                return;
            }
        };
        let raw_options =
            remote_desktop::RemoteDesktopConnectionOptions::from_storage_params(params.clone());
        let uses_native_test = should_use_native_connection_test(&raw_options);
        let options = if uses_native_test || raw_options.proxy.is_some() {
            let mut params_for_test = params;
            if !uses_native_test {
                // An IronRDP reachability test may need proxy credentials, but
                // it must not resolve or validate the RDP account itself.
                params_for_test.credential_reference = None;
            }
            let params = match resolve_connection_for_runtime(
                StoredConnection::new_remote_desktop(
                    self.connection_name(&params_for_test, cx),
                    params_for_test,
                    None,
                ),
                cx,
            )
            .and_then(|connection| {
                connection
                    .to_remote_desktop_params()
                    .map_err(|error| error.to_string())
            }) {
                Ok(params) => params,
                Err(reason) => {
                    self.connection_test.fail_validation(reason);
                    cx.notify();
                    return;
                }
            };
            remote_desktop::RemoteDesktopConnectionOptions::from_storage_params(params)
        } else {
            raw_options
        };
        let Some(generation) = self.connection_test.begin() else {
            return;
        };
        self.error = None;
        cx.notify();

        let window_handle = window.window_handle();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let spawn_result = Tokio::spawn_result(cx, async move {
                let result = tokio::task::spawn_blocking(move || {
                    if uses_native_test {
                        test_native_rdp_reachability(&options, CONNECTION_TEST_TIMEOUT)
                    } else {
                        test_ironrdp_reachability(&options, CONNECTION_TEST_TIMEOUT)
                    }
                })
                .await?;
                result.map_err(anyhow::Error::msg)
            })
            .await;
            let result = match spawn_result {
                Ok(()) if uses_native_test => ConnectionTestResult::NativeReachable,
                Ok(()) => ConnectionTestResult::Success,
                Err(error) => ConnectionTestResult::Failure(error.to_string()),
            };

            let _ = cx.update_window(window_handle, |_, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    if this.connection_test.complete(generation, result) {
                        cx.notify();
                    }
                });
            });
        })
        .detach();
    }

    pub(super) fn render_connection_test_result(
        &self,
        result: ConnectionTestResult,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match result {
            ConnectionTestResult::Success | ConnectionTestResult::NativeReachable => {
                h_flex().justify_center().px_6().pb_2().child(
                    div().text_sm().text_color(cx.theme().success).child(
                        if matches!(result, ConnectionTestResult::NativeReachable) {
                            t!("RemoteDesktopForm.native_reachable")
                        } else {
                            t!("RemoteDesktopForm.credentials_valid")
                        }
                        .to_string(),
                    ),
                )
            }
            ConnectionTestResult::Failure(reason) => h_flex().px_6().pb_2().child(
                div()
                    .w_full()
                    .max_h(px(120.0))
                    .overflow_y_scrollbar()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(cx.theme().danger.opacity(0.12))
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(reason),
            ),
        }
    }

    pub(super) fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_testing = self.connection_test.is_testing();
        h_flex()
            .justify_end()
            .gap_2()
            .px_6()
            .py_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .when(self.protocol == RemoteDesktopProtocol::Rdp, |footer| {
                footer.child(
                    Button::new("test-remote-desktop")
                        .small()
                        .outline()
                        .loading(is_testing)
                        .disabled(is_testing)
                        .label(
                            if is_testing {
                                t!("RemoteDesktopForm.testing_connection")
                            } else {
                                t!("RemoteDesktopForm.test_connection")
                            }
                            .to_string(),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.on_test_connection(window, cx);
                        })),
                )
            })
            .child(
                Button::new("cancel-remote-desktop")
                    .small()
                    .label(t!("Common.cancel").to_string())
                    .on_click(cx.listener(|_, _, window, _| window.remove_window())),
            )
            .child(
                Button::new("save-remote-desktop")
                    .small()
                    .primary()
                    .disabled(is_testing)
                    .label(t!("Common.ok").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.on_save(window, cx))),
            )
    }
}

/// Whether the connection test should probe the target with a plain TCP
/// reachability check instead of launching the helper-based backend.
///
/// The Windows native MSTSC backend embeds the RDP control directly in the
/// app process and never spawns the `onetcli-rdp-helper` process, so
/// requiring that helper to be installed just to test the connection would
/// mislead users into downloading an extension they do not need. When the
/// user has explicitly selected the native backend (and the build actually
/// compiled it, and no SOCKS/HTTP proxy is configured — native RDP cannot
/// tunnel through a proxy), we probe the destination with a lightweight TCP
/// connect instead.
fn should_use_native_connection_test(
    options: &remote_desktop::RemoteDesktopConnectionOptions,
) -> bool {
    cfg!(all(feature = "windows-native-rdp", target_os = "windows"))
        && options.backend_preference == RemoteDesktopBackendPreference::WindowsNative
        && options.proxy.is_none()
}

/// Probe an IronRDP destination with a TCP connect instead of starting the
/// helper and performing a complete RDP/TLS/NLA login.
fn test_ironrdp_reachability(
    options: &remote_desktop::RemoteDesktopConnectionOptions,
    timeout: Duration,
) -> Result<(), String> {
    if let Some(proxy) = options.proxy.as_ref() {
        let (host, port) = remote_desktop::backend::parse_destination(&options.destination)
            .map_err(|error| error.to_string())?;
        return connection_tunnel::test_proxy_reachability(proxy, &host, port, timeout)
            .map_err(|error| error.to_string());
    }
    let (host, port) = remote_desktop::backend::parse_destination(&options.destination)
        .map_err(|error| error.to_string())?;
    connection_tunnel::test_tcp_reachability(&host, port, timeout)
        .map_err(|error| error.to_string())
}

/// Probe the RDP destination with a TCP connect. This verifies that the host
/// resolves and the RDP port is reachable, which is the closest side-effect
/// free check available for the native MSTSC backend: a real credential
/// validation would require starting an actual RDP session.
fn test_native_rdp_reachability(
    options: &remote_desktop::RemoteDesktopConnectionOptions,
    timeout: Duration,
) -> Result<(), String> {
    use std::net::{TcpStream, ToSocketAddrs};

    let (host, port) = remote_desktop::backend::parse_destination(&options.destination)
        .map_err(|error| error.to_string())?;

    let socket_addrs = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| format!("failed to resolve {host}:{port}: {error}"))?;

    let mut last_error = None::<String>;
    for socket_addr in socket_addrs {
        match TcpStream::connect_timeout(&socket_addr, timeout) {
            Ok(_) => return Ok(()),
            Err(error) => {
                last_error = Some(format!("{host}:{port} ({socket_addr}): {error}"));
            }
        }
    }

    Err(match last_error {
        Some(detail) => format!("could not connect to {host}:{port}: {detail}"),
        None => format!("could not resolve any address for {host}:{port}"),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CONNECTION_TEST_TIMEOUT, ConnectionTestResult, ConnectionTestState,
        should_use_native_connection_test, test_ironrdp_reachability, test_native_rdp_reachability,
    };
    use one_core::storage::RemoteDesktopBackendPreference;

    fn native_test_options(
        backend: RemoteDesktopBackendPreference,
    ) -> remote_desktop::RemoteDesktopConnectionOptions {
        remote_desktop::RemoteDesktopConnectionOptions::from_storage_params(
            one_core::storage::RemoteDesktopParams {
                protocol: one_core::storage::RemoteDesktopProtocol::Rdp,
                host: "127.0.0.1".to_string(),
                port: 3389,
                username: None,
                password: None,
                credential_reference: None,
                domain: None,
                read_only: false,
                audio_playback: false,
                proxy: None,
                backend_preference: backend,
                rdp: None,
            },
        )
    }

    #[test]
    fn repeated_begin_is_rejected_while_test_is_running() {
        let mut state = ConnectionTestState::default();

        assert_eq!(Some(1), state.begin());
        assert_eq!(None, state.begin());
    }

    #[test]
    fn stale_completion_does_not_replace_current_test() {
        let mut state = ConnectionTestState::default();
        let first = state.begin().unwrap();
        assert!(state.complete(first, ConnectionTestResult::Success));
        let second = state.begin().unwrap();

        assert!(!state.complete(first, ConnectionTestResult::Failure("stale".to_string())));
        assert!(state.is_testing());
        assert_eq!(None, state.result());
        assert_eq!(2, second);
    }

    #[test]
    fn current_completion_updates_state() {
        let mut state = ConnectionTestState::default();
        let generation = state.begin().unwrap();

        assert!(state.complete(generation, ConnectionTestResult::Success));
        assert!(!state.is_testing());
        assert_eq!(Some(&ConnectionTestResult::Success), state.result());
    }

    #[test]
    fn new_test_clears_previous_result() {
        let mut state = ConnectionTestState::default();
        let generation = state.begin().unwrap();
        state.complete(generation, ConnectionTestResult::Success);

        assert_eq!(Some(2), state.begin());
        assert_eq!(None, state.result());
    }

    #[test]
    fn native_connection_test_selected_for_windows_native_preference() {
        let options = native_test_options(RemoteDesktopBackendPreference::WindowsNative);

        assert_eq!(
            should_use_native_connection_test(&options),
            cfg!(all(feature = "windows-native-rdp", target_os = "windows"))
        );
    }

    #[test]
    fn native_connection_test_skipped_for_canvas_preference() {
        let options = native_test_options(RemoteDesktopBackendPreference::Canvas);

        assert!(!should_use_native_connection_test(&options));
    }

    #[test]
    fn native_connection_test_skipped_for_auto_preference() {
        // Auto may fall back to Canvas (helper) when the native probe is
        // unavailable, so the connection test stays on the helper path.
        let options = native_test_options(RemoteDesktopBackendPreference::Auto);

        assert!(!should_use_native_connection_test(&options));
    }

    #[test]
    fn native_reachability_reports_missing_port() {
        let mut options = native_test_options(RemoteDesktopBackendPreference::WindowsNative);
        options.destination = "just-a-host".to_string();

        let error = test_native_rdp_reachability(&options, CONNECTION_TEST_TIMEOUT)
            .expect_err("destination without a port must fail");

        assert!(error.contains("port"));
    }

    #[test]
    fn native_reachability_reports_unreachable_host() {
        let mut options = native_test_options(RemoteDesktopBackendPreference::WindowsNative);
        // RFC 5737 TEST-NET-1 address: guaranteed not to route.
        options.destination = "192.0.2.1:3389".to_string();

        let error = test_native_rdp_reachability(&options, std::time::Duration::from_millis(300))
            .expect_err("test address must not accept connections");

        assert!(error.contains("192.0.2.1"));
    }

    #[test]
    fn ironrdp_reachability_accepts_a_listening_target() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test listener");
        let destination = listener.local_addr().unwrap().to_string();

        for backend in [
            RemoteDesktopBackendPreference::Auto,
            RemoteDesktopBackendPreference::Canvas,
        ] {
            let mut options = native_test_options(backend);
            options.destination = destination.clone();
            test_ironrdp_reachability(&options, CONNECTION_TEST_TIMEOUT)
                .expect("a listening TCP target must be reachable");
        }
    }

    #[test]
    fn ironrdp_reachability_reports_a_closed_port() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve local test port");
        let destination = listener.local_addr().unwrap().to_string();
        drop(listener);

        let mut options = native_test_options(RemoteDesktopBackendPreference::Canvas);
        options.destination = destination.clone();

        let error = test_ironrdp_reachability(&options, std::time::Duration::from_secs(1))
            .expect_err("closed local port must reject connections");

        assert!(error.contains(&destination));
    }

    #[test]
    fn ironrdp_reachability_honors_proxy_configuration() {
        let mut options = native_test_options(RemoteDesktopBackendPreference::Canvas);
        options.proxy = Some(connection_tunnel::ProxyTunnelConfig {
            proxy_type: connection_tunnel::ProxyTunnelType::Socks5,
            host: String::new(),
            port: 1080,
            username: None,
            password: None,
        });

        let error = test_ironrdp_reachability(&options, CONNECTION_TEST_TIMEOUT)
            .expect_err("an invalid proxy must not fall back to a direct target probe");

        assert!(error.contains("proxy `host` is required"));
    }
}
