//! In-memory identity for safely sharing SSH transports.
//!
//! A [`ConnectionKey`] deliberately does not hash or retain passwords, private
//! key contents, passphrases, proxy passwords, or MFA responses.  Callers
//! provide an opaque [`CredentialRevision`] obtained from a non-secret
//! credential slot/version.  That revision must change whenever the
//! corresponding secret or authentication context changes.
//!
//! The key is intended for an application-lifetime registry.  Its fields are
//! private, its `Debug` implementation is redacted, and it intentionally does
//! not implement a persistence/serialization contract.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use crate::host_key::normalize_host;
use crate::{
    HostKeyIdentity, HostKeyPolicy, JumpServerConnectConfig, ProxyConnectConfig, ProxyType,
    SshAuth, SshConnectConfig,
};

/// Non-secret identity of one credential slot at a specific revision.
///
/// `slot` identifies the credential record or authentication context;
/// `revision` must advance when its password, key, passphrase, agent context,
/// proxy credential, or MFA responder context changes.  Neither value may be
/// derived by applying an ordinary unsalted hash to secret material.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct CredentialRevision {
    slot: u64,
    revision: u64,
}

impl CredentialRevision {
    #[must_use]
    pub const fn new(slot: u64, revision: u64) -> Self {
        Self { slot, revision }
    }
}

impl fmt::Debug for CredentialRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialRevision(<opaque>)")
    }
}

/// Credential identities associated with all authentication boundaries in an
/// [`SshConnectConfig`].
///
/// Target credentials are always required.  Jump, authenticated proxy, and
/// keyboard-interactive identities must be present exactly when the matching
/// configuration is present.  This fail-closed shape prevents a registry from
/// silently ignoring an authentication boundary it cannot hash.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ConnectionCredentialRevisions {
    target: CredentialRevision,
    jump: Option<CredentialRevision>,
    proxy: Option<CredentialRevision>,
    keyboard_interactive: Option<CredentialRevision>,
}

impl ConnectionCredentialRevisions {
    #[must_use]
    pub const fn new(target: CredentialRevision) -> Self {
        Self {
            target,
            jump: None,
            proxy: None,
            keyboard_interactive: None,
        }
    }

    #[must_use]
    pub const fn with_jump(mut self, jump: CredentialRevision) -> Self {
        self.jump = Some(jump);
        self
    }

    #[must_use]
    pub const fn with_proxy(mut self, proxy: CredentialRevision) -> Self {
        self.proxy = Some(proxy);
        self
    }

    #[must_use]
    pub const fn with_keyboard_interactive(
        mut self,
        keyboard_interactive: CredentialRevision,
    ) -> Self {
        self.keyboard_interactive = Some(keyboard_interactive);
        self
    }
}

impl fmt::Debug for ConnectionCredentialRevisions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionCredentialRevisions")
            .field("target", &self.target)
            .field("has_jump", &self.jump.is_some())
            .field("has_proxy", &self.proxy.is_some())
            .field(
                "has_keyboard_interactive",
                &self.keyboard_interactive.is_some(),
            )
            .finish()
    }
}

/// Authentication boundary whose opaque identity does not match the config.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialScope {
    Jump,
    Proxy,
    KeyboardInteractive,
}

impl fmt::Display for CredentialScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Jump => "jump-server",
            Self::Proxy => "proxy",
            Self::KeyboardInteractive => "keyboard-interactive",
        })
    }
}

/// Fail-closed error returned when credential identities and config disagree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionKeyError {
    MissingCredentialRevision(CredentialScope),
    UnexpectedCredentialRevision(CredentialScope),
}

impl fmt::Display for ConnectionKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCredentialRevision(scope) => {
                write!(formatter, "missing opaque credential revision for {scope}")
            }
            Self::UnexpectedCredentialRevision(scope) => {
                write!(
                    formatter,
                    "unexpected opaque credential revision for {scope}"
                )
            }
        }
    }
}

impl std::error::Error for ConnectionKeyError {}

/// Canonical in-memory key for deciding whether two consumers may share one
/// SSH transport.
///
/// Equality covers endpoint/route normalization, usernames, authentication
/// type and opaque revisions, host-key trust namespace, proxy/jump security
/// context, transport timeouts/keepalive, keyboard-interactive context, and
/// X11 forwarding.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct ConnectionKey {
    target: HostKeyIdentity,
    username: String,
    authentication: AuthenticationKey,
    jump: Option<JumpKey>,
    proxy: Option<ProxyKey>,
    host_key: HostKeyNamespace,
    timeout: Option<Duration>,
    keepalive_interval: Option<Duration>,
    keepalive_max: Option<usize>,
    keyboard_interactive: Option<CredentialRevision>,
    x11_forwarding: bool,
}

impl ConnectionKey {
    /// Build a fail-closed key without inspecting or retaining secret material.
    ///
    /// The caller must source `credentials` from non-secret credential record
    /// identity/version metadata and rotate the relevant revision whenever the
    /// actual authentication material changes.
    pub fn from_config(
        config: &SshConnectConfig,
        credentials: ConnectionCredentialRevisions,
    ) -> Result<Self, ConnectionKeyError> {
        let jump_revision = matching_revision(
            config.jump_server.is_some(),
            credentials.jump,
            CredentialScope::Jump,
        )?;
        let proxy_requires_revision = config.proxy.as_ref().is_some_and(proxy_has_credentials);
        let proxy_revision = matching_revision(
            proxy_requires_revision,
            credentials.proxy,
            CredentialScope::Proxy,
        )?;
        let keyboard_interactive = matching_revision(
            config.keyboard_interactive_responder.is_some(),
            credentials.keyboard_interactive,
            CredentialScope::KeyboardInteractive,
        )?;

        Ok(Self {
            target: config.target_host_key_identity(),
            username: config.username.clone(),
            authentication: AuthenticationKey::new(&config.auth, credentials.target),
            jump: config
                .jump_server
                .as_ref()
                .zip(jump_revision)
                .map(|(jump, revision)| {
                    JumpKey::new(
                        jump,
                        config
                            .jump_host_key_identity()
                            .expect("jump identity exists when jump config exists"),
                        revision,
                    )
                }),
            proxy: config
                .proxy
                .as_ref()
                .map(|proxy| ProxyKey::new(proxy, proxy_revision)),
            host_key: HostKeyNamespace::new(config),
            timeout: config.timeout,
            keepalive_interval: config.keepalive_interval,
            keepalive_max: config.keepalive_max,
            keyboard_interactive,
            x11_forwarding: config.x11_forwarding,
        })
    }

    #[must_use]
    pub fn target(&self) -> &HostKeyIdentity {
        &self.target
    }

    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// A credential-free label suitable for lifecycle diagnostics.
    #[must_use]
    pub fn label(&self) -> String {
        format!(
            "{}@{}:{} [{}]",
            self.username,
            self.target.host(),
            self.target.port(),
            self.authentication.kind()
        )
    }
}

impl fmt::Debug for ConnectionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionKey")
            .field("target", &self.target)
            .field("username", &self.username)
            .field("authentication", &self.authentication.kind())
            .field("has_jump", &self.jump.is_some())
            .field("has_proxy", &self.proxy.is_some())
            .field("host_key_policy", &self.host_key.policy)
            .field(
                "has_app_trust_store",
                &self.host_key.trust_store_path.is_some(),
            )
            .field(
                "has_openssh_known_hosts",
                &self.host_key.openssh_known_hosts_path.is_some(),
            )
            .field("timeout", &self.timeout)
            .field("keepalive_interval", &self.keepalive_interval)
            .field("keepalive_max", &self.keepalive_max)
            .field(
                "has_keyboard_interactive",
                &self.keyboard_interactive.is_some(),
            )
            .field("x11_forwarding", &self.x11_forwarding)
            .finish()
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct JumpKey {
    identity: HostKeyIdentity,
    username: String,
    authentication: AuthenticationKey,
}

impl JumpKey {
    fn new(
        config: &JumpServerConnectConfig,
        identity: HostKeyIdentity,
        credential: CredentialRevision,
    ) -> Self {
        Self {
            identity,
            username: config.username.clone(),
            authentication: AuthenticationKey::new(&config.auth, credential),
        }
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct ProxyKey {
    proxy_type: ProxyKind,
    host: String,
    port: u16,
    username: Option<String>,
    credential: Option<CredentialRevision>,
}

impl ProxyKey {
    fn new(config: &ProxyConnectConfig, credential: Option<CredentialRevision>) -> Self {
        Self {
            proxy_type: config.proxy_type.into(),
            host: normalize_host(&config.host),
            port: config.port,
            username: config.username.clone(),
            credential,
        }
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum ProxyKind {
    Socks5,
    Http,
}

impl From<ProxyType> for ProxyKind {
    fn from(value: ProxyType) -> Self {
        match value {
            ProxyType::Socks5 => Self::Socks5,
            ProxyType::Http => Self::Http,
        }
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
enum AuthenticationKey {
    Password(CredentialRevision),
    PrivateKey {
        key_path: String,
        certificate_path: Option<String>,
        credential: CredentialRevision,
    },
    PrivateKeyContent {
        certificate_path: Option<String>,
        credential: CredentialRevision,
    },
    Agent(CredentialRevision),
    AutoPublicKey(CredentialRevision),
}

impl AuthenticationKey {
    fn new(auth: &SshAuth, credential: CredentialRevision) -> Self {
        match auth {
            SshAuth::Password(_) => Self::Password(credential),
            SshAuth::PrivateKey {
                key_path,
                certificate_path,
                ..
            } => Self::PrivateKey {
                key_path: key_path.clone(),
                certificate_path: certificate_path.clone(),
                credential,
            },
            SshAuth::PrivateKeyContent {
                certificate_path, ..
            } => Self::PrivateKeyContent {
                certificate_path: certificate_path.clone(),
                credential,
            },
            SshAuth::Agent => Self::Agent(credential),
            SshAuth::AutoPublicKey => Self::AutoPublicKey(credential),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Password(_) => "password",
            Self::PrivateKey { .. } => "private-key",
            Self::PrivateKeyContent { .. } => "private-key-content",
            Self::Agent(_) => "agent",
            Self::AutoPublicKey(_) => "auto-public-key",
        }
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct HostKeyNamespace {
    policy: HostKeyPolicyKey,
    trust_store_path: Option<PathBuf>,
    openssh_known_hosts_path: Option<PathBuf>,
}

impl HostKeyNamespace {
    fn new(config: &SshConnectConfig) -> Self {
        Self {
            policy: config.host_key_verifier.policy().into(),
            trust_store_path: config
                .host_key_verifier
                .trust_store_path()
                .map(PathBuf::from),
            openssh_known_hosts_path: config
                .host_key_verifier
                .openssh_known_hosts_path()
                .map(PathBuf::from),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum HostKeyPolicyKey {
    Strict,
    AcceptNew,
    Insecure,
}

impl From<HostKeyPolicy> for HostKeyPolicyKey {
    fn from(value: HostKeyPolicy) -> Self {
        match value {
            HostKeyPolicy::Strict => Self::Strict,
            HostKeyPolicy::AcceptNew => Self::AcceptNew,
            HostKeyPolicy::Insecure => Self::Insecure,
        }
    }
}

fn matching_revision(
    configured: bool,
    revision: Option<CredentialRevision>,
    scope: CredentialScope,
) -> Result<Option<CredentialRevision>, ConnectionKeyError> {
    match (configured, revision) {
        (true, Some(revision)) => Ok(Some(revision)),
        (false, None) => Ok(None),
        (true, None) => Err(ConnectionKeyError::MissingCredentialRevision(scope)),
        (false, Some(_)) => Err(ConnectionKeyError::UnexpectedCredentialRevision(scope)),
    }
}

fn proxy_has_credentials(proxy: &ProxyConnectConfig) -> bool {
    proxy.username.is_some() || proxy.password.is_some()
}

#[cfg(test)]
mod tests {
    use super::{
        ConnectionCredentialRevisions, ConnectionKey, ConnectionKeyError, CredentialRevision,
        CredentialScope,
    };
    use crate::{
        HostKeyPolicy, HostKeyVerifier, JumpServerConnectConfig, KeyboardInteractiveRequest,
        KeyboardInteractiveResponder, ProxyConnectConfig, ProxyType, SshAuth, SshConnectConfig,
    };
    use anyhow::Result;
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::time::Duration;

    const TARGET_REVISION: CredentialRevision = CredentialRevision::new(10, 1);
    const JUMP_REVISION: CredentialRevision = CredentialRevision::new(20, 1);
    const PROXY_REVISION: CredentialRevision = CredentialRevision::new(30, 1);
    const INTERACTIVE_REVISION: CredentialRevision = CredentialRevision::new(40, 1);

    fn base_config() -> SshConnectConfig {
        SshConnectConfig {
            host: "example.com".to_owned(),
            port: 22,
            username: "alice".to_owned(),
            auth: SshAuth::Password("target-password".to_owned()),
            timeout: Some(Duration::from_secs(10)),
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: Some(6),
            jump_server: None,
            proxy: None,
            keyboard_interactive_responder: None,
            host_key_verifier: HostKeyVerifier::new(
                HostKeyPolicy::Strict,
                Some("/trust/app.json".into()),
                Some("/trust/known_hosts".into()),
            ),
            x11_forwarding: false,
        }
    }

    fn target_credentials() -> ConnectionCredentialRevisions {
        ConnectionCredentialRevisions::new(TARGET_REVISION)
    }

    fn key(config: &SshConnectConfig, credentials: ConnectionCredentialRevisions) -> ConnectionKey {
        ConnectionKey::from_config(config, credentials).expect("valid connection key")
    }

    #[test]
    fn equivalent_config_and_revisions_produce_the_same_key() {
        let left = base_config();
        let mut right = base_config();
        right.host = "  EXAMPLE.COM. ".to_owned();

        assert_eq!(
            key(&left, target_credentials()),
            key(&right, target_credentials())
        );
    }

    #[test]
    fn target_identity_security_boundaries_do_not_share() {
        let baseline = base_config();
        let baseline_key = key(&baseline, target_credentials());

        let mut changed_username = base_config();
        changed_username.username = "bob".to_owned();
        assert_ne!(baseline_key, key(&changed_username, target_credentials()));

        let mut whitespace_username = base_config();
        whitespace_username.username = " alice ".to_owned();
        assert_ne!(
            baseline_key,
            key(&whitespace_username, target_credentials())
        );

        let mut changed_auth_kind = base_config();
        changed_auth_kind.auth = SshAuth::Agent;
        assert_ne!(baseline_key, key(&changed_auth_kind, target_credentials()));

        let changed_revision = ConnectionCredentialRevisions::new(CredentialRevision::new(10, 2));
        assert_ne!(baseline_key, key(&baseline, changed_revision));

        let mut changed_x11 = base_config();
        changed_x11.x11_forwarding = true;
        assert_ne!(baseline_key, key(&changed_x11, target_credentials()));
    }

    #[test]
    fn jump_and_proxy_route_and_auth_boundaries_do_not_share() {
        let mut baseline = base_config();
        baseline.jump_server = Some(JumpServerConnectConfig {
            host: "jump.example.com".to_owned(),
            port: 22,
            username: "jumper".to_owned(),
            auth: SshAuth::Password("jump-password".to_owned()),
        });
        baseline.proxy = Some(ProxyConnectConfig {
            proxy_type: ProxyType::Socks5,
            host: "proxy.example.com".to_owned(),
            port: 1080,
            username: Some("proxy-user".to_owned()),
            password: Some("proxy-password".to_owned()),
        });
        let credentials = target_credentials()
            .with_jump(JUMP_REVISION)
            .with_proxy(PROXY_REVISION);
        let baseline_key = key(&baseline, credentials);

        let mut normalized = baseline.clone();
        let jump = normalized.jump_server.as_mut().expect("jump");
        jump.host = " JUMP.EXAMPLE.COM. ".to_owned();
        let proxy = normalized.proxy.as_mut().expect("proxy");
        proxy.host = " PROXY.EXAMPLE.COM. ".to_owned();
        assert_eq!(baseline_key, key(&normalized, credentials));

        let mut changed_jump_username = baseline.clone();
        changed_jump_username
            .jump_server
            .as_mut()
            .expect("jump")
            .username = "other-jumper".to_owned();
        assert_ne!(baseline_key, key(&changed_jump_username, credentials));

        let mut changed_proxy_username = baseline.clone();
        changed_proxy_username
            .proxy
            .as_mut()
            .expect("proxy")
            .username = Some("other-proxy-user".to_owned());
        assert_ne!(baseline_key, key(&changed_proxy_username, credentials));

        let mut changed_route = baseline.clone();
        changed_route.proxy.as_mut().expect("proxy").proxy_type = ProxyType::Http;
        assert_ne!(baseline_key, key(&changed_route, credentials));

        let changed_jump_revision = target_credentials()
            .with_jump(CredentialRevision::new(20, 2))
            .with_proxy(PROXY_REVISION);
        assert_ne!(baseline_key, key(&baseline, changed_jump_revision));

        let changed_proxy_revision = target_credentials()
            .with_jump(JUMP_REVISION)
            .with_proxy(CredentialRevision::new(30, 2));
        assert_ne!(baseline_key, key(&baseline, changed_proxy_revision));
    }

    #[test]
    fn trust_namespace_and_transport_settings_do_not_share() {
        let baseline = base_config();
        let baseline_key = key(&baseline, target_credentials());

        let mut changed_policy = base_config();
        changed_policy.host_key_verifier = HostKeyVerifier::new(
            HostKeyPolicy::AcceptNew,
            Some("/trust/app.json".into()),
            Some("/trust/known_hosts".into()),
        );
        assert_ne!(baseline_key, key(&changed_policy, target_credentials()));

        let mut changed_store = base_config();
        changed_store.host_key_verifier = HostKeyVerifier::new(
            HostKeyPolicy::Strict,
            Some("/trust/other.json".into()),
            Some("/trust/known_hosts".into()),
        );
        assert_ne!(baseline_key, key(&changed_store, target_credentials()));

        let mut changed_known_hosts = base_config();
        changed_known_hosts.host_key_verifier = HostKeyVerifier::new(
            HostKeyPolicy::Strict,
            Some("/trust/app.json".into()),
            Some("/trust/other_known_hosts".into()),
        );
        assert_ne!(
            baseline_key,
            key(&changed_known_hosts, target_credentials())
        );

        let mut changed_timeout = base_config();
        changed_timeout.timeout = Some(Duration::from_secs(11));
        assert_ne!(baseline_key, key(&changed_timeout, target_credentials()));
    }

    #[derive(Default)]
    struct TestResponder;

    #[async_trait]
    impl KeyboardInteractiveResponder for TestResponder {
        async fn respond(&self, _request: KeyboardInteractiveRequest) -> Result<Vec<String>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn keyboard_interactive_requires_an_explicit_context_revision() {
        let mut config = base_config();
        config.keyboard_interactive_responder = Some(Arc::new(TestResponder));

        assert_eq!(
            ConnectionKey::from_config(&config, target_credentials()),
            Err(ConnectionKeyError::MissingCredentialRevision(
                CredentialScope::KeyboardInteractive
            ))
        );

        let baseline = key(
            &config,
            target_credentials().with_keyboard_interactive(INTERACTIVE_REVISION),
        );
        let changed = key(
            &config,
            target_credentials().with_keyboard_interactive(CredentialRevision::new(40, 2)),
        );
        assert_ne!(baseline, changed);
    }

    #[test]
    fn credential_revision_shape_must_match_jump_proxy_and_responder_config() {
        assert_eq!(
            ConnectionKey::from_config(
                &base_config(),
                target_credentials().with_jump(JUMP_REVISION)
            ),
            Err(ConnectionKeyError::UnexpectedCredentialRevision(
                CredentialScope::Jump
            ))
        );

        let mut jump_config = base_config();
        jump_config.jump_server = Some(JumpServerConnectConfig {
            host: "jump.example.com".to_owned(),
            port: 22,
            username: "jumper".to_owned(),
            auth: SshAuth::Agent,
        });
        assert_eq!(
            ConnectionKey::from_config(&jump_config, target_credentials()),
            Err(ConnectionKeyError::MissingCredentialRevision(
                CredentialScope::Jump
            ))
        );

        let mut anonymous_proxy = base_config();
        anonymous_proxy.proxy = Some(ProxyConnectConfig {
            proxy_type: ProxyType::Socks5,
            host: "proxy.example.com".to_owned(),
            port: 1080,
            username: None,
            password: None,
        });
        assert!(ConnectionKey::from_config(&anonymous_proxy, target_credentials()).is_ok());
        assert_eq!(
            ConnectionKey::from_config(
                &anonymous_proxy,
                target_credentials().with_proxy(PROXY_REVISION)
            ),
            Err(ConnectionKeyError::UnexpectedCredentialRevision(
                CredentialScope::Proxy
            ))
        );
    }

    #[test]
    fn debug_and_errors_never_include_secret_material() {
        let mut config = base_config();
        config.auth = SshAuth::PrivateKeyContent {
            private_key: "PRIVATE-KEY-CONTENT-SECRET".to_owned(),
            passphrase: Some("TARGET-PASSPHRASE-SECRET".to_owned()),
            certificate_path: Some("/sensitive/target-certificate.pub".to_owned()),
        };
        config.jump_server = Some(JumpServerConnectConfig {
            host: "jump.example.com".to_owned(),
            port: 22,
            username: "jumper".to_owned(),
            auth: SshAuth::Password("JUMP-PASSWORD-SECRET".to_owned()),
        });
        config.proxy = Some(ProxyConnectConfig {
            proxy_type: ProxyType::Http,
            host: "proxy.example.com".to_owned(),
            port: 8080,
            username: Some("proxy-user".to_owned()),
            password: Some("PROXY-PASSWORD-SECRET".to_owned()),
        });
        config.keyboard_interactive_responder = Some(Arc::new(TestResponder));
        let credentials = target_credentials()
            .with_jump(JUMP_REVISION)
            .with_proxy(PROXY_REVISION)
            .with_keyboard_interactive(INTERACTIVE_REVISION);

        let debug = format!("{:?}", key(&config, credentials));
        for secret in [
            "PRIVATE-KEY-CONTENT-SECRET",
            "TARGET-PASSPHRASE-SECRET",
            "JUMP-PASSWORD-SECRET",
            "PROXY-PASSWORD-SECRET",
            "/sensitive/target-certificate.pub",
        ] {
            assert!(!debug.contains(secret), "debug leaked {secret}: {debug}");
        }

        let error = ConnectionKey::from_config(&config, target_credentials())
            .expect_err("missing revisions must fail");
        let diagnostic = format!("{error:?}: {error}");
        for secret in [
            "PRIVATE-KEY-CONTENT-SECRET",
            "TARGET-PASSPHRASE-SECRET",
            "JUMP-PASSWORD-SECRET",
            "PROXY-PASSWORD-SECRET",
        ] {
            assert!(
                !diagnostic.contains(secret),
                "error leaked {secret}: {diagnostic}"
            );
        }
    }

    #[test]
    fn label_uses_normalized_host_and_is_credential_free() {
        let mut config = base_config();
        config.host = " EXAMPLE.COM. ".to_owned();

        let key = key(&config, target_credentials());
        assert_eq!(key.target().host(), "example.com");
        assert_eq!(key.username(), "alice");
        assert_eq!(key.label(), "alice@example.com:22 [password]");
        assert!(!key.label().contains("target-password"));
    }
}
