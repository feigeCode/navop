rust_i18n::i18n!("locales", fallback = "en");

mod connection_key;
mod dynamic_socks;
mod host_key;
mod session_manager;
mod session_registry;
mod socks5;
mod ssh;

pub use connection_key::{
    ConnectionCredentialRevisions, ConnectionKey, ConnectionKeyError, CredentialRevision,
    CredentialScope,
};
pub use dynamic_socks::{DynamicSocksConfig, DynamicSocksTunnel, start_dynamic_socks_forward};
pub use host_key::{
    HostKeyAcceptance, HostKeyDetails, HostKeyIdentity, HostKeyPolicy, HostKeyProxyType,
    HostKeyRejection, HostKeyRoute, HostKeyVerifier,
};
pub use session_manager::SshSessionManager;
pub use session_registry::{
    SshSessionLease, SshSessionRegistry, SshSessionService, SshSessionServiceSnapshot,
    SshSessionServiceState, SshSessionShutdownReport,
};
pub use ssh::{
    AuthFailureMessages, ChannelEvent, JumpServerConnectConfig, KeyboardInteractivePrompt,
    KeyboardInteractiveRequest, KeyboardInteractiveResponder, KeyboardInteractiveTarget,
    LocalPortForwardActivity, LocalPortForwardConfig, LocalPortForwardTunnel, ProxyConnectConfig,
    ProxyType, PtyConfig, RusshChannel, RusshClient, ShellIntegrationSetup, SshAuth, SshChannel,
    SshClient, SshConnectConfig, authenticate_session, authenticate_session_with_fallbacks,
    authenticate_with_strategy, connect_via_proxy, defaults, expand_auto_publickey_auth,
    start_local_port_forward, start_local_port_forward_with_config,
};
pub use x11_forwarding::{ForwardRequest, X11Proxy};
