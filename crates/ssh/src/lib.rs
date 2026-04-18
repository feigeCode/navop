rust_i18n::i18n!("locales", fallback = "en");

mod session_manager;
mod ssh;

pub use session_manager::SshSessionManager;
pub use ssh::{
    authenticate_session, authenticate_session_with_fallbacks, authenticate_with_strategy,
    defaults, expand_auto_publickey_auth, start_local_port_forward, AuthFailureMessages,
    ChannelEvent, JumpServerConnectConfig, LocalPortForwardTunnel, ProxyConnectConfig, ProxyType,
    PtyConfig, RusshChannel, RusshClient, ShellIntegrationSetup, SshAuth, SshChannel, SshClient,
    SshConnectConfig,
};
