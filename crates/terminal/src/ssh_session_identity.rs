//! Persisted, credential-free identity for Terminal SSH session leases.
//!
//! The application SSH service can only share a transport when callers supply
//! an opaque credential revision for every authentication boundary. Terminal
//! derives those revisions from local repository metadata instead of hashing,
//! retaining, formatting, or logging secret material.
//!
//! Callers must pass the authoritative, repository-refreshed
//! [`StoredConnection`] used to resolve `config`. Unsaved, imported, cloud
//! deserialized, or stale in-memory records fail closed rather than acquiring
//! a shared lease under an ambiguous identity.

use std::fmt;

use one_core::storage::models::{ConnectionType, StoredConnection};
use ssh::{
    ConnectionCredentialRevisions, ConnectionKey, ConnectionKeyError, CredentialRevision,
    SshConnectConfig,
};

const PERSISTED_CREDENTIAL_SCOPE_COUNT: u64 = 4;

/// Whether an already-held Terminal SSH lease remains valid for a new
/// resolved identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SshSessionIdentityTransition {
    /// Ordinary reconnect: open a new channel on the same healthy lease.
    ReuseLease,
    /// Persisted identity, credential revision, or transport config changed.
    ReplaceLease,
}

/// Credential-free identity used to acquire an application-owned SSH session
/// lease for one persisted Terminal connection.
///
/// The inner [`ConnectionKey`] is deliberately non-serializable and omits
/// passwords, private-key contents, passphrases, proxy passwords, and MFA
/// responses. Its opaque slots are derived with checked arithmetic from the
/// positive local connection ID and positive local credential revision.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct PersistedSshSessionIdentity {
    connection_key: ConnectionKey,
}

impl PersistedSshSessionIdentity {
    /// Derive an identity from authoritative local repository metadata and the
    /// SSH config resolved from that same record.
    ///
    /// This fails closed when the record is not a persisted SSH connection,
    /// its local ID/revision is absent or invalid, scoped slot arithmetic
    /// overflows, or the credential scopes do not match `config`.
    pub fn derive(
        connection: &StoredConnection,
        config: &SshConnectConfig,
    ) -> Result<Self, PersistedSshSessionIdentityError> {
        if connection.connection_type != ConnectionType::SshSftp {
            return Err(PersistedSshSessionIdentityError::NotSshConnection);
        }

        let generation = PersistedCredentialGeneration::from_connection(connection)?;
        let mut credentials =
            ConnectionCredentialRevisions::new(generation.for_scope(CredentialSlotScope::Target)?);

        if config.jump_server.is_some() {
            credentials = credentials.with_jump(generation.for_scope(CredentialSlotScope::Jump)?);
        }
        if config
            .proxy
            .as_ref()
            .is_some_and(proxy_requires_credential_revision)
        {
            credentials = credentials.with_proxy(generation.for_scope(CredentialSlotScope::Proxy)?);
        }
        if config.keyboard_interactive_responder.is_some() {
            credentials = credentials.with_keyboard_interactive(
                generation.for_scope(CredentialSlotScope::KeyboardInteractive)?,
            );
        }

        let connection_key = ConnectionKey::from_config(config, credentials)
            .map_err(PersistedSshSessionIdentityError::ConnectionKey)?;
        Ok(Self { connection_key })
    }

    /// Borrow the key supplied to [`ssh::SshSessionService::acquire`].
    #[must_use]
    pub fn connection_key(&self) -> &ConnectionKey {
        &self.connection_key
    }

    /// Classify whether moving from this identity to `next` may retain the
    /// current generation-bound lease.
    #[must_use]
    pub fn transition_to(&self, next: &Self) -> SshSessionIdentityTransition {
        if self == next {
            SshSessionIdentityTransition::ReuseLease
        } else {
            SshSessionIdentityTransition::ReplaceLease
        }
    }
}

impl fmt::Debug for PersistedSshSessionIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PersistedSshSessionIdentity")
            .field("connection", &self.connection_key)
            .finish()
    }
}

impl fmt::Display for PersistedSshSessionIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.connection_key.label())
    }
}

/// Fail-closed errors while deriving a persisted Terminal SSH identity.
///
/// Variants intentionally contain neither the offending stored values nor any
/// SSH config so `Debug`, `Display`, error chains, and logs cannot expose
/// credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistedSshSessionIdentityError {
    NotSshConnection,
    MissingConnectionId,
    InvalidConnectionId,
    MissingCredentialRevision,
    InvalidCredentialRevision,
    CredentialSlotOverflow,
    ConnectionKey(ConnectionKeyError),
}

impl fmt::Display for PersistedSshSessionIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSshConnection => {
                formatter.write_str("SSH session identity requires a stored SSH connection")
            }
            Self::MissingConnectionId => {
                formatter.write_str("SSH session identity requires a persisted connection ID")
            }
            Self::InvalidConnectionId => {
                formatter.write_str("persisted SSH connection ID must be positive")
            }
            Self::MissingCredentialRevision => {
                formatter.write_str("SSH session identity requires a persisted credential revision")
            }
            Self::InvalidCredentialRevision => {
                formatter.write_str("persisted SSH credential revision must be positive")
            }
            Self::CredentialSlotOverflow => {
                formatter.write_str("persisted SSH credential scope is outside the identity range")
            }
            Self::ConnectionKey(error) => {
                write!(
                    formatter,
                    "SSH credential scopes do not match config: {error}"
                )
            }
        }
    }
}

impl std::error::Error for PersistedSshSessionIdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ConnectionKey(error) => Some(error),
            Self::NotSshConnection
            | Self::MissingConnectionId
            | Self::InvalidConnectionId
            | Self::MissingCredentialRevision
            | Self::InvalidCredentialRevision
            | Self::CredentialSlotOverflow => None,
        }
    }
}

#[derive(Clone, Copy)]
#[repr(u64)]
enum CredentialSlotScope {
    Target = 0,
    Jump = 1,
    Proxy = 2,
    KeyboardInteractive = 3,
}

#[derive(Clone, Copy)]
struct PersistedCredentialGeneration {
    slot_base: u64,
    revision: u64,
}

impl PersistedCredentialGeneration {
    fn from_connection(
        connection: &StoredConnection,
    ) -> Result<Self, PersistedSshSessionIdentityError> {
        let connection_id = connection
            .id
            .ok_or(PersistedSshSessionIdentityError::MissingConnectionId)?;
        let connection_id = u64::try_from(connection_id)
            .ok()
            .filter(|id| *id > 0)
            .ok_or(PersistedSshSessionIdentityError::InvalidConnectionId)?;

        let revision = connection
            .credential_revision
            .ok_or(PersistedSshSessionIdentityError::MissingCredentialRevision)?;
        let revision = u64::try_from(revision)
            .ok()
            .filter(|revision| *revision > 0)
            .ok_or(PersistedSshSessionIdentityError::InvalidCredentialRevision)?;

        let slot_base = connection_id
            .checked_mul(PERSISTED_CREDENTIAL_SCOPE_COUNT)
            .ok_or(PersistedSshSessionIdentityError::CredentialSlotOverflow)?;

        Ok(Self {
            slot_base,
            revision,
        })
    }

    fn for_scope(
        self,
        scope: CredentialSlotScope,
    ) -> Result<CredentialRevision, PersistedSshSessionIdentityError> {
        let slot = self
            .slot_base
            .checked_add(scope as u64)
            .ok_or(PersistedSshSessionIdentityError::CredentialSlotOverflow)?;
        Ok(CredentialRevision::new(slot, self.revision))
    }
}

fn proxy_requires_credential_revision(proxy: &ssh::ProxyConnectConfig) -> bool {
    proxy.username.is_some() || proxy.password.is_some()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;

    use anyhow::Result;
    use async_trait::async_trait;
    use one_core::storage::models::{ConnectionType, SshAuthMethod, SshParams, StoredConnection};
    use ssh::{
        HostKeyVerifier, JumpServerConnectConfig, KeyboardInteractiveRequest,
        KeyboardInteractiveResponder, ProxyConnectConfig, ProxyType, SshAuth, SshConnectConfig,
    };

    use super::{
        CredentialSlotScope, PersistedCredentialGeneration, PersistedSshSessionIdentity,
        PersistedSshSessionIdentityError, SshSessionIdentityTransition,
    };

    const TARGET_PASSWORD: &str = "TARGET-PASSWORD-SECRET";
    const TARGET_PRIVATE_KEY: &str = "TARGET-PRIVATE-KEY-CONTENT-SECRET";
    const TARGET_PASSPHRASE: &str = "TARGET-PASSPHRASE-SECRET";
    const JUMP_PASSWORD: &str = "JUMP-PASSWORD-SECRET";
    const PROXY_PASSWORD: &str = "PROXY-PASSWORD-SECRET";
    const MFA_RESPONSE: &str = "MFA-RESPONSE-SECRET";

    #[derive(Default)]
    struct TestResponder;

    #[async_trait]
    impl KeyboardInteractiveResponder for TestResponder {
        async fn respond(&self, _request: KeyboardInteractiveRequest) -> Result<Vec<String>> {
            Ok(vec![MFA_RESPONSE.to_string()])
        }
    }

    fn stored_connection(id: Option<i64>, revision: Option<i64>) -> StoredConnection {
        let mut connection = StoredConnection::new_ssh(
            "production".to_string(),
            SshParams {
                host: "target.example.com".to_string(),
                port: 22,
                username: "alice".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: TARGET_PASSWORD.to_string(),
                },
                connect_timeout: Some(10),
                keepalive_interval: Some(30),
                keepalive_max: Some(6),
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
            },
            None,
        );
        connection.id = id;
        connection.credential_revision = revision;
        connection
    }

    fn terminal_config() -> SshConnectConfig {
        SshConnectConfig {
            host: "target.example.com".to_string(),
            port: 22,
            username: "alice".to_string(),
            auth: SshAuth::Password(TARGET_PASSWORD.to_string()),
            timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            jump_server: None,
            proxy: None,
            keyboard_interactive_responder: Some(Arc::new(TestResponder)),
            host_key_verifier: HostKeyVerifier::default(),
            x11_forwarding: false,
        }
    }

    fn config_with_all_credential_scopes() -> SshConnectConfig {
        let mut config = terminal_config();
        config.jump_server = Some(JumpServerConnectConfig {
            host: "jump.example.com".to_string(),
            port: 2222,
            username: "jumper".to_string(),
            auth: SshAuth::Password(JUMP_PASSWORD.to_string()),
        });
        config.proxy = Some(ProxyConnectConfig {
            proxy_type: ProxyType::Socks5,
            host: "proxy.example.com".to_string(),
            port: 1080,
            username: Some("proxy-user".to_string()),
            password: Some(PROXY_PASSWORD.to_string()),
        });
        config
    }

    #[test]
    fn ordinary_reconnect_reuses_the_same_lease_identity() {
        let connection = stored_connection(Some(42), Some(7));
        let config = terminal_config();

        let current =
            PersistedSshSessionIdentity::derive(&connection, &config).expect("persisted identity");
        let reconnect = PersistedSshSessionIdentity::derive(&connection, &config)
            .expect("ordinary reconnect identity");

        assert_eq!(current, reconnect);
        assert_eq!(
            SshSessionIdentityTransition::ReuseLease,
            current.transition_to(&reconnect)
        );
    }

    #[test]
    fn credential_revision_or_transport_config_change_replaces_the_lease() {
        let connection = stored_connection(Some(42), Some(7));
        let config = terminal_config();
        let current =
            PersistedSshSessionIdentity::derive(&connection, &config).expect("persisted identity");

        let mut revised_connection = connection.clone();
        revised_connection.credential_revision = Some(8);
        let revised = PersistedSshSessionIdentity::derive(&revised_connection, &config)
            .expect("revised identity");
        assert_eq!(
            SshSessionIdentityTransition::ReplaceLease,
            current.transition_to(&revised)
        );

        let mut moved_config = config.clone();
        moved_config.host = "replacement.example.com".to_string();
        let moved = PersistedSshSessionIdentity::derive(&connection, &moved_config)
            .expect("changed transport identity");
        assert_eq!(
            SshSessionIdentityTransition::ReplaceLease,
            current.transition_to(&moved)
        );
    }

    #[test]
    fn secret_text_is_not_used_as_an_identity_substitute_for_revision() {
        let connection = stored_connection(Some(42), Some(7));
        let config = terminal_config();
        let baseline =
            PersistedSshSessionIdentity::derive(&connection, &config).expect("persisted identity");

        let mut changed_secret = config;
        changed_secret.auth = SshAuth::Password("ROTATED-TARGET-PASSWORD-SECRET".to_string());
        let without_revision_rotation =
            PersistedSshSessionIdentity::derive(&connection, &changed_secret)
                .expect("secret-free identity");

        assert_eq!(
            baseline, without_revision_rotation,
            "the repository revision, not secret hashing, rotates the identity"
        );
    }

    #[test]
    fn target_jump_proxy_and_keyboard_interactive_slots_are_disjoint() {
        let connection = stored_connection(Some(42), Some(7));
        let generation =
            PersistedCredentialGeneration::from_connection(&connection).expect("generation");
        let revisions = [
            generation
                .for_scope(CredentialSlotScope::Target)
                .expect("target"),
            generation
                .for_scope(CredentialSlotScope::Jump)
                .expect("jump"),
            generation
                .for_scope(CredentialSlotScope::Proxy)
                .expect("proxy"),
            generation
                .for_scope(CredentialSlotScope::KeyboardInteractive)
                .expect("keyboard interactive"),
        ];

        assert_eq!(4, HashSet::from(revisions).len());
        PersistedSshSessionIdentity::derive(&connection, &config_with_all_credential_scopes())
            .expect("all configured credential scopes must produce a valid key");
    }

    #[test]
    fn missing_non_positive_and_overflowing_metadata_fail_closed() {
        let config = terminal_config();
        let cases = [
            (
                stored_connection(None, Some(1)),
                PersistedSshSessionIdentityError::MissingConnectionId,
            ),
            (
                stored_connection(Some(0), Some(1)),
                PersistedSshSessionIdentityError::InvalidConnectionId,
            ),
            (
                stored_connection(Some(-1), Some(1)),
                PersistedSshSessionIdentityError::InvalidConnectionId,
            ),
            (
                stored_connection(Some(1), None),
                PersistedSshSessionIdentityError::MissingCredentialRevision,
            ),
            (
                stored_connection(Some(1), Some(0)),
                PersistedSshSessionIdentityError::InvalidCredentialRevision,
            ),
            (
                stored_connection(Some(1), Some(-1)),
                PersistedSshSessionIdentityError::InvalidCredentialRevision,
            ),
        ];

        for (connection, expected) in cases {
            assert_eq!(
                Err(expected),
                PersistedSshSessionIdentity::derive(&connection, &config)
            );
        }

        let largest_safe_id = i64::try_from(u64::MAX / super::PERSISTED_CREDENTIAL_SCOPE_COUNT)
            .expect("largest safe scoped ID fits i64");
        PersistedSshSessionIdentity::derive(
            &stored_connection(Some(largest_safe_id), Some(i64::MAX)),
            &config_with_all_credential_scopes(),
        )
        .expect("largest complete scoped range remains representable");

        let overflow = stored_connection(Some(largest_safe_id + 1), Some(1));
        assert_eq!(
            Err(PersistedSshSessionIdentityError::CredentialSlotOverflow),
            PersistedSshSessionIdentity::derive(&overflow, &config)
        );
    }

    #[test]
    fn cloud_or_export_roundtrip_without_local_revision_fails_closed() {
        let connection = stored_connection(Some(42), Some(7));
        let mut serialized = Vec::new();
        plist::to_writer_xml(&mut serialized, &connection).expect("serialize stored connection");
        assert!(
            !String::from_utf8_lossy(&serialized).contains("credential_revision"),
            "local revision must not cross serialized connection boundaries"
        );
        let restored: StoredConnection =
            plist::from_bytes(&serialized).expect("deserialize stored connection");

        assert_eq!(Some(42), restored.id);
        assert_eq!(None, restored.credential_revision);
        assert_eq!(
            Err(PersistedSshSessionIdentityError::MissingCredentialRevision),
            PersistedSshSessionIdentity::derive(&restored, &terminal_config())
        );
    }

    #[test]
    fn anonymous_proxy_does_not_require_an_opaque_proxy_scope() {
        let connection = stored_connection(Some(42), Some(7));
        let mut config = terminal_config();
        config.proxy = Some(ProxyConnectConfig {
            proxy_type: ProxyType::Http,
            host: "proxy.example.com".to_string(),
            port: 8080,
            username: None,
            password: None,
        });

        PersistedSshSessionIdentity::derive(&connection, &config)
            .expect("anonymous proxy shape should match ConnectionKey");
    }

    #[test]
    fn non_ssh_records_fail_closed() {
        let mut connection = stored_connection(Some(42), Some(7));
        connection.connection_type = ConnectionType::Database;

        assert_eq!(
            Err(PersistedSshSessionIdentityError::NotSshConnection),
            PersistedSshSessionIdentity::derive(&connection, &terminal_config())
        );
    }

    #[test]
    fn identity_debug_display_and_errors_do_not_expose_secrets() {
        let mut connection = stored_connection(Some(42), Some(7));
        connection.params = format!(
            r#"{{"password":"{TARGET_PASSWORD}","private_key":"{TARGET_PRIVATE_KEY}","passphrase":"{TARGET_PASSPHRASE}"}}"#
        );
        let mut config = config_with_all_credential_scopes();
        config.auth = SshAuth::PrivateKeyContent {
            private_key: TARGET_PRIVATE_KEY.to_string(),
            passphrase: Some(TARGET_PASSPHRASE.to_string()),
            certificate_path: Some("/secret/target-certificate.pub".to_string()),
        };

        let identity = PersistedSshSessionIdentity::derive(&connection, &config)
            .expect("secret-free persisted identity");
        let diagnostics = [
            format!("{identity:?}"),
            identity.to_string(),
            format!("{:?}", identity.connection_key()),
        ];

        let mut invalid = connection;
        invalid.id = None;
        let error = PersistedSshSessionIdentity::derive(&invalid, &config)
            .expect_err("missing ID must fail");
        let error_diagnostic = format!("{error:?}: {error}");

        for secret in [
            TARGET_PASSWORD,
            TARGET_PRIVATE_KEY,
            TARGET_PASSPHRASE,
            JUMP_PASSWORD,
            PROXY_PASSWORD,
            MFA_RESPONSE,
            "/secret/target-certificate.pub",
        ] {
            for diagnostic in &diagnostics {
                assert!(
                    !diagnostic.contains(secret),
                    "identity diagnostic leaked {secret}: {diagnostic}"
                );
            }
            assert!(
                !error_diagnostic.contains(secret),
                "identity error leaked {secret}: {error_diagnostic}"
            );
        }
    }
}
