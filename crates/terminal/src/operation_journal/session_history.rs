use super::model::{OperationJournal, OperationJournalSessionId};
use super::persistence::{
    OperationJournalFileStore, OperationJournalPersistenceConfig,
    OperationJournalPersistenceCorruption, OperationJournalPersistenceError,
    OperationJournalPersistencePaths, OperationJournalRecoverySource, open_regular_read_file,
    write_atomic_file,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub const OPERATION_JOURNAL_SESSION_MANIFEST_SCHEMA_VERSION: u16 = 1;

const OPERATION_JOURNAL_FILE_PREFIX: &str = "terminal-operation-journal-";
const OPERATION_JOURNAL_SESSION_MANIFEST_SUFFIX: &str = ".session.json";
const MAX_OPERATION_JOURNAL_CONNECTION_ID_CHARS: usize = 256;
const MAX_OPERATION_JOURNAL_SESSION_ID_CHARS: usize = 256;
const MAX_OPERATION_JOURNAL_DIRECTORY_ENTRIES: u64 = 1_024;
const MAX_OPERATION_JOURNAL_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_OPERATION_JOURNAL_HISTORY_SESSIONS: u64 = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationJournalScopeKind {
    Local,
    Ssh,
    Serial,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct OperationJournalScope {
    kind: OperationJournalScopeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    connection_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationJournalScopeSnapshot {
    kind: OperationJournalScopeKind,
    #[serde(default)]
    connection_id: Option<String>,
}

impl<'de> Deserialize<'de> for OperationJournalScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let snapshot = OperationJournalScopeSnapshot::deserialize(deserializer)?;
        Self::from_parts(snapshot.kind, snapshot.connection_id).map_err(serde::de::Error::custom)
    }
}

impl OperationJournalScope {
    pub fn local() -> Self {
        Self {
            kind: OperationJournalScopeKind::Local,
            connection_id: None,
        }
    }

    pub fn ssh(connection_id: impl Into<String>) -> Result<Self, OperationJournalScopeError> {
        Self::connected(OperationJournalScopeKind::Ssh, connection_id.into())
    }

    pub fn serial(connection_id: impl Into<String>) -> Result<Self, OperationJournalScopeError> {
        Self::connected(OperationJournalScopeKind::Serial, connection_id.into())
    }

    pub fn kind(&self) -> OperationJournalScopeKind {
        self.kind
    }

    pub fn connection_id(&self) -> Option<&str> {
        self.connection_id.as_deref()
    }

    pub fn storage_key(&self) -> String {
        match (&self.kind, &self.connection_id) {
            (OperationJournalScopeKind::Local, None) => "local".to_string(),
            (OperationJournalScopeKind::Ssh, Some(connection_id)) => {
                format!("ssh:{connection_id}")
            }
            (OperationJournalScopeKind::Serial, Some(connection_id)) => {
                format!("serial:{connection_id}")
            }
            _ => unreachable!("operation journal scopes are validated at construction"),
        }
    }

    fn connected(
        kind: OperationJournalScopeKind,
        connection_id: String,
    ) -> Result<Self, OperationJournalScopeError> {
        validate_connection_id(&connection_id)?;
        Ok(Self {
            kind,
            connection_id: Some(connection_id),
        })
    }

    fn from_parts(
        kind: OperationJournalScopeKind,
        connection_id: Option<String>,
    ) -> Result<Self, OperationJournalScopeError> {
        match (kind, connection_id) {
            (OperationJournalScopeKind::Local, None) => Ok(Self::local()),
            (OperationJournalScopeKind::Local, Some(_)) => {
                Err(OperationJournalScopeError::UnexpectedConnectionId)
            }
            (OperationJournalScopeKind::Ssh, Some(connection_id)) => Self::ssh(connection_id),
            (OperationJournalScopeKind::Serial, Some(connection_id)) => Self::serial(connection_id),
            (OperationJournalScopeKind::Ssh | OperationJournalScopeKind::Serial, None) => {
                Err(OperationJournalScopeError::MissingConnectionId)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationJournalScopeError {
    MissingConnectionId,
    UnexpectedConnectionId,
    EmptyConnectionId,
    ConnectionIdTooLong { max_chars: usize },
    ConnectionIdContainsControlCharacter,
}

impl fmt::Display for OperationJournalScopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingConnectionId => formatter.write_str("connection id is required"),
            Self::UnexpectedConnectionId => {
                formatter.write_str("local journal scope cannot contain a connection id")
            }
            Self::EmptyConnectionId => formatter.write_str("connection id is empty"),
            Self::ConnectionIdTooLong { max_chars } => {
                write!(formatter, "connection id exceeds {max_chars} characters")
            }
            Self::ConnectionIdContainsControlCharacter => {
                formatter.write_str("connection id contains a control character")
            }
        }
    }
}

impl std::error::Error for OperationJournalScopeError {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OperationJournalSessionManifest {
    schema_version: u16,
    session_id: OperationJournalSessionId,
    scope: OperationJournalScope,
    created_at_unix_ms: u64,
    updated_at_unix_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationJournalSessionManifestSnapshot {
    schema_version: u16,
    session_id: OperationJournalSessionId,
    scope: OperationJournalScope,
    created_at_unix_ms: u64,
    updated_at_unix_ms: u64,
}

impl<'de> Deserialize<'de> for OperationJournalSessionManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let snapshot = OperationJournalSessionManifestSnapshot::deserialize(deserializer)?;
        let manifest = Self {
            schema_version: snapshot.schema_version,
            session_id: snapshot.session_id,
            scope: snapshot.scope,
            created_at_unix_ms: snapshot.created_at_unix_ms,
            updated_at_unix_ms: snapshot.updated_at_unix_ms,
        };
        manifest.validate().map_err(serde::de::Error::custom)?;
        Ok(manifest)
    }
}

impl OperationJournalSessionManifest {
    pub fn new(
        session_id: OperationJournalSessionId,
        scope: OperationJournalScope,
        created_at_unix_ms: u64,
    ) -> Result<Self, OperationJournalHistoryError> {
        let manifest = Self {
            schema_version: OPERATION_JOURNAL_SESSION_MANIFEST_SCHEMA_VERSION,
            session_id,
            scope,
            created_at_unix_ms,
            updated_at_unix_ms: created_at_unix_ms,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub fn session_id(&self) -> &OperationJournalSessionId {
        &self.session_id
    }

    pub fn scope(&self) -> &OperationJournalScope {
        &self.scope
    }

    pub fn created_at_unix_ms(&self) -> u64 {
        self.created_at_unix_ms
    }

    pub fn updated_at_unix_ms(&self) -> u64 {
        self.updated_at_unix_ms
    }

    pub fn touch(&mut self, updated_at_unix_ms: u64) -> Result<(), OperationJournalHistoryError> {
        if updated_at_unix_ms < self.updated_at_unix_ms {
            return Err(OperationJournalHistoryError::InvalidManifest {
                reason: "manifest timestamp moved backwards",
            });
        }
        self.updated_at_unix_ms = updated_at_unix_ms;
        Ok(())
    }

    fn validate(&self) -> Result<(), OperationJournalHistoryError> {
        if self.schema_version != OPERATION_JOURNAL_SESSION_MANIFEST_SCHEMA_VERSION {
            return Err(OperationJournalHistoryError::InvalidManifest {
                reason: "unsupported manifest schema version",
            });
        }
        validate_session_id(&self.session_id)?;
        if self.updated_at_unix_ms < self.created_at_unix_ms {
            return Err(OperationJournalHistoryError::InvalidManifest {
                reason: "manifest update predates creation",
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationJournalHistoryConfig {
    pub max_directory_entries: u64,
    pub max_manifest_bytes: u64,
    pub max_history_sessions: u64,
    pub persistence: OperationJournalPersistenceConfig,
}

impl Default for OperationJournalHistoryConfig {
    fn default() -> Self {
        Self {
            max_directory_entries: 512,
            max_manifest_bytes: MAX_OPERATION_JOURNAL_MANIFEST_BYTES,
            max_history_sessions: 64,
            persistence: OperationJournalPersistenceConfig::default(),
        }
    }
}

impl OperationJournalHistoryConfig {
    fn validate(&self) -> Result<(), OperationJournalHistoryError> {
        if self.max_directory_entries == 0 {
            return Err(OperationJournalHistoryError::InvalidConfig {
                reason: "max_directory_entries must be greater than zero",
            });
        }
        if self.max_manifest_bytes == 0 {
            return Err(OperationJournalHistoryError::InvalidConfig {
                reason: "max_manifest_bytes must be greater than zero",
            });
        }
        if self.max_history_sessions == 0 {
            return Err(OperationJournalHistoryError::InvalidConfig {
                reason: "max_history_sessions must be greater than zero",
            });
        }
        if self.max_directory_entries > MAX_OPERATION_JOURNAL_DIRECTORY_ENTRIES {
            return Err(OperationJournalHistoryError::InvalidConfig {
                reason: "max_directory_entries exceeds the hard limit",
            });
        }
        if self.max_manifest_bytes > MAX_OPERATION_JOURNAL_MANIFEST_BYTES {
            return Err(OperationJournalHistoryError::InvalidConfig {
                reason: "max_manifest_bytes exceeds the hard limit",
            });
        }
        if self.max_history_sessions > MAX_OPERATION_JOURNAL_HISTORY_SESSIONS {
            return Err(OperationJournalHistoryError::InvalidConfig {
                reason: "max_history_sessions exceeds the hard limit",
            });
        }
        self.persistence.validate().map_err(|error| {
            OperationJournalHistoryError::InvalidPersistenceConfig {
                reason: error.to_string(),
            }
        })
    }
}

/// Discovers operation journal history beneath a trusted application data root.
///
/// Manifest, checkpoint, and append-log leaf files are opened without following
/// symlinks or Windows reparse points. The root and its ancestor directories
/// remain part of the caller's trusted storage boundary.
#[derive(Clone, Debug)]
pub struct OperationJournalHistoryStore {
    root: PathBuf,
    config: OperationJournalHistoryConfig,
}

impl OperationJournalHistoryStore {
    pub fn new(
        root: impl AsRef<Path>,
        config: OperationJournalHistoryConfig,
    ) -> Result<Self, OperationJournalHistoryError> {
        config.validate()?;
        Ok(Self {
            root: root.as_ref().to_path_buf(),
            config,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn paths_for_session(
        &self,
        session_id: &OperationJournalSessionId,
    ) -> OperationJournalPersistencePaths {
        OperationJournalPersistencePaths::for_session(&self.root, session_id)
    }

    pub fn write_manifest(
        &self,
        manifest: &OperationJournalSessionManifest,
    ) -> Result<(), OperationJournalHistoryError> {
        manifest.validate()?;
        let mut serialized = serde_json::to_vec(manifest)
            .map_err(|source| OperationJournalHistoryError::ManifestSerialization { source })?;
        serialized.push(b'\n');
        let actual_bytes = u64::try_from(serialized.len()).map_err(|_| {
            OperationJournalHistoryError::ManifestTooLarge {
                actual_bytes: u64::MAX,
                max_bytes: self.config.max_manifest_bytes,
            }
        })?;
        if actual_bytes > self.config.max_manifest_bytes {
            return Err(OperationJournalHistoryError::ManifestTooLarge {
                actual_bytes,
                max_bytes: self.config.max_manifest_bytes,
            });
        }

        let path = self
            .paths_for_session(manifest.session_id())
            .session_manifest_path()
            .to_path_buf();
        write_atomic_file(&path, &serialized)
            .map_err(|source| OperationJournalHistoryError::ManifestWrite { source })
    }

    pub fn discover(
        &self,
        scope: &OperationJournalScope,
        excluded_session_ids: &[OperationJournalSessionId],
    ) -> OperationJournalHistoryDiscovery {
        let excluded_session_ids = excluded_session_ids.iter().cloned().collect::<HashSet<_>>();
        let mut warnings = Vec::new();
        let mut manifests = self.discover_manifests(scope, &excluded_session_ids, &mut warnings);
        manifests.sort_by(|left, right| {
            right
                .updated_at_unix_ms()
                .cmp(&left.updated_at_unix_ms())
                .then_with(|| left.session_id().as_str().cmp(right.session_id().as_str()))
        });

        let mut seen_session_ids = HashSet::new();
        manifests.retain(|manifest| {
            if seen_session_ids.insert(manifest.session_id().clone()) {
                true
            } else {
                warnings.push(OperationJournalRecoveryWarning::new(
                    OperationJournalRecoveryWarningKind::DuplicateSessionId,
                    Some(manifest.session_id().clone()),
                ));
                false
            }
        });

        let mut histories = Vec::new();
        for (index, manifest) in manifests.iter().enumerate() {
            if histories.len()
                >= usize::try_from(self.config.max_history_sessions).unwrap_or(usize::MAX)
            {
                warnings.push(OperationJournalRecoveryWarning::new(
                    OperationJournalRecoveryWarningKind::HistoryLimitReached,
                    None,
                ));
                break;
            }

            if let Some(history) = self.recover_history(manifest, &mut warnings) {
                histories.push(history);
                if histories.len()
                    >= usize::try_from(self.config.max_history_sessions).unwrap_or(usize::MAX)
                    && index + 1 < manifests.len()
                {
                    warnings.push(OperationJournalRecoveryWarning::new(
                        OperationJournalRecoveryWarningKind::HistoryLimitReached,
                        None,
                    ));
                    break;
                }
            }
        }

        OperationJournalHistoryDiscovery {
            histories,
            warnings,
        }
    }

    fn discover_manifests(
        &self,
        scope: &OperationJournalScope,
        excluded_session_ids: &HashSet<OperationJournalSessionId>,
        warnings: &mut Vec<OperationJournalRecoveryWarning>,
    ) -> Vec<OperationJournalSessionManifest> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(_) => {
                warnings.push(OperationJournalRecoveryWarning::new(
                    OperationJournalRecoveryWarningKind::DirectoryReadFailed,
                    None,
                ));
                return Vec::new();
            }
        };

        let mut directory_entries = Vec::with_capacity(
            usize::try_from(self.config.max_directory_entries).unwrap_or(usize::MAX),
        );
        let mut scanned_entries = 0_u64;
        for entry in entries {
            if scanned_entries >= self.config.max_directory_entries {
                warnings.push(OperationJournalRecoveryWarning::new(
                    OperationJournalRecoveryWarningKind::DirectoryScanLimitReached,
                    None,
                ));
                // Do not recover an arbitrary OS-order-dependent subset. Returning
                // no histories is conservative, deterministic, and keeps the scan
                // bounded to at most one entry beyond the configured limit.
                return Vec::new();
            }
            scanned_entries += 1;

            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    warnings.push(OperationJournalRecoveryWarning::new(
                        OperationJournalRecoveryWarningKind::DirectoryEntryReadFailed,
                        None,
                    ));
                    continue;
                }
            };
            directory_entries.push(entry);
        }
        directory_entries.sort_by_key(|entry| entry.file_name());

        let mut manifests = Vec::new();
        for entry in directory_entries {
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            if !is_manifest_file_name(file_name) {
                continue;
            }

            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    warnings.push(OperationJournalRecoveryWarning::new(
                        OperationJournalRecoveryWarningKind::ManifestReadFailed,
                        None,
                    ));
                    continue;
                }
            };
            if !file_type.is_file() {
                warnings.push(OperationJournalRecoveryWarning::new(
                    OperationJournalRecoveryWarningKind::ManifestNotRegularFile,
                    None,
                ));
                continue;
            }

            let manifest = match self.read_manifest(&entry.path()) {
                Ok(manifest) => manifest,
                Err(ManifestReadFailure::TooLarge) => {
                    warnings.push(OperationJournalRecoveryWarning::new(
                        OperationJournalRecoveryWarningKind::ManifestTooLarge,
                        None,
                    ));
                    continue;
                }
                Err(ManifestReadFailure::Read) => {
                    warnings.push(OperationJournalRecoveryWarning::new(
                        OperationJournalRecoveryWarningKind::ManifestReadFailed,
                        None,
                    ));
                    continue;
                }
                Err(ManifestReadFailure::Invalid) => {
                    warnings.push(OperationJournalRecoveryWarning::new(
                        OperationJournalRecoveryWarningKind::InvalidManifest,
                        None,
                    ));
                    continue;
                }
            };

            let expected_file_name = self
                .paths_for_session(manifest.session_id())
                .session_manifest_path()
                .file_name()
                .and_then(|expected| expected.to_str())
                .map(str::to_owned);
            if expected_file_name.as_deref() != Some(file_name) {
                warnings.push(OperationJournalRecoveryWarning::new(
                    OperationJournalRecoveryWarningKind::InvalidManifest,
                    Some(manifest.session_id().clone()),
                ));
                continue;
            }
            if manifest.scope() != scope || excluded_session_ids.contains(manifest.session_id()) {
                continue;
            }
            manifests.push(manifest);
        }
        manifests
    }

    fn read_manifest(
        &self,
        path: &Path,
    ) -> Result<OperationJournalSessionManifest, ManifestReadFailure> {
        let file = open_regular_read_file(path).map_err(|_| ManifestReadFailure::Read)?;
        let metadata = file.metadata().map_err(|_| ManifestReadFailure::Read)?;
        if !metadata.is_file() {
            return Err(ManifestReadFailure::Read);
        }
        if metadata.len() > self.config.max_manifest_bytes {
            return Err(ManifestReadFailure::TooLarge);
        }

        let read_limit = self.config.max_manifest_bytes.saturating_add(1);
        let mut serialized = Vec::with_capacity(
            usize::try_from(metadata.len())
                .unwrap_or(0)
                .min(usize::try_from(self.config.max_manifest_bytes).unwrap_or(0)),
        );
        file.take(read_limit)
            .read_to_end(&mut serialized)
            .map_err(|_| ManifestReadFailure::Read)?;
        if u64::try_from(serialized.len()).unwrap_or(u64::MAX) > self.config.max_manifest_bytes {
            return Err(ManifestReadFailure::TooLarge);
        }
        serde_json::from_slice(&serialized).map_err(|_| ManifestReadFailure::Invalid)
    }

    fn recover_history(
        &self,
        manifest: &OperationJournalSessionManifest,
        warnings: &mut Vec<OperationJournalRecoveryWarning>,
    ) -> Option<OperationJournalHistorySnapshot> {
        let paths = self.paths_for_session(manifest.session_id());
        let recovery = match OperationJournalFileStore::recover_read_only(
            paths,
            self.config.persistence.clone(),
        ) {
            Ok(recovery) => recovery,
            Err(error) => {
                warnings.push(OperationJournalRecoveryWarning::new(
                    recovery_failure_warning_kind(&error),
                    Some(manifest.session_id().clone()),
                ));
                return None;
            }
        };

        let recovery_source = recovery.source();
        let checkpoint_rejection = recovery.checkpoint_rejection();
        let discarded_log_tail_bytes = recovery.discarded_log_tail_bytes();
        let Some(journal) = recovery.into_journal() else {
            warnings.push(OperationJournalRecoveryWarning::new(
                OperationJournalRecoveryWarningKind::JournalMissing,
                Some(manifest.session_id().clone()),
            ));
            return None;
        };
        if journal.session_id() != manifest.session_id() {
            warnings.push(OperationJournalRecoveryWarning::new(
                OperationJournalRecoveryWarningKind::JournalSessionMismatch,
                Some(manifest.session_id().clone()),
            ));
            return None;
        }
        if checkpoint_rejection.is_some() {
            warnings.push(OperationJournalRecoveryWarning::new(
                OperationJournalRecoveryWarningKind::CheckpointRejected,
                Some(manifest.session_id().clone()),
            ));
        }
        if discarded_log_tail_bytes > 0 {
            warnings.push(OperationJournalRecoveryWarning::new(
                OperationJournalRecoveryWarningKind::TruncatedLogTailRecovered,
                Some(manifest.session_id().clone()),
            ));
        }

        Some(OperationJournalHistorySnapshot {
            manifest: manifest.clone(),
            journal,
            recovery_source,
            checkpoint_rejection,
            discarded_log_tail_bytes,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationJournalHistorySnapshot {
    manifest: OperationJournalSessionManifest,
    journal: OperationJournal,
    recovery_source: Option<OperationJournalRecoverySource>,
    checkpoint_rejection: Option<OperationJournalPersistenceCorruption>,
    discarded_log_tail_bytes: u64,
}

impl OperationJournalHistorySnapshot {
    pub fn manifest(&self) -> &OperationJournalSessionManifest {
        &self.manifest
    }

    pub fn session_id(&self) -> &OperationJournalSessionId {
        self.manifest.session_id()
    }

    pub fn scope(&self) -> &OperationJournalScope {
        self.manifest.scope()
    }

    pub fn journal(&self) -> &OperationJournal {
        &self.journal
    }

    pub fn recovery_source(&self) -> Option<OperationJournalRecoverySource> {
        self.recovery_source
    }

    pub fn checkpoint_rejection(&self) -> Option<OperationJournalPersistenceCorruption> {
        self.checkpoint_rejection
    }

    pub fn discarded_log_tail_bytes(&self) -> u64 {
        self.discarded_log_tail_bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationJournalHistoryDiscovery {
    histories: Vec<OperationJournalHistorySnapshot>,
    warnings: Vec<OperationJournalRecoveryWarning>,
}

impl OperationJournalHistoryDiscovery {
    pub fn histories(&self) -> &[OperationJournalHistorySnapshot] {
        &self.histories
    }

    pub fn into_histories(self) -> Vec<OperationJournalHistorySnapshot> {
        self.histories
    }

    pub fn warnings(&self) -> &[OperationJournalRecoveryWarning] {
        &self.warnings
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationJournalRecoveryWarningKind {
    DirectoryReadFailed,
    DirectoryEntryReadFailed,
    DirectoryScanLimitReached,
    ManifestNotRegularFile,
    ManifestTooLarge,
    ManifestReadFailed,
    InvalidManifest,
    DuplicateSessionId,
    JournalMissing,
    JournalRecoveryFailed,
    JournalSessionMismatch,
    CheckpointRejected,
    TruncatedLogTailRecovered,
    HistoryLimitReached,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationJournalRecoveryWarning {
    kind: OperationJournalRecoveryWarningKind,
    session_id: Option<OperationJournalSessionId>,
}

impl OperationJournalRecoveryWarning {
    fn new(
        kind: OperationJournalRecoveryWarningKind,
        session_id: Option<OperationJournalSessionId>,
    ) -> Self {
        Self { kind, session_id }
    }

    pub fn kind(&self) -> OperationJournalRecoveryWarningKind {
        self.kind
    }

    pub fn session_id(&self) -> Option<&OperationJournalSessionId> {
        self.session_id.as_ref()
    }
}

#[derive(Debug)]
pub enum OperationJournalHistoryError {
    InvalidConfig { reason: &'static str },
    InvalidPersistenceConfig { reason: String },
    InvalidManifest { reason: &'static str },
    ManifestTooLarge { actual_bytes: u64, max_bytes: u64 },
    ManifestSerialization { source: serde_json::Error },
    ManifestWrite { source: io::Error },
}

impl fmt::Display for OperationJournalHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig { reason } => {
                write!(
                    formatter,
                    "invalid operation journal history config: {reason}"
                )
            }
            Self::InvalidPersistenceConfig { reason } => {
                write!(
                    formatter,
                    "invalid operation journal persistence config: {reason}"
                )
            }
            Self::InvalidManifest { reason } => {
                write!(
                    formatter,
                    "invalid operation journal session manifest: {reason}"
                )
            }
            Self::ManifestTooLarge {
                actual_bytes,
                max_bytes,
            } => write!(
                formatter,
                "operation journal session manifest is {actual_bytes} bytes, above the {max_bytes}-byte limit"
            ),
            Self::ManifestSerialization { source } => {
                write!(
                    formatter,
                    "failed to serialize operation journal session manifest: {source}"
                )
            }
            Self::ManifestWrite { source } => {
                write!(
                    formatter,
                    "failed to write operation journal session manifest: {source}"
                )
            }
        }
    }
}

impl std::error::Error for OperationJournalHistoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ManifestSerialization { source } => Some(source),
            Self::ManifestWrite { source } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManifestReadFailure {
    TooLarge,
    Read,
    Invalid,
}

fn validate_connection_id(connection_id: &str) -> Result<(), OperationJournalScopeError> {
    if connection_id.is_empty() {
        return Err(OperationJournalScopeError::EmptyConnectionId);
    }
    if connection_id.chars().count() > MAX_OPERATION_JOURNAL_CONNECTION_ID_CHARS {
        return Err(OperationJournalScopeError::ConnectionIdTooLong {
            max_chars: MAX_OPERATION_JOURNAL_CONNECTION_ID_CHARS,
        });
    }
    if connection_id.chars().any(char::is_control) {
        return Err(OperationJournalScopeError::ConnectionIdContainsControlCharacter);
    }
    Ok(())
}

fn validate_session_id(
    session_id: &OperationJournalSessionId,
) -> Result<(), OperationJournalHistoryError> {
    if session_id.as_str().is_empty() {
        return Err(OperationJournalHistoryError::InvalidManifest {
            reason: "session id is empty",
        });
    }
    if session_id.as_str().chars().count() > MAX_OPERATION_JOURNAL_SESSION_ID_CHARS {
        return Err(OperationJournalHistoryError::InvalidManifest {
            reason: "session id is too long",
        });
    }
    if session_id.as_str().chars().any(char::is_control) {
        return Err(OperationJournalHistoryError::InvalidManifest {
            reason: "session id contains a control character",
        });
    }
    Ok(())
}

fn is_manifest_file_name(file_name: &str) -> bool {
    let Some(digest) = file_name
        .strip_prefix(OPERATION_JOURNAL_FILE_PREFIX)
        .and_then(|name| name.strip_suffix(OPERATION_JOURNAL_SESSION_MANIFEST_SUFFIX))
    else {
        return false;
    };
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn recovery_failure_warning_kind(
    error: &OperationJournalPersistenceError,
) -> OperationJournalRecoveryWarningKind {
    match error {
        OperationJournalPersistenceError::SessionMismatch
        | OperationJournalPersistenceError::CorruptLogEntry {
            corruption: OperationJournalPersistenceCorruption::SessionMismatch,
            ..
        }
        | OperationJournalPersistenceError::UnrecoverableCheckpoint {
            corruption: OperationJournalPersistenceCorruption::SessionMismatch,
        } => OperationJournalRecoveryWarningKind::JournalSessionMismatch,
        _ => OperationJournalRecoveryWarningKind::JournalRecoveryFailed,
    }
}

impl From<OperationJournalPersistenceError> for OperationJournalHistoryError {
    fn from(error: OperationJournalPersistenceError) -> Self {
        Self::InvalidPersistenceConfig {
            reason: error.to_string(),
        }
    }
}
