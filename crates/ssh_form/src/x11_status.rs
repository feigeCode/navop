//! SSH 测试连接的结构化 X11 状态。

use one_core::settings::{X11ForwardingStatus, X11ServerMode};
use rust_i18n::t;
use ssh::{SshChannel as _, SshClient, X11RequestRejected};

const REMOTE_GUIDANCE_KEYS: [&str; 2] = [
    "SSH.x11_remote_forwarding_disabled_checklist",
    "SSH.x11_untrusted_ssh_x_security",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum X11RequestOutcome {
    NotAttempted,
    Accepted,
    Rejected,
    Failed,
}

#[derive(Clone, Debug)]
pub(crate) struct X11Probe {
    pub(crate) outcome: X11RequestOutcome,
    pub(crate) status: Option<X11ForwardingStatus>,
}

impl X11Probe {
    pub(crate) fn accepted(&self) -> bool {
        self.outcome == X11RequestOutcome::Accepted
    }
}

/// 用临时 session channel 发送真实 x11-req；任何 X11 错误都只降级 X11 状态。
pub(crate) async fn probe_connection_x11<C: SshClient>(client: &mut C) -> X11Probe {
    let Some(proxy) = client.x11_forwarding().cloned() else {
        return X11Probe {
            outcome: X11RequestOutcome::NotAttempted,
            status: client.x11_status(),
        };
    };

    let ready_status = client.x11_status();
    let mut channel = match client.open_channel().await {
        Ok(channel) => channel,
        Err(error) => {
            return X11Probe {
                outcome: X11RequestOutcome::Failed,
                status: Some(X11ForwardingStatus::Disabled {
                    reason: t!("SSH.x11_channel_open_failed", error = format!("{error:#}"))
                        .to_string(),
                }),
            };
        }
    };

    let request = proxy.issue_request(false);
    let result = channel.request_x11_forwarding(&request).await;
    // 测试 cookie 不会被后续终端复用，成功与失败都立即撤销授权。
    proxy.retract_request(&request);
    let _ = channel.close().await;

    match result {
        Ok(()) => X11Probe {
            outcome: X11RequestOutcome::Accepted,
            status: ready_status,
        },
        Err(error) => {
            let rejected = error.downcast_ref::<X11RequestRejected>().is_some();
            X11Probe {
                outcome: if rejected {
                    X11RequestOutcome::Rejected
                } else {
                    X11RequestOutcome::Failed
                },
                status: Some(x11_request_error_status(&error)),
            }
        }
    }
}

fn x11_request_error_status(error: &anyhow::Error) -> X11ForwardingStatus {
    let key = if error.downcast_ref::<X11RequestRejected>().is_some() {
        "SSH.x11_remote_request_rejected"
    } else {
        "SSH.x11_request_failed"
    };
    X11ForwardingStatus::Disabled {
        reason: t!(key, error = format!("{error:#}")).to_string(),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct X11ConnectionState {
    pub(crate) status: X11ForwardingStatus,
    pub(crate) remote_unavailable: bool,
    mode: X11ServerMode,
}

impl X11ConnectionState {
    pub(crate) fn from_test(
        requested: bool,
        mode: X11ServerMode,
        probe: &X11Probe,
    ) -> Option<Self> {
        if !requested {
            return None;
        }
        let resolved = probe.status.clone()?;
        let remote_unavailable = probe.outcome == X11RequestOutcome::Rejected;
        if matches!(resolved, X11ForwardingStatus::Disabled { .. })
            || matches!(
                probe.outcome,
                X11RequestOutcome::Accepted | X11RequestOutcome::NotAttempted
            )
        {
            return Some(Self {
                status: resolved,
                remote_unavailable,
                mode,
            });
        }
        let reason = if remote_unavailable {
            t!("SSH.x11_remote_channel_unavailable").to_string()
        } else {
            t!("SSH.x11_request_unavailable").to_string()
        };
        Some(Self {
            status: X11ForwardingStatus::Disabled { reason },
            remote_unavailable,
            mode,
        })
    }

    pub(crate) fn guidance_keys(&self) -> &'static [&'static str] {
        if self.remote_unavailable {
            &REMOTE_GUIDANCE_KEYS
        } else {
            &[]
        }
    }

    pub(crate) fn should_offer_external(&self) -> bool {
        self.mode != X11ServerMode::External
            && matches!(self.status, X11ForwardingStatus::Disabled { .. })
    }

    pub(crate) fn apply_external_resolution(&mut self, status: X11ForwardingStatus) {
        self.mode = X11ServerMode::External;
        self.status = status;
        self.remote_unavailable = false;
    }

    pub(crate) fn label(&self) -> String {
        match &self.status {
            X11ForwardingStatus::External { server } => {
                t!("SSH.x11_status_external", server = server).to_string()
            }
            X11ForwardingStatus::Embedded { version } => {
                t!("SSH.x11_status_embedded", version = version).to_string()
            }
            X11ForwardingStatus::Disabled { reason } => {
                t!("SSH.x11_status_disabled", reason = reason).to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use one_core::settings::{X11ForwardingStatus, X11ServerMode};
    use ssh::{
        ChannelEvent, ForwardRequest, MagicCookie, PtyConfig, ServerEndpoint, SshChannel,
        SshClient, SshConnectConfig, X11Proxy, X11RequestRejected,
    };

    use super::{X11ConnectionState, X11Probe, X11RequestOutcome, probe_connection_x11};

    fn probe(outcome: X11RequestOutcome, status: X11ForwardingStatus) -> X11Probe {
        X11Probe {
            outcome,
            status: Some(status),
        }
    }

    #[derive(Default)]
    struct MockChannelState {
        requests: Vec<String>,
        closed: bool,
        rejection: bool,
        transport_error: Option<String>,
    }

    struct MockChannel {
        state: Arc<Mutex<MockChannelState>>,
    }

    #[async_trait]
    impl SshChannel for MockChannel {
        async fn request_pty(&mut self, _config: &PtyConfig) -> Result<()> {
            Ok(())
        }

        async fn exec(&mut self, _command: &str) -> Result<()> {
            Ok(())
        }

        async fn request_shell(&mut self) -> Result<()> {
            Ok(())
        }

        async fn request_x11_forwarding(&mut self, request: &ForwardRequest) -> Result<()> {
            let mut state = self.state.lock().expect("mock channel state");
            state.requests.push(request.cookie_hex().to_string());
            assert!(!request.single_use);
            if state.rejection {
                return Err(anyhow::Error::new(X11RequestRejected));
            }
            match state.transport_error.clone() {
                Some(error) => Err(anyhow!(error)),
                None => Ok(()),
            }
        }

        async fn set_env(&mut self, _name: &str, _value: &str) -> Result<()> {
            Ok(())
        }

        async fn send_data(&mut self, _data: &[u8]) -> Result<()> {
            Ok(())
        }

        async fn resize_pty(&mut self, _width: u32, _height: u32) -> Result<()> {
            Ok(())
        }

        async fn recv(&mut self) -> Option<ChannelEvent> {
            None
        }

        async fn eof(&mut self) -> Result<()> {
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            self.state.lock().expect("mock channel state").closed = true;
            Ok(())
        }
    }

    struct MockClient {
        proxy: Option<X11Proxy>,
        ready_status: Option<X11ForwardingStatus>,
        status: Mutex<Option<X11ForwardingStatus>>,
        channel_state: Arc<Mutex<MockChannelState>>,
        open_error: Option<String>,
        open_count: usize,
    }

    impl MockClient {
        fn new(rejection: bool, transport_error: Option<&str>) -> Self {
            let ready_status = X11ForwardingStatus::External {
                server: ":0".to_string(),
            };
            Self {
                proxy: Some(X11Proxy::for_endpoint(
                    ServerEndpoint::Inet {
                        host: "127.0.0.1".to_string(),
                        port: 6137,
                    },
                    MagicCookie::generate(),
                )),
                ready_status: Some(ready_status.clone()),
                status: Mutex::new(Some(ready_status)),
                channel_state: Arc::new(Mutex::new(MockChannelState {
                    rejection,
                    transport_error: transport_error.map(str::to_string),
                    ..MockChannelState::default()
                })),
                open_error: None,
                open_count: 0,
            }
        }
    }

    #[async_trait]
    impl SshClient for MockClient {
        type Channel = MockChannel;

        async fn connect(_config: SshConnectConfig) -> Result<Self> {
            unreachable!("测试直接构造 mock client")
        }

        async fn open_channel(&mut self) -> Result<Self::Channel> {
            self.open_count += 1;
            if let Some(error) = self.open_error.take() {
                return Err(anyhow!(error));
            }
            Ok(MockChannel {
                state: Arc::clone(&self.channel_state),
            })
        }

        async fn disconnect(&mut self) -> Result<()> {
            Ok(())
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn x11_forwarding(&self) -> Option<&X11Proxy> {
            self.proxy.as_ref()
        }

        fn x11_status(&self) -> Option<X11ForwardingStatus> {
            self.status.lock().expect("mock status").clone()
        }
    }

    #[tokio::test]
    async fn connection_test_sends_real_x11_request_on_a_temporary_channel() {
        let mut client = MockClient::new(false, None);
        let probe = probe_connection_x11(&mut client).await;

        assert_eq!(X11RequestOutcome::Accepted, probe.outcome);
        assert!(probe.accepted());
        assert_eq!(1, client.open_count);
        assert_eq!(client.ready_status, probe.status);
        let state = client.channel_state.lock().expect("mock channel state");
        assert_eq!(1, state.requests.len());
        assert!(state.closed);
    }

    #[tokio::test]
    async fn remote_x11_rejection_is_visible_but_not_an_ssh_error() {
        let mut client = MockClient::new(true, None);
        let probe = probe_connection_x11(&mut client).await;
        let state = X11ConnectionState::from_test(true, X11ServerMode::External, &probe)
            .expect("X11 拒绝状态");

        assert_eq!(X11RequestOutcome::Rejected, probe.outcome);
        assert!(!probe.accepted());
        assert!(state.remote_unavailable);
        assert!(matches!(
            state.status,
            X11ForwardingStatus::Disabled { ref reason } if reason.contains("x11-req")
        ));
        assert!(client.is_connected());
        assert!(
            client
                .channel_state
                .lock()
                .expect("mock channel state")
                .closed
        );
    }

    #[tokio::test]
    async fn x11_transport_failure_does_not_claim_remote_rejection() {
        let mut client = MockClient::new(false, Some("local send failed"));
        let probe = probe_connection_x11(&mut client).await;
        let state = X11ConnectionState::from_test(true, X11ServerMode::External, &probe)
            .expect("X11 失败状态");

        assert_eq!(X11RequestOutcome::Failed, probe.outcome);
        assert!(!state.remote_unavailable);
        assert!(state.guidance_keys().is_empty());
        assert!(matches!(
            state.status,
            X11ForwardingStatus::Disabled { ref reason }
                if reason.contains("local send failed") && !reason.contains("sshd_config")
        ));
        assert!(client.is_connected());
    }

    #[test]
    fn disabled_x11_checkbox_has_no_connection_status() {
        assert!(
            X11ConnectionState::from_test(
                false,
                X11ServerMode::Auto,
                &probe(
                    X11RequestOutcome::NotAttempted,
                    X11ForwardingStatus::Disabled {
                        reason: "not requested".to_string(),
                    },
                ),
            )
            .is_none()
        );
    }

    #[test]
    fn active_status_preserves_external_or_embedded_details() {
        let external = X11ConnectionState::from_test(
            true,
            X11ServerMode::External,
            &probe(
                X11RequestOutcome::Accepted,
                X11ForwardingStatus::External {
                    server: ":0".to_string(),
                },
            ),
        )
        .expect("外部状态");
        assert!(matches!(
            external.status,
            X11ForwardingStatus::External { ref server } if server == ":0"
        ));

        let embedded = X11ConnectionState::from_test(
            true,
            X11ServerMode::Embedded,
            &probe(
                X11RequestOutcome::Accepted,
                X11ForwardingStatus::Embedded {
                    version: "1.2.3".to_string(),
                },
            ),
        )
        .expect("内置状态");
        assert!(matches!(
            embedded.status,
            X11ForwardingStatus::Embedded { ref version } if version == "1.2.3"
        ));
    }

    #[test]
    fn refused_remote_channel_becomes_visible_disabled_state() {
        let state = X11ConnectionState::from_test(
            true,
            X11ServerMode::External,
            &probe(
                X11RequestOutcome::Rejected,
                X11ForwardingStatus::External {
                    server: ":0".to_string(),
                },
            ),
        )
        .expect("远端拒绝也应显示状态");

        assert!(state.remote_unavailable);
        assert!(matches!(state.status, X11ForwardingStatus::Disabled { .. }));
    }

    #[test]
    fn local_embedded_failure_preserves_reason_and_offers_external() {
        let state = X11ConnectionState::from_test(
            true,
            X11ServerMode::Auto,
            &probe(
                X11RequestOutcome::NotAttempted,
                X11ForwardingStatus::Disabled {
                    reason: "Vulkan unavailable".to_string(),
                },
            ),
        )
        .expect("失败状态");

        assert!(!state.remote_unavailable);
        assert!(state.guidance_keys().is_empty());
        assert!(matches!(
            state.status,
            X11ForwardingStatus::Disabled { ref reason } if reason == "Vulkan unavailable"
        ));
    }

    #[test]
    fn remote_guidance_is_limited_to_actual_channel_refusal() {
        let remote = X11ConnectionState::from_test(
            true,
            X11ServerMode::Embedded,
            &probe(
                X11RequestOutcome::Rejected,
                X11ForwardingStatus::Embedded {
                    version: "1.2.3".to_string(),
                },
            ),
        )
        .expect("远端拒绝状态");
        assert_eq!(
            &[
                "SSH.x11_remote_forwarding_disabled_checklist",
                "SSH.x11_untrusted_ssh_x_security",
            ],
            remote.guidance_keys()
        );
        assert!(
            !remote
                .guidance_keys()
                .contains(&"SSH.x11_remote_display_restarted")
        );
    }

    #[test]
    fn external_selection_hides_the_offer() {
        let mut state = X11ConnectionState {
            status: X11ForwardingStatus::Disabled {
                reason: "Vulkan unavailable".to_string(),
            },
            remote_unavailable: false,
            mode: X11ServerMode::Auto,
        };
        assert!(state.should_offer_external());
        state.apply_external_resolution(X11ForwardingStatus::External {
            server: ":0".to_string(),
        });
        assert!(!state.should_offer_external());
        assert!(matches!(
            state.status,
            X11ForwardingStatus::External { ref server } if server == ":0"
        ));
    }

    #[test]
    fn external_probe_failure_replaces_stale_disabled_reason() {
        let mut state = X11ConnectionState {
            status: X11ForwardingStatus::Disabled {
                reason: "old embedded failure".to_string(),
            },
            remote_unavailable: true,
            mode: X11ServerMode::Auto,
        };
        state.apply_external_resolution(X11ForwardingStatus::Disabled {
            reason: "DISPLAY is not reachable".to_string(),
        });
        assert!(!state.remote_unavailable);
        assert!(state.guidance_keys().is_empty());
        assert!(matches!(
            state.status,
            X11ForwardingStatus::Disabled { ref reason } if reason == "DISPLAY is not reachable"
        ));
    }

    #[test]
    fn ssh_failure_guidance_exists_in_all_three_languages() {
        for language in ["en", "zh-CN", "zh-HK"] {
            rust_i18n::set_locale(language);
            for key in [
                "SSH.x11_status_external",
                "SSH.x11_status_embedded",
                "SSH.x11_status_disabled",
                "SSH.x11_remote_forwarding_disabled_checklist",
                "SSH.x11_remote_display_restarted",
                "SSH.x11_untrusted_ssh_x_security",
                "SSH.x11_macos_xquartz_guidance",
                "SSH.x11_switch_external",
                "SSH.x11_switching_external",
                "SSH.x11_channel_open_failed",
                "SSH.x11_remote_request_rejected",
                "SSH.x11_request_failed",
                "SSH.x11_request_unavailable",
            ] {
                assert_ne!(key, rust_i18n::t!(key), "{language} 缺少文案: {key}");
            }
        }
        rust_i18n::set_locale("en");
    }

    #[test]
    fn connection_result_renders_persistent_structured_status() {
        let window = include_str!("ssh_form_window.rs");
        assert!(window.contains("x11_test_status: Option<X11ConnectionState>"));
        assert!(window.contains("render_x11_status"));
        assert!(window.contains("x11_test_status = Some(x11_state)"));
        assert!(window.contains("probe_connection_x11(&mut client).await"));
        assert!(!window.contains("client.x11_forwarding().is_some()"));
        assert!(!window.contains("current_status()"));
        assert!(window.contains("switch_x11_to_external"));
        assert!(window.contains("x11_component::resolve_server(X11ServerMode::External).await"));
        assert!(window.contains("AppSettings::update_and_save"));
    }
}
