//! SSH server host-key verification and trust storage.
//!
//! The russh client callback only returns a boolean, which makes it tempting to
//! accept every key.  This module keeps the trust decision separate from the
//! transport so both Terminal SSH and SFTP use the same contract:
//!
//! * `Strict` accepts only a key that is already in the trust store (or the
//!   user's OpenSSH `known_hosts` file).
//! * `AcceptNew` is an explicit TOFU mode.  It writes an unknown key, but still
//!   rejects a changed key.
//! * `Insecure` is available only as an explicit opt-in for tests and temporary
//!   diagnostics; it is never the default.
//!
//! Trust entries are bound to the endpoint and route (direct/proxy/jump), so a
//! key learned through one connection path cannot silently authorize another
//! path.  Writes use a same-directory temporary file followed by an atomic
//! persist and are serialized in-process.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use hmac::{Hmac, Mac};
use russh::keys::{HashAlg, PublicKey, ssh_key};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use tempfile::NamedTempFile;

type HmacSha1 = Hmac<Sha1>;

const TRUST_STORE_VERSION: u32 = 1;
const TRUST_STORE_FILE_NAME: &str = "ssh-host-keys.json";

/// Whether unknown server keys may be persisted.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HostKeyPolicy {
    /// Reject unknown and changed keys.
    #[default]
    Strict,
    /// Persist an unknown key, but always reject a changed key.
    ///
    /// Callers should use this only after an explicit user confirmation.
    AcceptNew,
    /// Accept every key.  This is intentionally opt-in and should not be used
    /// by normal application connection builders.
    Insecure,
}

/// The network path used to reach an SSH endpoint.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum HostKeyRoute {
    Direct,
    Proxy {
        proxy_type: HostKeyProxyType,
        host: String,
        port: u16,
    },
    Jump {
        host: String,
        port: u16,
    },
    JumpViaProxy {
        jump_host: String,
        jump_port: u16,
        proxy_type: HostKeyProxyType,
        proxy_host: String,
        proxy_port: u16,
    },
}

impl HostKeyRoute {
    pub(crate) fn normalize(self) -> Self {
        match self {
            Self::Direct => Self::Direct,
            Self::Proxy {
                proxy_type,
                host,
                port,
            } => Self::Proxy {
                proxy_type,
                host: normalize_host(&host),
                port,
            },
            Self::Jump { host, port } => Self::Jump {
                host: normalize_host(&host),
                port,
            },
            Self::JumpViaProxy {
                jump_host,
                jump_port,
                proxy_type,
                proxy_host,
                proxy_port,
            } => Self::JumpViaProxy {
                jump_host: normalize_host(&jump_host),
                jump_port,
                proxy_type,
                proxy_host: normalize_host(&proxy_host),
                proxy_port,
            },
        }
    }
}

/// Proxy protocol is part of the trust identity.  An HTTP CONNECT endpoint
/// and a SOCKS endpoint should not share a route namespace accidentally.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum HostKeyProxyType {
    Socks5,
    Http,
}

/// Canonical identity of a server host key.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct HostKeyIdentity {
    host: String,
    port: u16,
    route: HostKeyRoute,
}

impl HostKeyIdentity {
    #[must_use]
    pub fn new(host: impl Into<String>, port: u16, route: HostKeyRoute) -> Self {
        Self {
            host: normalize_host(&host.into()),
            port,
            route: route.normalize(),
        }
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub fn route(&self) -> &HostKeyRoute {
        &self.route
    }

    fn openssh_host(&self) -> String {
        if self.port == 22 {
            self.host.clone()
        } else {
            format!("[{}]:{}", self.host, self.port)
        }
    }
}

impl fmt::Display for HostKeyIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{} via {}",
            self.host,
            self.port,
            RouteDisplay(&self.route)
        )
    }
}

struct RouteDisplay<'a>(&'a HostKeyRoute);

impl fmt::Display for RouteDisplay<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            HostKeyRoute::Direct => formatter.write_str("direct"),
            HostKeyRoute::Proxy {
                proxy_type,
                host,
                port,
            } => write!(formatter, "{} proxy {}:{}", proxy_type, host, port),
            HostKeyRoute::Jump { host, port } => write!(formatter, "jump {}:{}", host, port),
            HostKeyRoute::JumpViaProxy {
                jump_host,
                jump_port,
                proxy_type,
                proxy_host,
                proxy_port,
            } => write!(
                formatter,
                "jump {}:{} via {} proxy {}:{}",
                jump_host, jump_port, proxy_type, proxy_host, proxy_port
            ),
        }
    }
}

impl fmt::Display for HostKeyProxyType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Socks5 => "socks5",
            Self::Http => "http",
        })
    }
}

/// Algorithm and SHA-256 fingerprint shown in diagnostics and confirmation UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostKeyDetails {
    pub algorithm: String,
    pub fingerprint: String,
}

impl HostKeyDetails {
    #[must_use]
    pub fn from_public_key(public_key: &PublicKey) -> Self {
        Self {
            algorithm: public_key.algorithm().as_str().to_owned(),
            fingerprint: public_key.fingerprint(HashAlg::Sha256).to_string(),
        }
    }
}

impl fmt::Display for HostKeyDetails {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.algorithm, self.fingerprint)
    }
}

/// Result of an accepted verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostKeyAcceptance {
    Known,
    AcceptedNew,
    AcceptedOnce,
    Insecure,
}

/// A host that has been trusted in the app's host-key trust store, surfaced to
/// the "Known Hosts" UI.  Time fields are seconds since the Unix epoch; they are
/// `None` for records created by an older trust-store version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnownHost {
    pub identity: HostKeyIdentity,
    pub algorithm: String,
    pub fingerprint: String,
    pub public_key: String,
    pub discovered_at: Option<u64>,
    pub last_seen: Option<u64>,
}

/// A fail-closed host-key rejection.  The details intentionally contain only
/// endpoint/key metadata; credentials and private key material never enter
/// these messages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostKeyRejection {
    Unknown {
        identity: HostKeyIdentity,
        presented: HostKeyDetails,
    },
    Changed {
        identity: HostKeyIdentity,
        presented: HostKeyDetails,
        expected: Vec<HostKeyDetails>,
    },
    Revoked {
        identity: HostKeyIdentity,
        presented: HostKeyDetails,
    },
    StoreUnavailable {
        identity: HostKeyIdentity,
        presented: HostKeyDetails,
        reason: String,
    },
}

impl fmt::Display for HostKeyRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown {
                identity,
                presented,
            } => write!(
                formatter,
                "unknown SSH host key for {identity}: {presented}; verify the fingerprint before enabling AcceptNew"
            ),
            Self::Changed {
                identity,
                presented,
                expected,
            } => write!(
                formatter,
                "changed SSH host key for {identity}: presented {presented}, expected {}",
                format_details(expected)
            ),
            Self::Revoked {
                identity,
                presented,
            } => write!(
                formatter,
                "revoked SSH host key for {identity}: {presented}"
            ),
            Self::StoreUnavailable {
                identity,
                presented,
                reason,
            } => write!(
                formatter,
                "cannot verify SSH host key for {identity} ({presented}): {reason}"
            ),
        }
    }
}

impl std::error::Error for HostKeyRejection {}

fn format_details(details: &[HostKeyDetails]) -> String {
    if details.is_empty() {
        return "<none>".to_owned();
    }
    details
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Shared verifier configuration.  Cloning it is cheap; the trust file itself
/// is protected by the process-wide lock below.
#[derive(Clone)]
pub struct HostKeyVerifier {
    policy: HostKeyPolicy,
    trust_store_path: Option<PathBuf>,
    openssh_known_hosts_path: Option<PathBuf>,
    confirmed_keys: Vec<ConfirmedHostKey>,
}

#[derive(Clone)]
struct ConfirmedHostKey {
    identity: HostKeyIdentity,
    details: HostKeyDetails,
    persist: bool,
    replace_existing: bool,
}

impl Default for HostKeyVerifier {
    fn default() -> Self {
        Self::new(
            HostKeyPolicy::Strict,
            default_trust_store_path(),
            default_openssh_known_hosts_path(),
        )
    }
}

impl HostKeyVerifier {
    #[must_use]
    pub fn new(
        policy: HostKeyPolicy,
        trust_store_path: Option<PathBuf>,
        openssh_known_hosts_path: Option<PathBuf>,
    ) -> Self {
        Self {
            policy,
            trust_store_path,
            openssh_known_hosts_path,
            confirmed_keys: Vec::new(),
        }
    }

    #[must_use]
    pub fn strict() -> Self {
        Self::new(
            HostKeyPolicy::Strict,
            default_trust_store_path(),
            default_openssh_known_hosts_path(),
        )
    }

    #[must_use]
    pub fn accept_new() -> Self {
        Self::new(
            HostKeyPolicy::AcceptNew,
            default_trust_store_path(),
            default_openssh_known_hosts_path(),
        )
    }

    /// Explicit insecure mode.  Normal connection builders must not use this.
    #[must_use]
    pub fn insecure() -> Self {
        Self::new(HostKeyPolicy::Insecure, None, None)
    }

    /// Construct a verifier backed by a caller-selected app trust file.
    ///
    /// OpenSSH `known_hosts` lookup is deliberately disabled for this
    /// constructor so tests and isolated trust namespaces cannot accidentally
    /// inherit a user's global entries.
    #[must_use]
    pub fn for_store(policy: HostKeyPolicy, path: impl Into<PathBuf>) -> Self {
        Self::new(policy, Some(path.into()), None)
    }

    #[must_use]
    pub fn policy(&self) -> HostKeyPolicy {
        self.policy
    }

    #[must_use]
    pub fn trust_store_path(&self) -> Option<&Path> {
        self.trust_store_path.as_deref()
    }

    #[must_use]
    pub fn openssh_known_hosts_path(&self) -> Option<&Path> {
        self.openssh_known_hosts_path.as_deref()
    }

    /// Return a verifier that accepts exactly the key the user confirmed.
    ///
    /// Confirmations are additive so jump-host and target-host prompts can be
    /// handled independently during the same connection attempt. Unknown keys
    /// that were not shown to the user remain fail-closed.
    #[must_use]
    pub fn with_confirmed_key(
        mut self,
        identity: HostKeyIdentity,
        details: HostKeyDetails,
        persist: bool,
    ) -> Self {
        self.confirmed_keys
            .retain(|entry| entry.identity != identity || entry.details != details);
        self.confirmed_keys.push(ConfirmedHostKey {
            identity,
            details,
            persist,
            replace_existing: false,
        });
        self
    }

    /// Return a verifier that accepts a changed key only after the user
    /// explicitly confirmed the presented replacement.
    #[must_use]
    pub fn with_confirmed_changed_key(
        mut self,
        identity: HostKeyIdentity,
        details: HostKeyDetails,
        persist: bool,
    ) -> Self {
        self.confirmed_keys
            .retain(|entry| entry.identity != identity || entry.details != details);
        self.confirmed_keys.push(ConfirmedHostKey {
            identity,
            details,
            persist,
            replace_existing: true,
        });
        self
    }

    /// Return the host-key algorithms already trusted for an identity.
    ///
    /// This is used before the SSH handshake so the client can prefer a key
    /// algorithm that is already trusted for this exact host, port, and route.
    /// The final host-key decision still compares the complete public key in
    /// [`Self::verify`]; this method never authorizes a different key.
    pub fn known_host_key_algorithms(
        &self,
        identity: &HostKeyIdentity,
    ) -> Result<Vec<String>, String> {
        // Preserve insecure mode's existing behavior: it must not become
        // dependent on the availability or validity of trust files.
        if self.policy == HostKeyPolicy::Insecure {
            return Ok(Vec::new());
        }

        let _lock = trust_store_lock();
        let app_entries = self.load_app_entries()?;
        let mut algorithms = Vec::new();
        for confirmation in self.confirmed_keys.iter().filter(|confirmation| {
            confirmation.identity == *identity && confirmation.replace_existing
        }) {
            push_unique_algorithm(&mut algorithms, &confirmation.details.algorithm);
        }

        let app_matches = app_entries
            .iter()
            .filter(|entry| entry.identity == *identity)
            .collect::<Vec<_>>();
        if !app_matches.is_empty() {
            for entry in app_matches {
                push_unique_algorithm(&mut algorithms, &entry.details.algorithm);
            }
            return Ok(algorithms);
        }

        let openssh_entries = self.load_openssh_entries(identity)?;
        if !openssh_entries.is_empty() {
            for entry in openssh_entries.iter().filter(|entry| !entry.revoked) {
                push_unique_algorithm(&mut algorithms, &entry.details.algorithm);
            }
            return Ok(algorithms);
        }

        for confirmation in self
            .confirmed_keys
            .iter()
            .filter(|confirmation| confirmation.identity == *identity)
        {
            push_unique_algorithm(&mut algorithms, &confirmation.details.algorithm);
        }
        Ok(algorithms)
    }

    /// Enumerate the hosts currently trusted in the app trust store, newest
    /// activity first.  Used by the Known Hosts UI and by import tools.
    ///
    /// Listing is tolerant: an individual entry that fails to re-parse (for
    /// example from an older or partial write) is skipped with a warning
    /// instead of hiding the rest of the list.  Structural store errors
    /// (invalid JSON, unsupported version) still fail the whole call.
    pub fn list_known_hosts(&self) -> Result<Vec<KnownHost>, String> {
        let _lock = trust_store_lock();
        let Some(persisted) = self.read_persisted_entries()? else {
            return Ok(Vec::new());
        };
        let path_label = self
            .trust_store_path
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<unset>".to_owned());
        let mut hosts = Vec::new();
        for entry in persisted {
            match StoredHostKey::try_from(entry) {
                Ok(entry) => hosts.push(KnownHost {
                    identity: entry.identity,
                    algorithm: entry.details.algorithm,
                    fingerprint: entry.details.fingerprint,
                    public_key: entry.public_key,
                    discovered_at: entry.discovered_at,
                    last_seen: entry.last_seen,
                }),
                Err(error) => tracing::warn!(
                    target: "host_key",
                    path = %path_label,
                    error,
                    "skipping invalid host-key trust entry while listing known hosts"
                ),
            }
        }
        hosts.sort_by_key(|host| std::cmp::Reverse(host.last_seen.unwrap_or_default()));
        Ok(hosts)
    }

    /// Remove one trusted host-key identity from the app trust store.
    pub fn remove_known_host(&self, identity: &HostKeyIdentity) -> Result<bool, String> {
        let _lock = trust_store_lock();
        let mut entries = self.load_app_entries()?;
        let original_len = entries.len();
        entries.retain(|entry| entry.identity != *identity);
        if entries.len() == original_len {
            return Ok(false);
        }
        self.save_app_entries(&entries)?;
        Ok(true)
    }

    /// Import host keys from an OpenSSH `known_hosts` file into the app trust
    /// store.  Only plain, concrete host patterns (host or `[host]:port`) are
    /// importable; hashed names, wildcards, negated patterns, certificate
    /// authorities and revoked keys are skipped.  Returns the number of hosts
    /// newly imported (already-trusted identities are left untouched).
    pub fn import_system_known_hosts(&self, path: &Path) -> Result<usize, String> {
        let input = match fs::read_to_string(path) {
            Ok(input) => input,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => {
                return Err(format!(
                    "read OpenSSH known_hosts {}: {error}",
                    path.display()
                ));
            }
        };

        let _lock = trust_store_lock();
        let mut entries = self.load_app_entries()?;
        let now = now_secs();
        let mut imported = 0usize;
        for parsed in ssh_key::known_hosts::KnownHosts::new(&input) {
            let entry = match parsed {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::warn!(path = %path.display(), error = %error, "skipping malformed known_hosts entry");
                    continue;
                }
            };
            if matches!(
                entry.marker(),
                Some(
                    ssh_key::known_hosts::Marker::CertAuthority
                        | ssh_key::known_hosts::Marker::Revoked
                )
            ) {
                continue;
            }
            let ssh_key::known_hosts::HostPatterns::Patterns(patterns) = entry.host_patterns()
            else {
                continue;
            };
            let public_key = entry.public_key().clone();
            let details = HostKeyDetails::from_public_key(&public_key);
            for pattern in patterns {
                let Some((host, port)) = known_host_pattern_to_identity(pattern) else {
                    continue;
                };
                let identity = HostKeyIdentity::new(host, port, HostKeyRoute::Direct);
                if entries.iter().any(|entry| entry.identity == identity) {
                    continue;
                }
                entries.push(StoredHostKey {
                    identity,
                    public_key: public_key_string(&public_key),
                    details: details.clone(),
                    discovered_at: Some(now),
                    last_seen: Some(now),
                });
                imported += 1;
            }
        }
        if imported > 0 {
            self.save_app_entries(&entries)?;
        }
        Ok(imported)
    }

    /// Verify a server key and, in `AcceptNew`, persist an unknown key.
    // 错误类型为语义完整的 HostKeyRejection(含身份/指纹/原因),装箱会波及全部调用方;
    // 该错误仅在主机密钥校验失败时构造,非热路径,维持值类型返回。
    #[allow(clippy::result_large_err)]
    pub fn verify(
        &self,
        identity: &HostKeyIdentity,
        server_public_key: &PublicKey,
    ) -> Result<HostKeyAcceptance, HostKeyRejection> {
        let presented = HostKeyDetails::from_public_key(server_public_key);

        if self.policy == HostKeyPolicy::Insecure {
            return Ok(HostKeyAcceptance::Insecure);
        }

        let _lock = trust_store_lock();
        let app_entries = match self.load_app_entries() {
            Ok(entries) => entries,
            Err(reason) => {
                return Err(HostKeyRejection::StoreUnavailable {
                    identity: identity.clone(),
                    presented,
                    reason,
                });
            }
        };

        let presented_public_key = public_key_string(server_public_key);
        let confirmation = self.confirmed_keys.iter().find(|confirmation| {
            confirmation.identity == *identity && confirmation.details == presented
        });
        let openssh_lookup = match self.lookup_openssh(identity, server_public_key) {
            Ok(OpenSshLookup::Revoked) => {
                return Err(HostKeyRejection::Revoked {
                    identity: identity.clone(),
                    presented,
                });
            }
            Ok(lookup) => lookup,
            Err(reason) => {
                return Err(HostKeyRejection::StoreUnavailable {
                    identity: identity.clone(),
                    presented,
                    reason,
                });
            }
        };

        let mut app_entries = app_entries;
        let mut app_known_index = None;
        let mut app_has_entry = false;
        for (index, entry) in app_entries.iter().enumerate() {
            if entry.identity != *identity {
                continue;
            }
            app_has_entry = true;
            if entry.public_key == presented_public_key {
                app_known_index = Some(index);
            }
        }

        if app_has_entry {
            if let Some(index) = app_known_index {
                if let Some(entry) = app_entries.get_mut(index) {
                    entry.last_seen = Some(now_secs());
                }
                if let Err(reason) = self.save_app_entries(&app_entries) {
                    // A timestamp write must not turn a successful verification
                    // into a failure; the trust decision is already made.
                    tracing::warn!(
                        target: "host_key",
                        identity = %identity,
                        reason,
                        "could not record known-host last_seen"
                    );
                }
                return Ok(HostKeyAcceptance::Known);
            }
            if let Some(confirmation) = confirmation.filter(|entry| entry.replace_existing) {
                return self.accept_confirmation(identity, server_public_key, confirmation);
            }
            return Err(HostKeyRejection::Changed {
                identity: identity.clone(),
                presented,
                expected: app_entries
                    .iter()
                    .filter(|entry| entry.identity == *identity)
                    .map(|entry| entry.details.clone())
                    .collect(),
            });
        }

        match openssh_lookup {
            OpenSshLookup::Known => return Ok(HostKeyAcceptance::Known),
            OpenSshLookup::Changed(expected) => {
                if let Some(confirmation) = confirmation.filter(|entry| entry.replace_existing) {
                    return self.accept_confirmation(identity, server_public_key, confirmation);
                }
                return Err(HostKeyRejection::Changed {
                    identity: identity.clone(),
                    presented,
                    expected,
                });
            }
            OpenSshLookup::Unknown => {}
            OpenSshLookup::Revoked => unreachable!("revoked keys are rejected before app trust"),
        }

        if let Some(confirmation) = confirmation.filter(|entry| !entry.replace_existing) {
            return self.accept_confirmation(identity, server_public_key, confirmation);
        }

        if self.policy == HostKeyPolicy::AcceptNew {
            let public_key = public_key_string(server_public_key);
            let details = HostKeyDetails::from_public_key(server_public_key);
            let now = now_secs();
            let mut entries = app_entries;
            entries.push(StoredHostKey {
                identity: identity.clone(),
                public_key,
                details,
                discovered_at: Some(now),
                last_seen: Some(now),
            });
            if let Err(reason) = self.save_app_entries(&entries) {
                return Err(HostKeyRejection::StoreUnavailable {
                    identity: identity.clone(),
                    presented,
                    reason,
                });
            }
            return Ok(HostKeyAcceptance::AcceptedNew);
        }

        Err(HostKeyRejection::Unknown {
            identity: identity.clone(),
            presented,
        })
    }

    // 错误类型为语义完整的 HostKeyRejection,装箱会波及调用方;仅在确认持久化失败时构造,维持值类型。
    #[allow(clippy::result_large_err)]
    fn accept_confirmation(
        &self,
        identity: &HostKeyIdentity,
        server_public_key: &PublicKey,
        confirmation: &ConfirmedHostKey,
    ) -> Result<HostKeyAcceptance, HostKeyRejection> {
        if !confirmation.persist {
            return Ok(HostKeyAcceptance::AcceptedOnce);
        }

        let presented = HostKeyDetails::from_public_key(server_public_key);
        let mut entries =
            self.load_app_entries()
                .map_err(|reason| HostKeyRejection::StoreUnavailable {
                    identity: identity.clone(),
                    presented: presented.clone(),
                    reason,
                })?;
        if confirmation.replace_existing {
            entries.retain(|entry| entry.identity != *identity);
        }
        let now = now_secs();
        entries.push(StoredHostKey {
            identity: identity.clone(),
            public_key: public_key_string(server_public_key),
            details: presented.clone(),
            discovered_at: Some(now),
            last_seen: Some(now),
        });
        self.save_app_entries(&entries)
            .map_err(|reason| HostKeyRejection::StoreUnavailable {
                identity: identity.clone(),
                presented,
                reason,
            })?;
        Ok(HostKeyAcceptance::AcceptedNew)
    }

    fn load_app_entries(&self) -> Result<Vec<StoredHostKey>, String> {
        let Some(entries) = self.read_persisted_entries()? else {
            return Ok(Vec::new());
        };
        entries.into_iter().map(StoredHostKey::try_from).collect()
    }

    /// Read and structurally validate the trust store, returning `None` when
    /// the file does not exist yet.  Entries are returned unvalidated so
    /// callers can decide whether one bad row is fatal (verification) or only
    /// worth skipping (listing).
    fn read_persisted_entries(&self) -> Result<Option<Vec<PersistedHostKey>>, String> {
        let Some(path) = &self.trust_store_path else {
            return Ok(Some(Vec::new()));
        };
        match fs::read(path) {
            Ok(bytes) => {
                let store: PersistedTrustStore =
                    serde_json::from_slice(&bytes).map_err(|error| {
                        format!("invalid host-key trust store {}: {error}", path.display())
                    })?;
                if store.version != TRUST_STORE_VERSION {
                    return Err(format!(
                        "unsupported host-key trust store version {} in {}",
                        store.version,
                        path.display()
                    ));
                }
                Ok(Some(store.entries))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!(
                "read host-key trust store {}: {error}",
                path.display()
            )),
        }
    }

    fn save_app_entries(&self, entries: &[StoredHostKey]) -> Result<(), String> {
        let Some(path) = &self.trust_store_path else {
            return Err("host-key trust store is not configured".to_owned());
        };
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create host-key trust directory {}: {error}",
                parent.display()
            )
        })?;

        let persisted = PersistedTrustStore {
            version: TRUST_STORE_VERSION,
            entries: entries.iter().map(PersistedHostKey::from).collect(),
        };
        let mut temporary = NamedTempFile::new_in(parent).map_err(|error| {
            format!(
                "create host-key trust temp file in {}: {error}",
                parent.display()
            )
        })?;
        serde_json::to_writer_pretty(temporary.as_file_mut(), &persisted)
            .map_err(|error| format!("serialize host-key trust store: {error}"))?;
        temporary
            .write_all(b"\n")
            .and_then(|_| temporary.as_file_mut().flush())
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|error| format!("flush host-key trust store: {error}"))?;
        temporary.persist(path).map_err(|error| {
            format!(
                "atomically replace host-key trust store {}: {error}",
                path.display()
            )
        })?;

        // Persisting the file makes the rename atomic, but syncing the parent
        // directory closes the durability window on Unix filesystems.
        sync_parent_directory(parent).map_err(|error| {
            format!(
                "sync host-key trust directory {}: {error}",
                parent.display()
            )
        })?;
        Ok(())
    }

    fn lookup_openssh(
        &self,
        identity: &HostKeyIdentity,
        server_public_key: &PublicKey,
    ) -> Result<OpenSshLookup, String> {
        let entries = self.load_openssh_entries(identity)?;
        if entries.is_empty() {
            return Ok(OpenSshLookup::Unknown);
        }

        let mut expected = Vec::new();
        for entry in entries {
            if entry.revoked && entry.public_key == *server_public_key {
                return Ok(OpenSshLookup::Revoked);
            }
            if entry.public_key == *server_public_key {
                return Ok(OpenSshLookup::Known);
            }
            if !expected.contains(&entry.details) {
                expected.push(entry.details);
            }
        }
        Ok(OpenSshLookup::Changed(expected))
    }

    fn load_openssh_entries(
        &self,
        identity: &HostKeyIdentity,
    ) -> Result<Vec<OpenSshHostKey>, String> {
        let Some(path) = &self.openssh_known_hosts_path else {
            return Ok(Vec::new());
        };
        let input = match fs::read_to_string(path) {
            Ok(input) => input,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(format!(
                    "read OpenSSH known_hosts {}: {error}",
                    path.display()
                ));
            }
        };

        let candidate = identity.openssh_host();
        let mut entries = Vec::new();
        for parsed in ssh_key::known_hosts::KnownHosts::new(&input) {
            let entry = match parsed {
                Ok(entry) => entry,
                Err(error) => {
                    // OpenSSH ignores malformed lines while continuing to use
                    // the rest of the file.  Unknown keys still fail closed.
                    tracing::warn!(path = %path.display(), error = %error, "ignoring malformed known_hosts entry");
                    continue;
                }
            };
            if !known_hosts_match(entry.host_patterns(), &candidate) {
                continue;
            }
            if entry.marker() == Some(&ssh_key::known_hosts::Marker::CertAuthority) {
                // Certificate-authority entries require certificate validation,
                // which russh does not expose through this callback yet.
                continue;
            }
            let details = HostKeyDetails::from_public_key(entry.public_key());
            entries.push(OpenSshHostKey {
                public_key: entry.public_key().clone(),
                details,
                revoked: entry.marker() == Some(&ssh_key::known_hosts::Marker::Revoked),
            });
        }

        Ok(entries)
    }
}

#[derive(Clone, Debug)]
struct StoredHostKey {
    identity: HostKeyIdentity,
    public_key: String,
    details: HostKeyDetails,
    discovered_at: Option<u64>,
    last_seen: Option<u64>,
}

struct OpenSshHostKey {
    public_key: PublicKey,
    details: HostKeyDetails,
    revoked: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct PersistedTrustStore {
    version: u32,
    entries: Vec<PersistedHostKey>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PersistedHostKey {
    host: String,
    port: u16,
    route: HostKeyRoute,
    public_key: String,
    algorithm: String,
    fingerprint: String,
    #[serde(default)]
    discovered_at: Option<u64>,
    #[serde(default)]
    last_seen: Option<u64>,
}

impl From<&StoredHostKey> for PersistedHostKey {
    fn from(entry: &StoredHostKey) -> Self {
        Self {
            host: entry.identity.host.clone(),
            port: entry.identity.port,
            route: entry.identity.route.clone(),
            public_key: entry.public_key.clone(),
            algorithm: entry.details.algorithm.clone(),
            fingerprint: entry.details.fingerprint.clone(),
            discovered_at: entry.discovered_at,
            last_seen: entry.last_seen,
        }
    }
}

impl TryFrom<PersistedHostKey> for StoredHostKey {
    type Error = String;

    fn try_from(entry: PersistedHostKey) -> Result<Self, Self::Error> {
        let parsed = entry
            .public_key
            .parse::<PublicKey>()
            .map_err(|error| format!("invalid stored host key: {error}"))?;
        let details = HostKeyDetails::from_public_key(&parsed);
        if details.algorithm != entry.algorithm || details.fingerprint != entry.fingerprint {
            return Err("stored host-key metadata does not match its public key".to_owned());
        }
        Ok(Self {
            identity: HostKeyIdentity::new(entry.host, entry.port, entry.route),
            public_key: entry.public_key,
            details,
            discovered_at: entry.discovered_at,
            last_seen: entry.last_seen,
        })
    }
}

enum OpenSshLookup {
    Known,
    Changed(Vec<HostKeyDetails>),
    Revoked,
    Unknown,
}

fn public_key_string(public_key: &PublicKey) -> String {
    public_key
        .to_openssh()
        .unwrap_or_else(|_| public_key.to_string())
}

fn push_unique_algorithm(algorithms: &mut Vec<String>, algorithm: &str) {
    if !algorithms.iter().any(|known| known == algorithm) {
        algorithms.push(algorithm.to_owned());
    }
}

pub(crate) fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn default_trust_store_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join(".config")
            .join("navop")
            .join(TRUST_STORE_FILE_NAME)
    })
}

fn default_openssh_known_hosts_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh").join("known_hosts"))
}

fn trust_store_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// Recover a concrete (host, port) from an OpenSSH `known_hosts` pattern.
/// Returns `None` for hashed names, wildcards, negations and ambiguous forms
/// (bare `host:port` without brackets is treated as an IPv6 literal and skipped).
fn known_host_pattern_to_identity(pattern: &str) -> Option<(String, u16)> {
    let pattern = pattern.trim();
    if pattern.is_empty()
        || pattern.starts_with('!')
        || pattern.starts_with('|')
        || pattern.contains('*')
        || pattern.contains('?')
    {
        return None;
    }
    let (host, port) = if let Some(rest) = pattern.strip_prefix('[') {
        let Some((host, remainder)) = rest.split_once(']') else {
            return None;
        };
        let port = match remainder.strip_prefix(':') {
            Some(next) => next.parse::<u16>().ok()?,
            None => 22,
        };
        (host, port)
    } else {
        if pattern.contains(':') {
            return None;
        }
        (pattern, 22)
    };
    if host.is_empty() {
        return None;
    }
    Some((host.to_owned(), port))
}

fn sync_parent_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn known_hosts_match(patterns: &ssh_key::known_hosts::HostPatterns, candidate: &str) -> bool {
    match patterns {
        ssh_key::known_hosts::HostPatterns::Patterns(patterns) => {
            let mut positive_match = false;
            for pattern in patterns {
                let (negated, pattern) = match pattern.strip_prefix('!') {
                    Some(pattern) => (true, pattern),
                    None => (false, pattern.as_str()),
                };
                if glob_matches(
                    &pattern.to_ascii_lowercase(),
                    &candidate.to_ascii_lowercase(),
                ) {
                    if negated {
                        return false;
                    }
                    positive_match = true;
                }
            }
            positive_match
        }
        ssh_key::known_hosts::HostPatterns::HashedName { salt, hash } => {
            let Ok(mut mac) = HmacSha1::new_from_slice(salt) else {
                return false;
            };
            mac.update(candidate.as_bytes());
            mac.verify_slice(hash).is_ok()
        }
    }
}

fn glob_matches(pattern: &str, candidate: &str) -> bool {
    // Small wildcard matcher for OpenSSH's '*' and '?' host patterns.
    let pattern = pattern.as_bytes();
    let candidate = candidate.as_bytes();
    let mut table = vec![vec![false; candidate.len() + 1]; pattern.len() + 1];
    table[0][0] = true;
    for index in 0..pattern.len() {
        if pattern[index] == b'*' {
            table[index + 1][0] = table[index][0];
        }
    }
    for p in 0..pattern.len() {
        for c in 0..candidate.len() {
            table[p + 1][c + 1] = match pattern[p] {
                b'*' => table[p][c + 1] || table[p + 1][c],
                b'?' => table[p][c],
                byte => table[p][c] && byte == candidate[c],
            };
        }
    }
    table[pattern.len()][candidate.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_010::rng;
    use tempfile::TempDir;

    fn key(algorithm: ssh_key::Algorithm) -> PublicKey {
        ssh_key::PrivateKey::random(&mut rng(), algorithm)
            .expect("test host key should be generated")
            .public_key()
            .clone()
    }

    fn identity(host: &str, port: u16) -> HostKeyIdentity {
        HostKeyIdentity::new(host, port, HostKeyRoute::Direct)
    }

    fn verifier(temp: &TempDir, policy: HostKeyPolicy) -> HostKeyVerifier {
        HostKeyVerifier::for_store(policy, temp.path().join("keys.json"))
    }

    #[test]
    fn strict_rejects_unknown_key_with_fingerprint() {
        let temp = TempDir::new().expect("temp dir");
        let verifier = verifier(&temp, HostKeyPolicy::Strict);
        let presented = key(ssh_key::Algorithm::Ed25519);
        let error = verifier
            .verify(&identity("Example.COM.", 22), &presented)
            .expect_err("unknown key must be rejected");
        let HostKeyRejection::Unknown {
            identity,
            presented: details,
        } = error
        else {
            panic!("expected unknown rejection");
        };
        assert_eq!(identity.host(), "example.com");
        assert!(details.fingerprint.starts_with("SHA256:"));
    }

    #[test]
    fn accept_new_persists_and_strict_reuses_known_key() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let first = key(ssh_key::Algorithm::Ed25519);
        let id = identity("host.example", 2200);
        assert_eq!(
            HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path)
                .verify(&id, &first)
                .expect("accept-new should persist"),
            HostKeyAcceptance::AcceptedNew
        );
        assert!(path.exists());
        assert_eq!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
                .verify(&id, &first)
                .expect("persisted key should be known"),
            HostKeyAcceptance::Known
        );
    }

    #[test]
    fn confirmed_once_accepts_only_exact_key_without_persisting() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let id = identity("host.example", 22);
        let presented = key(ssh_key::Algorithm::Ed25519);
        let details = HostKeyDetails::from_public_key(&presented);

        assert_eq!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
                .with_confirmed_key(id.clone(), details, false)
                .verify(&id, &presented)
                .expect("the explicitly confirmed key should be accepted once"),
            HostKeyAcceptance::AcceptedOnce
        );
        assert!(!path.exists());
        assert!(matches!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path).verify(&id, &presented),
            Err(HostKeyRejection::Unknown { .. })
        ));
    }

    #[test]
    fn confirmed_and_saved_key_is_reused_by_strict_verifier() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let id = identity("host.example", 22);
        let presented = key(ssh_key::Algorithm::Ed25519);
        let details = HostKeyDetails::from_public_key(&presented);

        assert_eq!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
                .with_confirmed_key(id.clone(), details, true)
                .verify(&id, &presented)
                .expect("the explicitly confirmed key should be persisted"),
            HostKeyAcceptance::AcceptedNew
        );
        assert_eq!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
                .verify(&id, &presented)
                .expect("the persisted key should be trusted"),
            HostKeyAcceptance::Known
        );
    }

    #[test]
    fn confirmation_does_not_accept_another_unknown_key() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let confirmed_id = identity("confirmed.example", 22);
        let other_id = identity("other.example", 22);
        let confirmed_key = key(ssh_key::Algorithm::Ed25519);
        let other_key = key(ssh_key::Algorithm::Ed25519);
        let verifier = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path).with_confirmed_key(
            confirmed_id,
            HostKeyDetails::from_public_key(&confirmed_key),
            false,
        );

        assert!(matches!(
            verifier.verify(&other_id, &other_key),
            Err(HostKeyRejection::Unknown { .. })
        ));
    }

    #[test]
    fn confirmations_are_additive_for_jump_and_target_hosts() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let jump_id = identity("jump.example", 22);
        let target_id = identity("target.example", 22);
        let jump_key = key(ssh_key::Algorithm::Ed25519);
        let target_key = key(ssh_key::Algorithm::Ed25519);
        let verifier = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
            .with_confirmed_key(
                jump_id.clone(),
                HostKeyDetails::from_public_key(&jump_key),
                false,
            )
            .with_confirmed_key(
                target_id.clone(),
                HostKeyDetails::from_public_key(&target_key),
                false,
            );

        assert_eq!(
            verifier.verify(&jump_id, &jump_key).expect("jump key"),
            HostKeyAcceptance::AcceptedOnce
        );
        assert_eq!(
            verifier
                .verify(&target_id, &target_key)
                .expect("target key"),
            HostKeyAcceptance::AcceptedOnce
        );
    }

    #[test]
    fn changed_key_is_rejected_even_in_accept_new_mode() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let id = identity("host.example", 22);
        let first = key(ssh_key::Algorithm::Ed25519);
        let changed = key(ssh_key::Algorithm::Ed25519);
        let verifier = HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path);
        verifier.verify(&id, &first).expect("seed key");
        let error = verifier
            .verify(&id, &changed)
            .expect_err("changed key must remain blocked");
        assert!(matches!(error, HostKeyRejection::Changed { .. }));
    }

    #[test]
    fn changed_algorithm_is_rejected() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let id = identity("host.example", 22);
        let first = key(ssh_key::Algorithm::Ed25519);
        let changed = key(ssh_key::Algorithm::Ecdsa {
            curve: ssh_key::EcdsaCurve::NistP256,
        });
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path)
            .verify(&id, &first)
            .expect("seed key");

        let error = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
            .verify(&id, &changed)
            .expect_err("algorithm changes must remain blocked");
        let HostKeyRejection::Changed {
            presented,
            expected,
            ..
        } = error
        else {
            panic!("expected changed rejection");
        };
        assert_ne!(presented.algorithm, expected[0].algorithm);
    }

    #[test]
    fn confirmed_changed_key_is_accepted_once_without_replacing_trust() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let id = identity("host.example", 22);
        let original = key(ssh_key::Algorithm::Ed25519);
        let changed = key(ssh_key::Algorithm::Ed25519);
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path)
            .verify(&id, &original)
            .expect("seed original key");

        assert_eq!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
                .with_confirmed_changed_key(
                    id.clone(),
                    HostKeyDetails::from_public_key(&changed),
                    false,
                )
                .verify(&id, &changed)
                .expect("explicitly confirmed changed key should be accepted once"),
            HostKeyAcceptance::AcceptedOnce
        );
        assert_eq!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
                .verify(&id, &original)
                .expect("accept-once must preserve the original trust entry"),
            HostKeyAcceptance::Known
        );
        assert!(matches!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path).verify(&id, &changed),
            Err(HostKeyRejection::Changed { .. })
        ));
    }

    #[test]
    fn confirmed_changed_key_replaces_app_trust_when_saved() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let id = identity("host.example", 22);
        let original = key(ssh_key::Algorithm::Ed25519);
        let changed = key(ssh_key::Algorithm::Ed25519);
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path)
            .verify(&id, &original)
            .expect("seed original key");

        assert_eq!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
                .with_confirmed_changed_key(
                    id.clone(),
                    HostKeyDetails::from_public_key(&changed),
                    true,
                )
                .verify(&id, &changed)
                .expect("explicitly confirmed changed key should replace app trust"),
            HostKeyAcceptance::AcceptedNew
        );
        assert_eq!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
                .verify(&id, &changed)
                .expect("replacement key should be trusted"),
            HostKeyAcceptance::Known
        );
        assert!(matches!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path).verify(&id, &original),
            Err(HostKeyRejection::Changed { .. })
        ));

        let entries = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
            .load_app_entries()
            .expect("trust store should load");
        assert_eq!(
            1,
            entries.iter().filter(|entry| entry.identity == id).count()
        );
    }

    #[test]
    fn confirmed_changed_key_cannot_bypass_openssh_revocation() {
        let temp = TempDir::new().expect("temp dir");
        let app_store_path = temp.path().join("keys.json");
        let known_hosts_path = temp.path().join("known_hosts");
        let id = identity("host.example", 22);
        let original = key(ssh_key::Algorithm::Ed25519);
        let changed = key(ssh_key::Algorithm::Ed25519);
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &app_store_path)
            .verify(&id, &original)
            .expect("seed original app trust");
        fs::write(
            &known_hosts_path,
            format!(
                "@revoked {} {}\n",
                id.openssh_host(),
                changed.to_openssh().expect("changed key should encode"),
            ),
        )
        .expect("write revoked known_hosts entry");

        let error = HostKeyVerifier::new(
            HostKeyPolicy::Strict,
            Some(app_store_path),
            Some(known_hosts_path),
        )
        .with_confirmed_changed_key(id.clone(), HostKeyDetails::from_public_key(&changed), false)
        .verify(&id, &changed)
        .expect_err("changed-key confirmation must not bypass OpenSSH revocation");

        assert!(matches!(
            error,
            HostKeyRejection::Revoked {
                identity,
                presented,
            } if identity == id && presented == HostKeyDetails::from_public_key(&changed)
        ));
    }

    #[test]
    fn app_trust_cannot_bypass_openssh_revocation() {
        let temp = TempDir::new().expect("temp dir");
        let app_store_path = temp.path().join("keys.json");
        let known_hosts_path = temp.path().join("known_hosts");
        let id = identity("host.example", 22);
        let presented = key(ssh_key::Algorithm::Ed25519);
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &app_store_path)
            .verify(&id, &presented)
            .expect("seed app trust");
        fs::write(
            &known_hosts_path,
            format!(
                "@revoked {} {}\n",
                id.openssh_host(),
                presented.to_openssh().expect("host key should encode"),
            ),
        )
        .expect("write revoked known_hosts entry");

        let error = HostKeyVerifier::new(
            HostKeyPolicy::Strict,
            Some(app_store_path),
            Some(known_hosts_path),
        )
        .verify(&id, &presented)
        .expect_err("app trust must not bypass OpenSSH revocation");

        assert!(matches!(
            error,
            HostKeyRejection::Revoked {
                identity,
                presented: details,
            } if identity == id && details == HostKeyDetails::from_public_key(&presented)
        ));
    }

    #[test]
    fn changed_confirmation_cannot_bypass_openssh_store_error() {
        let temp = TempDir::new().expect("temp dir");
        let app_store_path = temp.path().join("keys.json");
        let known_hosts_path = temp.path().join("known_hosts");
        let id = identity("host.example", 22);
        let original = key(ssh_key::Algorithm::Ed25519);
        let changed = key(ssh_key::Algorithm::Ed25519);
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &app_store_path)
            .verify(&id, &original)
            .expect("seed original app trust");
        fs::write(&known_hosts_path, [0xff]).expect("write invalid UTF-8 known_hosts");

        let error = HostKeyVerifier::new(
            HostKeyPolicy::Strict,
            Some(app_store_path),
            Some(known_hosts_path),
        )
        .with_confirmed_changed_key(id.clone(), HostKeyDetails::from_public_key(&changed), false)
        .verify(&id, &changed)
        .expect_err("changed-key confirmation must not bypass known_hosts read errors");

        assert!(matches!(
            error,
            HostKeyRejection::StoreUnavailable {
                identity,
                presented,
                ..
            } if identity == id && presented == HostKeyDetails::from_public_key(&changed)
        ));
    }

    #[test]
    fn confirmed_openssh_changed_key_persists_without_rewriting_known_hosts() {
        let temp = TempDir::new().expect("temp dir");
        let app_store_path = temp.path().join("keys.json");
        let known_hosts_path = temp.path().join("known_hosts");
        let id = identity("host.example", 22);
        let original = key(ssh_key::Algorithm::Ed25519);
        let changed = key(ssh_key::Algorithm::Ed25519);
        let original_line = format!(
            "{} {}\n",
            id.openssh_host(),
            original.to_openssh().expect("original key should encode"),
        );
        fs::write(&known_hosts_path, original_line).expect("write known_hosts");
        let known_hosts_before = fs::read(&known_hosts_path).expect("read known_hosts before");

        assert!(matches!(
            HostKeyVerifier::new(
                HostKeyPolicy::Strict,
                Some(app_store_path.clone()),
                Some(known_hosts_path.clone()),
            )
            .verify(&id, &changed),
            Err(HostKeyRejection::Changed { .. })
        ));

        assert_eq!(
            HostKeyVerifier::new(
                HostKeyPolicy::Strict,
                Some(app_store_path.clone()),
                Some(known_hosts_path.clone()),
            )
            .with_confirmed_changed_key(
                id.clone(),
                HostKeyDetails::from_public_key(&changed),
                true,
            )
            .verify(&id, &changed)
            .expect("confirmed replacement should persist"),
            HostKeyAcceptance::AcceptedNew
        );
        assert_eq!(
            known_hosts_before,
            fs::read(&known_hosts_path).expect("read known_hosts after"),
            "OpenSSH known_hosts must remain byte-for-byte unchanged",
        );
        assert_eq!(
            HostKeyVerifier::new(
                HostKeyPolicy::Strict,
                Some(app_store_path),
                Some(known_hosts_path),
            )
            .verify(&id, &changed)
            .expect("persisted replacement should be known"),
            HostKeyAcceptance::Known
        );
    }

    #[test]
    fn changed_confirmation_is_bound_to_exact_identity_and_key() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let id = identity("host.example", 22);
        let other_id = identity("other.example", 22);
        let original = key(ssh_key::Algorithm::Ed25519);
        let confirmed = key(ssh_key::Algorithm::Ed25519);
        let other = key(ssh_key::Algorithm::Ed25519);
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path)
            .verify(&id, &original)
            .expect("seed original key");
        let verifier = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
            .with_confirmed_changed_key(
                id.clone(),
                HostKeyDetails::from_public_key(&confirmed),
                false,
            );

        assert!(matches!(
            verifier.verify(&id, &other),
            Err(HostKeyRejection::Changed { .. })
        ));
        assert!(matches!(
            verifier.verify(&other_id, &confirmed),
            Err(HostKeyRejection::Unknown { .. })
        ));
    }

    #[test]
    fn known_algorithms_follow_app_identity_and_route_isolation() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let direct = identity("host.example", 22);
        let proxy = HostKeyIdentity::new(
            "host.example",
            22,
            HostKeyRoute::Proxy {
                proxy_type: HostKeyProxyType::Socks5,
                host: "proxy.example".to_owned(),
                port: 1080,
            },
        );
        let ecdsa = key(ssh_key::Algorithm::Ecdsa {
            curve: ssh_key::EcdsaCurve::NistP256,
        });
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path)
            .verify(&direct, &ecdsa)
            .expect("seed direct key");

        let verifier = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path);
        assert_eq!(
            verifier
                .known_host_key_algorithms(&direct)
                .expect("known algorithms should load"),
            vec!["ecdsa-sha2-nistp256"]
        );
        assert!(
            verifier
                .known_host_key_algorithms(&proxy)
                .expect("proxy identity lookup should load")
                .is_empty()
        );
    }

    #[test]
    fn app_algorithms_keep_precedence_over_openssh_algorithms() {
        let temp = TempDir::new().expect("temp dir");
        let app_path = temp.path().join("keys.json");
        let openssh_path = temp.path().join("known_hosts");
        let id = identity("host.example", 22);
        let ecdsa = key(ssh_key::Algorithm::Ecdsa {
            curve: ssh_key::EcdsaCurve::NistP256,
        });
        let ed25519 = key(ssh_key::Algorithm::Ed25519);
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &app_path)
            .verify(&id, &ecdsa)
            .expect("seed app key");
        fs::write(
            &openssh_path,
            format!("host.example {}\n", public_key_string(&ed25519)),
        )
        .expect("write known_hosts");

        let verifier =
            HostKeyVerifier::new(HostKeyPolicy::Strict, Some(app_path), Some(openssh_path));
        assert_eq!(
            verifier
                .known_host_key_algorithms(&id)
                .expect("known algorithms should load"),
            vec!["ecdsa-sha2-nistp256"]
        );
    }

    #[test]
    fn confirmed_key_algorithm_is_known_before_handshake() {
        let temp = TempDir::new().expect("temp dir");
        let id = identity("host.example", 22);
        let verifier = verifier(&temp, HostKeyPolicy::Strict).with_confirmed_key(
            id.clone(),
            HostKeyDetails {
                algorithm: "ecdsa-sha2-nistp384".to_owned(),
                fingerprint: "SHA256:test".to_owned(),
            },
            false,
        );

        assert_eq!(
            verifier
                .known_host_key_algorithms(&id)
                .expect("confirmed algorithm should be available"),
            vec!["ecdsa-sha2-nistp384"]
        );
    }

    #[test]
    fn confirmed_changed_algorithm_is_preferred_before_handshake() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let id = identity("host.example", 22);
        let original = key(ssh_key::Algorithm::Ed25519);
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path)
            .verify(&id, &original)
            .expect("seed original key");

        let verifier = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
            .with_confirmed_changed_key(
                id.clone(),
                HostKeyDetails {
                    algorithm: "ecdsa-sha2-nistp384".to_owned(),
                    fingerprint: "SHA256:test".to_owned(),
                },
                false,
            );

        assert_eq!(
            verifier
                .known_host_key_algorithms(&id)
                .expect("confirmed changed algorithm should be preferred"),
            vec!["ecdsa-sha2-nistp384", "ssh-ed25519"]
        );
    }

    #[test]
    fn host_port_and_route_are_separate_trust_identities() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let key = key(ssh_key::Algorithm::Ed25519);
        let direct = HostKeyIdentity::new("host.example", 22, HostKeyRoute::Direct);
        let proxy = HostKeyIdentity::new(
            "host.example",
            22,
            HostKeyRoute::Proxy {
                proxy_type: HostKeyProxyType::Socks5,
                host: "proxy.example".to_owned(),
                port: 1080,
            },
        );
        let other_port = identity("host.example", 2222);
        let verifier = HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path);
        verifier.verify(&direct, &key).expect("seed direct key");
        assert!(matches!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path).verify(&proxy, &key),
            Err(HostKeyRejection::Unknown { .. })
        ));
        assert!(matches!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path).verify(&other_port, &key),
            Err(HostKeyRejection::Unknown { .. })
        ));
    }

    #[test]
    fn insecure_mode_is_explicit_and_does_not_write_trust() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let verifier = HostKeyVerifier::new(HostKeyPolicy::Insecure, Some(path.clone()), None);
        let result = verifier
            .verify(
                &identity("host.example", 22),
                &key(ssh_key::Algorithm::Ed25519),
            )
            .expect("insecure mode should accept");
        assert_eq!(result, HostKeyAcceptance::Insecure);
        assert!(!path.exists());
    }

    #[test]
    fn openssh_known_hosts_accepts_matching_host_and_port() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("known_hosts");
        let presented = key(ssh_key::Algorithm::Ed25519);
        fs::write(
            &path,
            format!("[known.example]:2200 {}\n", public_key_string(&presented)),
        )
        .expect("write known_hosts");
        let verifier = HostKeyVerifier::new(HostKeyPolicy::Strict, None, Some(path));

        assert_eq!(
            verifier
                .verify(&identity("known.example", 2200), &presented)
                .expect("matching known_hosts entry should be accepted"),
            HostKeyAcceptance::Known
        );
        assert!(matches!(
            verifier.verify(&identity("known.example", 22), &presented),
            Err(HostKeyRejection::Unknown { .. })
        ));
    }

    #[test]
    fn openssh_revoked_key_is_rejected() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("known_hosts");
        let presented = key(ssh_key::Algorithm::Ed25519);
        fs::write(
            &path,
            format!(
                "@revoked revoked.example {}\n",
                public_key_string(&presented)
            ),
        )
        .expect("write known_hosts");
        let verifier = HostKeyVerifier::new(HostKeyPolicy::Strict, None, Some(path));

        assert!(matches!(
            verifier.verify(&identity("revoked.example", 22), &presented),
            Err(HostKeyRejection::Revoked { .. })
        ));
    }

    #[test]
    fn concurrent_accept_new_updates_do_not_lose_entries() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let entries = (0..8)
            .map(|index| {
                (
                    identity(&format!("host-{index}.example"), 22),
                    key(ssh_key::Algorithm::Ed25519),
                )
            })
            .collect::<Vec<_>>();
        let threads = entries
            .iter()
            .cloned()
            .map(|(identity, key)| {
                let path = path.clone();
                std::thread::spawn(move || {
                    HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, path)
                        .verify(&identity, &key)
                        .expect("concurrent accept-new should persist")
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            assert_eq!(
                thread.join().expect("verification thread should finish"),
                HostKeyAcceptance::AcceptedNew
            );
        }
        let verifier = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path);
        for (identity, key) in entries {
            assert_eq!(
                verifier
                    .verify(&identity, &key)
                    .expect("every concurrent entry should remain persisted"),
                HostKeyAcceptance::Known
            );
        }
    }

    #[test]
    fn glob_patterns_honor_negation() {
        let patterns = ssh_key::known_hosts::HostPatterns::Patterns(vec![
            "*.example.com".to_owned(),
            "!blocked.example.com".to_owned(),
        ]);
        assert!(known_hosts_match(&patterns, "ok.example.com"));
        assert!(!known_hosts_match(&patterns, "blocked.example.com"));
    }

    #[test]
    fn hashed_hostname_matches_openssh_hmac() {
        let salt = b"01234567890123456789".to_vec();
        let mut mac = HmacSha1::new_from_slice(&salt).expect("valid HMAC key");
        mac.update(b"hashed.example");
        let hash = mac.finalize().into_bytes().into();
        let patterns = ssh_key::known_hosts::HostPatterns::HashedName { salt, hash };

        assert!(known_hosts_match(&patterns, "hashed.example"));
        assert!(!known_hosts_match(&patterns, "other.example"));
    }

    fn hashed_known_hosts_line(host: &str) -> String {
        use base64::Engine as _;
        let salt = b"01234567890123456789".to_vec();
        let mut mac = HmacSha1::new_from_slice(&salt).expect("valid HMAC key");
        mac.update(host.as_bytes());
        let hash = mac.finalize().into_bytes();
        let encode = base64::engine::general_purpose::STANDARD;
        format!("|1|{}|{}", encode.encode(salt), encode.encode(hash))
    }

    #[test]
    fn accepted_host_is_listed_with_algorithm_fingerprint_and_timestamps() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let id = identity("host.example", 22);
        let presented = key(ssh_key::Algorithm::Ed25519);
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path)
            .verify(&id, &presented)
            .expect("accept new");

        let hosts = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
            .list_known_hosts()
            .expect("list known hosts");

        assert_eq!(hosts.len(), 1);
        let host = &hosts[0];
        assert_eq!(host.identity, id);
        assert_eq!(host.algorithm, "ssh-ed25519");
        assert!(host.fingerprint.starts_with("SHA256:"));
        assert_eq!(host.public_key, public_key_string(&presented));
        assert!(host.discovered_at.is_some());
        assert!(host.last_seen.is_some());
    }

    #[test]
    fn reusing_known_host_preserves_metadata() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let id = identity("host.example", 22);
        let presented = key(ssh_key::Algorithm::Ed25519);
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path)
            .verify(&id, &presented)
            .expect("seed app trust");

        let verifier = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path);
        assert_eq!(
            verifier.verify(&id, &presented).expect("known host reuses"),
            HostKeyAcceptance::Known
        );

        let hosts = verifier.list_known_hosts().expect("list known hosts");
        assert_eq!(hosts.len(), 1);
        assert!(hosts[0].discovered_at.is_some());
        assert!(hosts[0].last_seen.is_some());
        assert!(hosts[0].discovered_at <= hosts[0].last_seen);
    }

    #[test]
    fn legacy_trust_store_without_metadata_still_loads() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let id = identity("host.example", 22);
        let presented = key(ssh_key::Algorithm::Ed25519);
        HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path)
            .verify(&id, &presented)
            .expect("seed app trust");
        let mut store: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read seeded trust store"))
                .expect("parse seeded trust store");
        for entry in store["entries"].as_array_mut().expect("entries array") {
            entry
                .as_object_mut()
                .expect("entry object")
                .remove("discovered_at");
            entry
                .as_object_mut()
                .expect("entry object")
                .remove("last_seen");
        }
        fs::write(
            &path,
            serde_json::to_string(&store).expect("serialize legacy store"),
        )
        .expect("write legacy trust store");

        let hosts = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
            .list_known_hosts()
            .expect("legacy store should load");
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].discovered_at, None);
        assert_eq!(hosts[0].last_seen, None);
        assert_eq!(
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
                .verify(&id, &presented)
                .expect("legacy key should remain trusted"),
            HostKeyAcceptance::Known
        );
    }

    #[test]
    fn import_system_known_hosts_imports_plain_and_skips_ambiguous() {
        let temp = TempDir::new().expect("temp dir");
        let app_path = temp.path().join("keys.json");
        let known_hosts_path = temp.path().join("known_hosts");
        let plain_key = key(ssh_key::Algorithm::Ed25519);
        let port_key = key(ssh_key::Algorithm::Ed25519);
        let revoked_key = key(ssh_key::Algorithm::Ed25519);
        let content = format!(
            "plain.example {}\n[port.example]:2200 {}\n*.wild.example {}\n!neg.example {}\n{}\n@revoked revoked.example {}\n@cert-authority ca.example {}\nnot-a-valid-known-hosts-line\n",
            public_key_string(&plain_key),
            public_key_string(&port_key),
            public_key_string(&port_key),
            public_key_string(&plain_key),
            hashed_known_hosts_line("hashed.example"),
            public_key_string(&revoked_key),
            public_key_string(&plain_key),
        );
        fs::write(&known_hosts_path, content).expect("write known_hosts");

        let verifier = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &app_path);
        let imported = verifier
            .import_system_known_hosts(&known_hosts_path)
            .expect("import system known_hosts");

        assert_eq!(imported, 2);
        let hosts = verifier.list_known_hosts().expect("list known hosts");
        let identities = hosts
            .iter()
            .map(|host| host.identity.clone())
            .collect::<Vec<_>>();
        assert!(identities.contains(&identity("plain.example", 22)));
        assert!(identities.contains(&identity("port.example", 2200)));
    }

    #[test]
    fn import_system_known_hosts_is_idempotent() {
        let temp = TempDir::new().expect("temp dir");
        let app_path = temp.path().join("keys.json");
        let known_hosts_path = temp.path().join("known_hosts");
        let presented = key(ssh_key::Algorithm::Ed25519);
        fs::write(
            &known_hosts_path,
            format!("plain.example {}\n", public_key_string(&presented)),
        )
        .expect("write known_hosts");

        let verifier = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &app_path);
        assert_eq!(
            verifier
                .import_system_known_hosts(&known_hosts_path)
                .expect("first import"),
            1
        );
        assert_eq!(
            verifier
                .import_system_known_hosts(&known_hosts_path)
                .expect("second import"),
            0
        );
    }

    #[test]
    fn missing_known_hosts_file_imports_nothing() {
        let temp = TempDir::new().expect("temp dir");
        let verifier =
            HostKeyVerifier::for_store(HostKeyPolicy::Strict, temp.path().join("keys.json"));
        assert_eq!(
            verifier
                .import_system_known_hosts(&temp.path().join("missing_known_hosts"))
                .expect("missing file is not an error"),
            0
        );
    }

    #[test]
    fn list_known_hosts_skips_a_single_corrupt_entry() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("keys.json");
        let good_id = identity("good.example", 22);
        let bad_id = identity("bad.example", 22);
        let verifier = HostKeyVerifier::for_store(HostKeyPolicy::AcceptNew, &path);
        verifier
            .verify(&good_id, &key(ssh_key::Algorithm::Ed25519))
            .expect("seed good host");
        verifier
            .verify(&bad_id, &key(ssh_key::Algorithm::Ed25519))
            .expect("seed bad host");

        let mut store: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read seeded trust store"))
                .expect("parse seeded trust store");
        for entry in store["entries"].as_array_mut().expect("entries array") {
            if entry["host"] == "bad.example" {
                entry["fingerprint"] = serde_json::Value::String("SHA256:corrupt".to_owned());
            }
        }
        fs::write(
            &path,
            serde_json::to_string(&store).expect("serialize store"),
        )
        .expect("rewrite trust store");

        let hosts = HostKeyVerifier::for_store(HostKeyPolicy::Strict, &path)
            .list_known_hosts()
            .expect("listing tolerates a corrupt entry");
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].identity, good_id);
    }
}
