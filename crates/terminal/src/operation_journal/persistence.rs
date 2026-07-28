use super::model::{OperationJournal, OperationJournalSessionId};
use same_file::Handle as FileHandle;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

pub const OPERATION_JOURNAL_PERSISTENCE_SCHEMA_VERSION: u16 = 1;

const OPERATION_JOURNAL_SNAPSHOT_FORMAT: &str = "navop_terminal_operation_journal_snapshot";
const MAX_OPERATION_JOURNAL_LOG_ENTRIES: u64 = 256;
const MAX_OPERATION_JOURNAL_ENTRY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_OPERATION_JOURNAL_LOG_BYTES: u64 = 32 * 1024 * 1024;
const MAX_OPERATION_JOURNAL_CHECKPOINT_BYTES: u64 = 4 * 1024 * 1024;

#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationJournalPersistenceConfig {
    pub max_log_entries: u64,
    pub max_entry_bytes: u64,
    pub max_log_bytes: u64,
    pub max_checkpoint_bytes: u64,
}

impl Default for OperationJournalPersistenceConfig {
    fn default() -> Self {
        Self {
            max_log_entries: MAX_OPERATION_JOURNAL_LOG_ENTRIES,
            max_entry_bytes: MAX_OPERATION_JOURNAL_ENTRY_BYTES,
            max_log_bytes: MAX_OPERATION_JOURNAL_LOG_BYTES,
            max_checkpoint_bytes: MAX_OPERATION_JOURNAL_CHECKPOINT_BYTES,
        }
    }
}

impl OperationJournalPersistenceConfig {
    pub(super) fn validate(&self) -> Result<(), OperationJournalPersistenceError> {
        if self.max_log_entries == 0 {
            return Err(OperationJournalPersistenceError::InvalidConfig {
                reason: "max_log_entries must be greater than zero",
            });
        }
        if self.max_entry_bytes == 0 {
            return Err(OperationJournalPersistenceError::InvalidConfig {
                reason: "max_entry_bytes must be greater than zero",
            });
        }
        if self.max_log_entries > MAX_OPERATION_JOURNAL_LOG_ENTRIES {
            return Err(OperationJournalPersistenceError::InvalidConfig {
                reason: "max_log_entries exceeds the hard limit",
            });
        }
        if self.max_entry_bytes > MAX_OPERATION_JOURNAL_ENTRY_BYTES {
            return Err(OperationJournalPersistenceError::InvalidConfig {
                reason: "max_entry_bytes exceeds the hard limit",
            });
        }
        if self.max_log_bytes > MAX_OPERATION_JOURNAL_LOG_BYTES {
            return Err(OperationJournalPersistenceError::InvalidConfig {
                reason: "max_log_bytes exceeds the hard limit",
            });
        }
        if self.max_checkpoint_bytes > MAX_OPERATION_JOURNAL_CHECKPOINT_BYTES {
            return Err(OperationJournalPersistenceError::InvalidConfig {
                reason: "max_checkpoint_bytes exceeds the hard limit",
            });
        }
        if self.max_log_bytes < self.max_entry_bytes {
            return Err(OperationJournalPersistenceError::InvalidConfig {
                reason: "max_log_bytes must fit at least one entry",
            });
        }
        if self.max_checkpoint_bytes < self.max_entry_bytes {
            return Err(OperationJournalPersistenceError::InvalidConfig {
                reason: "max_checkpoint_bytes must fit at least one entry",
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationJournalPersistencePaths {
    session_id: OperationJournalSessionId,
    append_log_path: PathBuf,
    checkpoint_path: PathBuf,
    session_manifest_path: PathBuf,
}

impl OperationJournalPersistencePaths {
    pub fn for_session(root: impl AsRef<Path>, session_id: &OperationJournalSessionId) -> Self {
        let digest = Sha256::digest(session_id.as_str().as_bytes());
        let digest = encode_hex(&digest);
        let file_stem = format!("terminal-operation-journal-{digest}");
        Self {
            session_id: session_id.clone(),
            append_log_path: root.as_ref().join(format!("{file_stem}.journal.partial")),
            checkpoint_path: root.as_ref().join(format!("{file_stem}.checkpoint.json")),
            session_manifest_path: root.as_ref().join(format!("{file_stem}.session.json")),
        }
    }

    pub fn session_id(&self) -> &OperationJournalSessionId {
        &self.session_id
    }

    pub fn append_log_path(&self) -> &Path {
        &self.append_log_path
    }

    pub fn checkpoint_path(&self) -> &Path {
        &self.checkpoint_path
    }

    pub fn session_manifest_path(&self) -> &Path {
        &self.session_manifest_path
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationJournalPersistOutcome {
    Appended,
    Compacted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationJournalRecoverySource {
    AppendLog,
    Checkpoint,
    CheckpointAndAppendLog,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationJournalRecovery {
    journal: Option<OperationJournal>,
    source: Option<OperationJournalRecoverySource>,
    checkpoint_rejection: Option<OperationJournalPersistenceCorruption>,
    discarded_log_tail_bytes: u64,
}

impl OperationJournalRecovery {
    pub fn journal(&self) -> Option<&OperationJournal> {
        self.journal.as_ref()
    }

    pub fn into_journal(self) -> Option<OperationJournal> {
        self.journal
    }

    pub fn source(&self) -> Option<OperationJournalRecoverySource> {
        self.source
    }

    pub fn checkpoint_was_rejected(&self) -> bool {
        self.checkpoint_rejection.is_some()
    }

    pub fn checkpoint_rejection(&self) -> Option<OperationJournalPersistenceCorruption> {
        self.checkpoint_rejection
    }

    pub fn discarded_log_tail_bytes(&self) -> u64 {
        self.discarded_log_tail_bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationJournalPersistenceCorruption {
    InvalidRecord,
    ChecksumMismatch,
    UnsupportedFormat,
    UnsupportedSchemaVersion,
    SessionMismatch,
    InvalidSequence,
    ConflictingSnapshot,
    RecordTooLarge,
    Unreadable,
    FileChanged,
}

#[derive(Debug)]
pub enum OperationJournalPersistenceError {
    InvalidConfig {
        reason: &'static str,
    },
    InvalidJournal {
        reason: String,
    },
    SessionMismatch,
    EntryTooLarge {
        actual_bytes: u64,
        max_bytes: u64,
    },
    LogTooLarge {
        actual_bytes: u64,
        max_bytes: u64,
    },
    TooManyLogEntries {
        actual_entries: u64,
        max_entries: u64,
    },
    CorruptLogEntry {
        line_number: u64,
        corruption: OperationJournalPersistenceCorruption,
    },
    UnrecoverableCheckpoint {
        corruption: OperationJournalPersistenceCorruption,
    },
    PersistenceSequenceOverflow,
    AppendLogChanged,
    WriteDisabledAfterFailure,
    OpenLog {
        source: io::Error,
    },
    ReadLog {
        source: io::Error,
    },
    RepairLog {
        source: io::Error,
    },
    AppendWrite {
        source: io::Error,
    },
    LogCompactionWrite {
        source: io::Error,
    },
    CheckpointWrite {
        source: io::Error,
    },
    Serialization {
        source: serde_json::Error,
    },
}

impl fmt::Display for OperationJournalPersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig { reason } => {
                write!(
                    formatter,
                    "invalid operation journal persistence config: {reason}"
                )
            }
            Self::InvalidJournal { reason } => {
                write!(formatter, "invalid operation journal snapshot: {reason}")
            }
            Self::SessionMismatch => {
                formatter.write_str("operation journal session does not match its storage path")
            }
            Self::EntryTooLarge {
                actual_bytes,
                max_bytes,
            } => write!(
                formatter,
                "operation journal snapshot is too large ({actual_bytes} bytes, max {max_bytes})"
            ),
            Self::LogTooLarge {
                actual_bytes,
                max_bytes,
            } => write!(
                formatter,
                "operation journal append log is too large ({actual_bytes} bytes, max {max_bytes})"
            ),
            Self::TooManyLogEntries {
                actual_entries,
                max_entries,
            } => write!(
                formatter,
                "operation journal append log has too many entries ({actual_entries}, max {max_entries})"
            ),
            Self::CorruptLogEntry {
                line_number,
                corruption,
            } => write!(
                formatter,
                "operation journal append log entry {line_number} is corrupt: {corruption:?}"
            ),
            Self::UnrecoverableCheckpoint { corruption } => write!(
                formatter,
                "operation journal checkpoint was rejected without a recoverable append log: {corruption:?}"
            ),
            Self::PersistenceSequenceOverflow => {
                formatter.write_str("operation journal persistence sequence overflowed")
            }
            Self::AppendLogChanged => {
                formatter.write_str("operation journal append log changed outside this store")
            }
            Self::WriteDisabledAfterFailure => formatter
                .write_str("operation journal writes are disabled until the store is reopened"),
            Self::OpenLog { source } => {
                write!(
                    formatter,
                    "failed to open operation journal append log: {source}"
                )
            }
            Self::ReadLog { source } => {
                write!(
                    formatter,
                    "failed to read operation journal append log: {source}"
                )
            }
            Self::RepairLog { source } => {
                write!(
                    formatter,
                    "failed to repair operation journal append log: {source}"
                )
            }
            Self::AppendWrite { source } => {
                write!(
                    formatter,
                    "failed to append operation journal snapshot: {source}"
                )
            }
            Self::LogCompactionWrite { source } => {
                write!(
                    formatter,
                    "failed to compact operation journal append log: {source}"
                )
            }
            Self::CheckpointWrite { source } => {
                write!(
                    formatter,
                    "failed to publish operation journal checkpoint: {source}"
                )
            }
            Self::Serialization { source } => {
                write!(
                    formatter,
                    "failed to serialize operation journal snapshot: {source}"
                )
            }
        }
    }
}

impl std::error::Error for OperationJournalPersistenceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OpenLog { source }
            | Self::ReadLog { source }
            | Self::RepairLog { source }
            | Self::AppendWrite { source }
            | Self::LogCompactionWrite { source }
            | Self::CheckpointWrite { source } => Some(source),
            Self::Serialization { source } => Some(source),
            _ => None,
        }
    }
}

/// A single-writer store for one session's journal paths.
///
/// The caller must not keep multiple live stores for the same paths. Supporting
/// concurrent writers would require an inter-process lock rather than metadata
/// checks around append operations.
///
/// The configured paths and their ancestor directories are trusted,
/// application-owned storage. Leaf journal files are opened without following
/// symlinks or Windows reparse points, but this store does not independently
/// validate every ancestor directory component.
#[derive(Debug)]
pub struct OperationJournalFileStore {
    paths: OperationJournalPersistencePaths,
    config: OperationJournalPersistenceConfig,
    log_file: Option<FileHandle>,
    next_persistence_sequence: u64,
    log_entry_count: u64,
    log_bytes: u64,
    force_compaction: bool,
    write_failed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TruncatedLogTailHandling {
    RepairForWriter,
    PreserveForReadOnlyRecovery,
}

struct RecoveredOperationJournalState {
    recovery: OperationJournalRecovery,
    log_file: Option<FileHandle>,
    next_persistence_sequence: u64,
    log_entry_count: u64,
    log_bytes: u64,
    force_compaction: bool,
}

impl OperationJournalFileStore {
    pub fn open(
        paths: OperationJournalPersistencePaths,
        config: OperationJournalPersistenceConfig,
    ) -> Result<(Self, OperationJournalRecovery), OperationJournalPersistenceError> {
        config.validate()?;
        let RecoveredOperationJournalState {
            recovery,
            log_file,
            next_persistence_sequence,
            log_entry_count,
            log_bytes,
            force_compaction,
        } = recover_operation_journal(&paths, &config, TruncatedLogTailHandling::RepairForWriter)?;
        let store = Self {
            paths,
            config,
            log_file,
            next_persistence_sequence,
            log_entry_count,
            log_bytes,
            force_compaction,
            write_failed: false,
        };
        Ok((store, recovery))
    }

    pub(super) fn recover_read_only(
        paths: OperationJournalPersistencePaths,
        config: OperationJournalPersistenceConfig,
    ) -> Result<OperationJournalRecovery, OperationJournalPersistenceError> {
        config.validate()?;
        Ok(recover_operation_journal(
            &paths,
            &config,
            TruncatedLogTailHandling::PreserveForReadOnlyRecovery,
        )?
        .recovery)
    }

    pub fn persist(
        &mut self,
        journal: &OperationJournal,
    ) -> Result<OperationJournalPersistOutcome, OperationJournalPersistenceError> {
        // A write can become visible before a later durability barrier or checkpoint
        // publication fails. Such failures are deliberately ambiguous: the store is
        // disabled and callers must reopen it to recover the authoritative snapshot
        // instead of assuming that an error means no data reached disk.
        if self.write_failed {
            return Err(OperationJournalPersistenceError::WriteDisabledAfterFailure);
        }
        journal
            .validate()
            .map_err(|error| OperationJournalPersistenceError::InvalidJournal {
                reason: error.to_string(),
            })?;
        if journal.session_id() != &self.paths.session_id {
            return Err(OperationJournalPersistenceError::SessionMismatch);
        }

        let sequence = self.next_persistence_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(OperationJournalPersistenceError::PersistenceSequenceOverflow)?;
        let serialized = serialize_snapshot(journal, sequence, self.config.max_entry_bytes)?;
        let serialized_bytes = u64::try_from(serialized.len()).map_err(|_| {
            OperationJournalPersistenceError::EntryTooLarge {
                actual_bytes: u64::MAX,
                max_bytes: self.config.max_entry_bytes,
            }
        })?;
        if serialized_bytes > self.config.max_entry_bytes {
            return Err(OperationJournalPersistenceError::EntryTooLarge {
                actual_bytes: serialized_bytes,
                max_bytes: self.config.max_entry_bytes,
            });
        }

        self.ensure_log_unchanged()?;
        let next_log_entries = self.log_entry_count.checked_add(1).ok_or(
            OperationJournalPersistenceError::TooManyLogEntries {
                actual_entries: u64::MAX,
                max_entries: self.config.max_log_entries,
            },
        )?;
        let next_log_bytes = self.log_bytes.checked_add(serialized_bytes).ok_or(
            OperationJournalPersistenceError::LogTooLarge {
                actual_bytes: u64::MAX,
                max_bytes: self.config.max_log_bytes,
            },
        )?;
        let must_compact = self.force_compaction
            || next_log_entries > self.config.max_log_entries
            || next_log_bytes > self.config.max_log_bytes;

        if must_compact {
            if let Err(error) = self.compact_log(&serialized, serialized_bytes, next_sequence) {
                self.write_failed = true;
                return Err(error);
            }
            if let Err(source) = write_atomic_file(self.paths.checkpoint_path(), &serialized) {
                self.write_failed = true;
                return Err(OperationJournalPersistenceError::CheckpointWrite { source });
            }
            Ok(OperationJournalPersistOutcome::Compacted)
        } else {
            if let Err(error) =
                self.append_log(&serialized, next_log_entries, next_log_bytes, next_sequence)
            {
                self.write_failed = true;
                return Err(error);
            }
            Ok(OperationJournalPersistOutcome::Appended)
        }
    }

    pub fn log_entry_count(&self) -> u64 {
        self.log_entry_count
    }

    pub fn log_bytes(&self) -> u64 {
        self.log_bytes
    }

    fn ensure_log_unchanged(&self) -> Result<(), OperationJournalPersistenceError> {
        let Some(expected) = self.log_file.as_ref() else {
            return match open_regular_file(
                self.paths.append_log_path(),
                RegularFileOpenMode::ReadOnly,
            ) {
                Err(error) if error.kind() == io::ErrorKind::NotFound && self.log_bytes == 0 => {
                    Ok(())
                }
                Ok(_) | Err(_) => Err(OperationJournalPersistenceError::AppendLogChanged),
            };
        };

        let current =
            open_regular_file_handle(self.paths.append_log_path(), RegularFileOpenMode::ReadOnly)
                .map_err(|_| OperationJournalPersistenceError::AppendLogChanged)?;
        let current_bytes = current
            .as_file()
            .metadata()
            .map_err(|_| OperationJournalPersistenceError::AppendLogChanged)?
            .len();
        if &current == expected && current_bytes == self.log_bytes {
            Ok(())
        } else {
            Err(OperationJournalPersistenceError::AppendLogChanged)
        }
    }

    fn append_log(
        &mut self,
        serialized: &[u8],
        next_log_entries: u64,
        next_log_bytes: u64,
        next_sequence: u64,
    ) -> Result<(), OperationJournalPersistenceError> {
        let parent = persistence_parent(self.paths.append_log_path())
            .map_err(|source| OperationJournalPersistenceError::AppendWrite { source })?;
        fs::create_dir_all(parent)
            .map_err(|source| OperationJournalPersistenceError::AppendWrite { source })?;
        let existed = self.log_file.is_some();
        let mut candidate = if let Some(expected) = self.log_file.as_ref() {
            let candidate =
                open_regular_file_handle(self.paths.append_log_path(), RegularFileOpenMode::Append)
                    .map_err(|_| OperationJournalPersistenceError::AppendLogChanged)?;
            let candidate_bytes = candidate
                .as_file()
                .metadata()
                .map_err(|_| OperationJournalPersistenceError::AppendLogChanged)?
                .len();
            if &candidate != expected || candidate_bytes != self.log_bytes {
                return Err(OperationJournalPersistenceError::AppendLogChanged);
            }
            candidate
        } else {
            let file = match open_regular_file(
                self.paths.append_log_path(),
                RegularFileOpenMode::CreateNewAppend,
            ) {
                Ok(file) => file,
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                    return Err(OperationJournalPersistenceError::AppendLogChanged);
                }
                Err(source) => {
                    return Err(OperationJournalPersistenceError::AppendWrite { source });
                }
            };
            let candidate = FileHandle::from_file(file)
                .map_err(|source| OperationJournalPersistenceError::AppendWrite { source })?;
            if candidate
                .as_file()
                .metadata()
                .map_err(|source| OperationJournalPersistenceError::AppendWrite { source })?
                .len()
                != self.log_bytes
            {
                return Err(OperationJournalPersistenceError::AppendLogChanged);
            }
            candidate
        };

        let result = (|| -> io::Result<()> {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                candidate
                    .as_file()
                    .set_permissions(fs::Permissions::from_mode(0o600))?;
            }
            let file = candidate.as_file_mut();
            file.write_all(serialized)?;
            file.flush()?;
            file.sync_data()?;
            Ok(())
        })();
        if let Err(source) = result {
            self.write_failed = true;
            return Err(OperationJournalPersistenceError::AppendWrite { source });
        }

        let current =
            open_regular_file_handle(self.paths.append_log_path(), RegularFileOpenMode::ReadOnly)
                .map_err(|_| OperationJournalPersistenceError::AppendLogChanged)?;
        let current_bytes = current
            .as_file()
            .metadata()
            .map_err(|_| OperationJournalPersistenceError::AppendLogChanged)?
            .len();
        if current != candidate || current_bytes != next_log_bytes {
            return Err(OperationJournalPersistenceError::AppendLogChanged);
        }

        // The appended bytes are already visible and file data has been synced. Keep
        // the in-memory metadata aligned with that file before attempting the parent
        // directory durability barrier; a later failure cannot safely roll them back.
        self.log_file = Some(candidate);
        self.log_entry_count = next_log_entries;
        self.log_bytes = next_log_bytes;
        self.next_persistence_sequence = next_sequence;
        if !existed {
            if let Err(source) = sync_parent_directory(parent) {
                self.write_failed = true;
                return Err(OperationJournalPersistenceError::AppendWrite { source });
            }
        }
        Ok(())
    }

    fn compact_log(
        &mut self,
        serialized: &[u8],
        serialized_bytes: u64,
        next_sequence: u64,
    ) -> Result<(), OperationJournalPersistenceError> {
        let parent = persistence_parent(self.paths.append_log_path())
            .map_err(|source| OperationJournalPersistenceError::LogCompactionWrite { source })?;
        fs::create_dir_all(parent)
            .map_err(|source| OperationJournalPersistenceError::LogCompactionWrite { source })?;
        let temporary = prepare_atomic_file(parent, serialized)
            .map_err(|source| OperationJournalPersistenceError::LogCompactionWrite { source })?;
        let file = temporary
            .persist(self.paths.append_log_path())
            .map_err(
                |error| OperationJournalPersistenceError::LogCompactionWrite {
                    source: error.error,
                },
            )?;
        let log_file = FileHandle::from_file(file)
            .map_err(|source| OperationJournalPersistenceError::LogCompactionWrite { source })?;
        let current =
            open_regular_file_handle(self.paths.append_log_path(), RegularFileOpenMode::ReadOnly)
                .map_err(|_| OperationJournalPersistenceError::AppendLogChanged)?;
        let current_bytes = current
            .as_file()
            .metadata()
            .map_err(|_| OperationJournalPersistenceError::AppendLogChanged)?
            .len();
        if current != log_file || current_bytes != serialized_bytes {
            return Err(OperationJournalPersistenceError::AppendLogChanged);
        }

        // Atomic replacement has already published the new segment. Record the
        // visible file state before syncing its parent; on sync failure the store is
        // fail-closed and recovery on reopen decides which snapshot is authoritative.
        self.log_file = Some(log_file);
        self.log_entry_count = 1;
        self.log_bytes = serialized_bytes;
        self.next_persistence_sequence = next_sequence;
        self.force_compaction = false;
        if let Err(source) = sync_parent_directory(parent) {
            self.write_failed = true;
            return Err(OperationJournalPersistenceError::LogCompactionWrite { source });
        }
        Ok(())
    }
}

fn recover_operation_journal(
    paths: &OperationJournalPersistencePaths,
    config: &OperationJournalPersistenceConfig,
    truncated_tail_handling: TruncatedLogTailHandling,
) -> Result<RecoveredOperationJournalState, OperationJournalPersistenceError> {
    let checkpoint = read_checkpoint(paths, config);
    let log = read_append_log(paths, config, truncated_tail_handling)?;
    let mut checkpoint_rejection = None;
    let checkpoint = match checkpoint {
        CheckpointRead::Missing => None,
        CheckpointRead::Valid(snapshot) => Some(snapshot),
        CheckpointRead::Rejected(corruption) => {
            checkpoint_rejection = Some(corruption);
            None
        }
    };

    if let (Some(corruption), None) = (checkpoint_rejection, log.latest.as_ref()) {
        return Err(OperationJournalPersistenceError::UnrecoverableCheckpoint { corruption });
    }

    let log_sequence = log
        .latest
        .as_ref()
        .map(|snapshot| snapshot.persistence_sequence);
    let (latest, source) = match (checkpoint, log.latest) {
        (None, None) => (None, None),
        (None, Some(log)) => (Some(log), Some(OperationJournalRecoverySource::AppendLog)),
        (Some(checkpoint), None) => (
            Some(checkpoint),
            Some(OperationJournalRecoverySource::Checkpoint),
        ),
        (Some(checkpoint), Some(log)) => {
            if checkpoint.persistence_sequence == log.persistence_sequence && checkpoint != log {
                checkpoint_rejection =
                    Some(OperationJournalPersistenceCorruption::ConflictingSnapshot);
                (Some(log), Some(OperationJournalRecoverySource::AppendLog))
            } else if checkpoint.persistence_sequence > log.persistence_sequence {
                (
                    Some(checkpoint),
                    Some(OperationJournalRecoverySource::CheckpointAndAppendLog),
                )
            } else {
                (
                    Some(log),
                    Some(OperationJournalRecoverySource::CheckpointAndAppendLog),
                )
            }
        }
    };

    let latest_sequence = latest
        .as_ref()
        .map_or(0, |snapshot| snapshot.persistence_sequence);
    let next_persistence_sequence = latest_sequence
        .checked_add(1)
        .ok_or(OperationJournalPersistenceError::PersistenceSequenceOverflow)?;
    let force_compaction = log_sequence.is_some_and(|log_sequence| log_sequence < latest_sequence);
    let journal = latest.map(|snapshot| snapshot.journal);
    let recovery = OperationJournalRecovery {
        journal,
        source,
        checkpoint_rejection,
        discarded_log_tail_bytes: log.discarded_tail_bytes,
    };
    Ok(RecoveredOperationJournalState {
        recovery,
        log_file: log.file,
        next_persistence_sequence,
        log_entry_count: log.entry_count,
        log_bytes: log.valid_bytes,
        force_compaction,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedOperationJournalSnapshot {
    format: String,
    persistence_schema_version: u16,
    persistence_sequence: u64,
    journal: OperationJournal,
    checksum_sha256: String,
}

#[derive(Serialize)]
struct OperationJournalChecksumInput<'a> {
    format: &'a str,
    persistence_schema_version: u16,
    persistence_sequence: u64,
    journal: &'a OperationJournal,
}

#[derive(Serialize)]
struct PersistedOperationJournalSnapshotRef<'a> {
    format: &'a str,
    persistence_schema_version: u16,
    persistence_sequence: u64,
    journal: &'a OperationJournal,
    checksum_sha256: &'a str,
}

fn serialize_snapshot(
    journal: &OperationJournal,
    persistence_sequence: u64,
    max_bytes: u64,
) -> Result<Vec<u8>, OperationJournalPersistenceError> {
    let checksum_sha256 = snapshot_checksum(journal, persistence_sequence)
        .map_err(|source| OperationJournalPersistenceError::Serialization { source })?;
    let snapshot = PersistedOperationJournalSnapshotRef {
        format: OPERATION_JOURNAL_SNAPSHOT_FORMAT,
        persistence_schema_version: OPERATION_JOURNAL_PERSISTENCE_SCHEMA_VERSION,
        persistence_sequence,
        journal,
        checksum_sha256: &checksum_sha256,
    };

    let mut counter = CountingWriter::default();
    serde_json::to_writer(&mut counter, &snapshot)
        .map_err(|source| OperationJournalPersistenceError::Serialization { source })?;
    let serialized_bytes =
        counter
            .len()
            .checked_add(1)
            .ok_or(OperationJournalPersistenceError::EntryTooLarge {
                actual_bytes: u64::MAX,
                max_bytes,
            })?;
    if serialized_bytes > max_bytes {
        return Err(OperationJournalPersistenceError::EntryTooLarge {
            actual_bytes: serialized_bytes,
            max_bytes,
        });
    }

    let max_bytes_usize = usize::try_from(max_bytes).map_err(|_| {
        OperationJournalPersistenceError::EntryTooLarge {
            actual_bytes: serialized_bytes,
            max_bytes,
        }
    })?;
    let capacity = usize::try_from(serialized_bytes).map_err(|_| {
        OperationJournalPersistenceError::EntryTooLarge {
            actual_bytes: serialized_bytes,
            max_bytes,
        }
    })?;
    let mut writer = BoundedVecWriter::new(max_bytes_usize, capacity);
    serde_json::to_writer(&mut writer, &snapshot)
        .map_err(|source| OperationJournalPersistenceError::Serialization { source })?;
    writer
        .write_all(b"\n")
        .map_err(|source| OperationJournalPersistenceError::Serialization {
            source: serde_json::Error::io(source),
        })?;
    debug_assert_eq!(u64::try_from(writer.len()).ok(), Some(serialized_bytes));
    Ok(writer.into_inner())
}

fn snapshot_checksum(
    journal: &OperationJournal,
    persistence_sequence: u64,
) -> Result<String, serde_json::Error> {
    let input = OperationJournalChecksumInput {
        format: OPERATION_JOURNAL_SNAPSHOT_FORMAT,
        persistence_schema_version: OPERATION_JOURNAL_PERSISTENCE_SCHEMA_VERSION,
        persistence_sequence,
        journal,
    };
    let mut writer = Sha256Writer::default();
    serde_json::to_writer(&mut writer, &input)?;
    let digest = writer.finalize();
    Ok(encode_hex(digest.as_ref()))
}

#[derive(Default)]
struct CountingWriter {
    len: u64,
}

impl CountingWriter {
    fn len(&self) -> u64 {
        self.len
    }
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let bytes_len = u64::try_from(bytes.len())
            .map_err(|_| io::Error::other("serialized operation journal length overflow"))?;
        self.len = self
            .len
            .checked_add(bytes_len)
            .ok_or_else(|| io::Error::other("serialized operation journal length overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct Sha256Writer {
    digest: Sha256,
}

impl Sha256Writer {
    fn finalize(self) -> impl AsRef<[u8]> {
        self.digest.finalize()
    }
}

impl Write for Sha256Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.digest.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct BoundedVecWriter {
    bytes: Vec<u8>,
    max_bytes: usize,
}

impl BoundedVecWriter {
    fn new(max_bytes: usize, capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity.min(max_bytes)),
            max_bytes,
        }
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedVecWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next_len = self.bytes.len().checked_add(bytes.len()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::WriteZero,
                "serialized operation journal snapshot exceeds its configured bound",
            )
        })?;
        if next_len > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "serialized operation journal snapshot exceeds its configured bound",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn parse_snapshot(
    serialized: &[u8],
    expected_session_id: &OperationJournalSessionId,
) -> Result<PersistedOperationJournalSnapshot, OperationJournalPersistenceCorruption> {
    let snapshot: PersistedOperationJournalSnapshot = serde_json::from_slice(serialized)
        .map_err(|_| OperationJournalPersistenceCorruption::InvalidRecord)?;
    if snapshot.format != OPERATION_JOURNAL_SNAPSHOT_FORMAT {
        return Err(OperationJournalPersistenceCorruption::UnsupportedFormat);
    }
    if snapshot.persistence_schema_version != OPERATION_JOURNAL_PERSISTENCE_SCHEMA_VERSION {
        return Err(OperationJournalPersistenceCorruption::UnsupportedSchemaVersion);
    }
    if snapshot.persistence_sequence == 0 {
        return Err(OperationJournalPersistenceCorruption::InvalidSequence);
    }
    if snapshot.journal.session_id() != expected_session_id {
        return Err(OperationJournalPersistenceCorruption::SessionMismatch);
    }
    let expected_checksum = snapshot_checksum(&snapshot.journal, snapshot.persistence_sequence)
        .map_err(|_| OperationJournalPersistenceCorruption::InvalidRecord)?;
    if snapshot.checksum_sha256 != expected_checksum {
        return Err(OperationJournalPersistenceCorruption::ChecksumMismatch);
    }
    Ok(snapshot)
}

enum CheckpointRead {
    Missing,
    Valid(PersistedOperationJournalSnapshot),
    Rejected(OperationJournalPersistenceCorruption),
}

#[derive(Clone, Copy)]
enum RegularFileOpenMode {
    ReadOnly,
    ReadWrite,
    Append,
    CreateNewAppend,
}

pub(super) fn open_regular_read_file(path: &Path) -> io::Result<File> {
    open_regular_file(path, RegularFileOpenMode::ReadOnly)
}

fn open_regular_file(path: &Path, mode: RegularFileOpenMode) -> io::Result<File> {
    let mut options = OpenOptions::new();
    match mode {
        RegularFileOpenMode::ReadOnly => {
            options.read(true);
        }
        RegularFileOpenMode::ReadWrite => {
            options.read(true).write(true);
        }
        RegularFileOpenMode::Append => {
            options.read(true).append(true);
        }
        RegularFileOpenMode::CreateNewAppend => {
            options.read(true).append(true).create_new(true);
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
        if matches!(mode, RegularFileOpenMode::CreateNewAppend) {
            options.mode(0o600);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }

    let file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "operation journal path is a reparse point",
            ));
        }
    }
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "operation journal path is not a regular file",
        ));
    }
    Ok(file)
}

fn open_regular_file_handle(path: &Path, mode: RegularFileOpenMode) -> io::Result<FileHandle> {
    FileHandle::from_file(open_regular_file(path, mode)?)
}

fn read_checkpoint(
    paths: &OperationJournalPersistencePaths,
    config: &OperationJournalPersistenceConfig,
) -> CheckpointRead {
    let file = match open_regular_file(paths.checkpoint_path(), RegularFileOpenMode::ReadOnly) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return CheckpointRead::Missing;
        }
        Err(_) => {
            return CheckpointRead::Rejected(OperationJournalPersistenceCorruption::Unreadable);
        }
    };
    let handle = match FileHandle::from_file(file) {
        Ok(handle) => handle,
        Err(_) => {
            return CheckpointRead::Rejected(OperationJournalPersistenceCorruption::Unreadable);
        }
    };
    let metadata = match handle.as_file().metadata() {
        Ok(metadata) => metadata,
        Err(_) => {
            return CheckpointRead::Rejected(OperationJournalPersistenceCorruption::Unreadable);
        }
    };
    if !metadata.is_file() {
        return CheckpointRead::Rejected(OperationJournalPersistenceCorruption::Unreadable);
    }
    if metadata.len() > config.max_checkpoint_bytes {
        return CheckpointRead::Rejected(OperationJournalPersistenceCorruption::RecordTooLarge);
    }
    let mut serialized = Vec::new();
    if Read::take(
        handle.as_file(),
        config.max_checkpoint_bytes.saturating_add(1),
    )
    .read_to_end(&mut serialized)
    .is_err()
    {
        return CheckpointRead::Rejected(OperationJournalPersistenceCorruption::Unreadable);
    }
    if u64::try_from(serialized.len()).map_or(true, |bytes| bytes > config.max_checkpoint_bytes) {
        return CheckpointRead::Rejected(OperationJournalPersistenceCorruption::RecordTooLarge);
    }
    let current =
        match open_regular_file_handle(paths.checkpoint_path(), RegularFileOpenMode::ReadOnly) {
            Ok(current) => current,
            Err(_) => {
                return CheckpointRead::Rejected(
                    OperationJournalPersistenceCorruption::FileChanged,
                );
            }
        };
    if current != handle
        || current
            .as_file()
            .metadata()
            .map_or(true, |current| current.len() != metadata.len())
    {
        return CheckpointRead::Rejected(OperationJournalPersistenceCorruption::FileChanged);
    }
    match parse_snapshot(&serialized, &paths.session_id) {
        Ok(snapshot) => CheckpointRead::Valid(snapshot),
        Err(corruption) => CheckpointRead::Rejected(corruption),
    }
}

struct AppendLogRead {
    latest: Option<PersistedOperationJournalSnapshot>,
    file: Option<FileHandle>,
    entry_count: u64,
    valid_bytes: u64,
    discarded_tail_bytes: u64,
}

fn read_append_log(
    paths: &OperationJournalPersistencePaths,
    config: &OperationJournalPersistenceConfig,
    truncated_tail_handling: TruncatedLogTailHandling,
) -> Result<AppendLogRead, OperationJournalPersistenceError> {
    let file = match open_regular_file(paths.append_log_path(), RegularFileOpenMode::ReadOnly) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(AppendLogRead {
                latest: None,
                file: None,
                entry_count: 0,
                valid_bytes: 0,
                discarded_tail_bytes: 0,
            });
        }
        Err(source) => {
            return Err(OperationJournalPersistenceError::OpenLog { source });
        }
    };
    let mut log_file = FileHandle::from_file(file)
        .map_err(|source| OperationJournalPersistenceError::OpenLog { source })?;
    let metadata = log_file
        .as_file()
        .metadata()
        .map_err(|source| OperationJournalPersistenceError::OpenLog { source })?;
    if metadata.len() > config.max_log_bytes {
        return Err(OperationJournalPersistenceError::LogTooLarge {
            actual_bytes: metadata.len(),
            max_bytes: config.max_log_bytes,
        });
    }

    let mut reader = BufReader::new(log_file.as_file());
    let mut latest = None;
    let mut previous_sequence: Option<u64> = None;
    let mut entry_count = 0_u64;
    let mut valid_bytes = 0_u64;
    let mut discarded_tail_bytes = 0_u64;
    let mut line_number = 1_u64;

    loop {
        let line = match read_bounded_line(&mut reader, config.max_entry_bytes) {
            Ok(line) => line,
            Err(BoundedLineReadError::Io(source)) => {
                return Err(OperationJournalPersistenceError::ReadLog { source });
            }
            Err(BoundedLineReadError::TooLarge) => {
                return Err(OperationJournalPersistenceError::CorruptLogEntry {
                    line_number,
                    corruption: OperationJournalPersistenceCorruption::RecordTooLarge,
                });
            }
        };
        let Some(line) = line else {
            break;
        };
        if !line.terminated {
            discarded_tail_bytes = metadata.len().saturating_sub(valid_bytes);
            break;
        }
        let next_entry_count = entry_count.checked_add(1).ok_or(
            OperationJournalPersistenceError::TooManyLogEntries {
                actual_entries: u64::MAX,
                max_entries: config.max_log_entries,
            },
        )?;
        if next_entry_count > config.max_log_entries {
            return Err(OperationJournalPersistenceError::TooManyLogEntries {
                actual_entries: next_entry_count,
                max_entries: config.max_log_entries,
            });
        }
        let snapshot = parse_snapshot(strip_newline(&line.bytes), &paths.session_id).map_err(
            |corruption| OperationJournalPersistenceError::CorruptLogEntry {
                line_number,
                corruption,
            },
        )?;
        // Every record contains a complete journal snapshot, including all retained
        // generations, operations, and transitions. Compaction starts a new append
        // segment, so its first sequence may be greater than one (and may remain
        // recoverable without a checkpoint if checkpoint publication failed). Only
        // records within the same segment must be consecutive.
        if previous_sequence.is_some_and(|previous| {
            previous
                .checked_add(1)
                .is_none_or(|expected| expected != snapshot.persistence_sequence)
        }) {
            return Err(OperationJournalPersistenceError::CorruptLogEntry {
                line_number,
                corruption: OperationJournalPersistenceCorruption::InvalidSequence,
            });
        }

        valid_bytes = valid_bytes.checked_add(line.consumed).ok_or(
            OperationJournalPersistenceError::LogTooLarge {
                actual_bytes: u64::MAX,
                max_bytes: config.max_log_bytes,
            },
        )?;
        if valid_bytes > config.max_log_bytes {
            return Err(OperationJournalPersistenceError::LogTooLarge {
                actual_bytes: valid_bytes,
                max_bytes: config.max_log_bytes,
            });
        }
        previous_sequence = Some(snapshot.persistence_sequence);
        latest = Some(snapshot);
        entry_count = next_entry_count;
        line_number = line_number.saturating_add(1);
    }
    drop(reader);

    let current = open_regular_file_handle(paths.append_log_path(), RegularFileOpenMode::ReadOnly)
        .map_err(|_| OperationJournalPersistenceError::AppendLogChanged)?;
    let current_bytes = current
        .as_file()
        .metadata()
        .map_err(|_| OperationJournalPersistenceError::AppendLogChanged)?
        .len();
    if current != log_file || current_bytes != metadata.len() {
        return Err(OperationJournalPersistenceError::AppendLogChanged);
    }
    if discarded_tail_bytes > 0
        && truncated_tail_handling == TruncatedLogTailHandling::RepairForWriter
    {
        let repair_file =
            open_regular_file_handle(paths.append_log_path(), RegularFileOpenMode::ReadWrite)
                .map_err(|source| OperationJournalPersistenceError::RepairLog { source })?;
        let repair_bytes = repair_file
            .as_file()
            .metadata()
            .map_err(|source| OperationJournalPersistenceError::RepairLog { source })?
            .len();
        if repair_file != log_file || repair_bytes != metadata.len() {
            return Err(OperationJournalPersistenceError::AppendLogChanged);
        }
        repair_file
            .as_file()
            .set_len(valid_bytes)
            .map_err(|source| OperationJournalPersistenceError::RepairLog { source })?;
        repair_file
            .as_file()
            .sync_all()
            .map_err(|source| OperationJournalPersistenceError::RepairLog { source })?;
        let current =
            open_regular_file_handle(paths.append_log_path(), RegularFileOpenMode::ReadOnly)
                .map_err(|_| OperationJournalPersistenceError::AppendLogChanged)?;
        let current_bytes = current
            .as_file()
            .metadata()
            .map_err(|_| OperationJournalPersistenceError::AppendLogChanged)?
            .len();
        if current != repair_file || current_bytes != valid_bytes {
            return Err(OperationJournalPersistenceError::AppendLogChanged);
        }
        log_file = repair_file;
    }

    Ok(AppendLogRead {
        latest,
        file: Some(log_file),
        entry_count,
        valid_bytes,
        discarded_tail_bytes,
    })
}

struct BoundedLine {
    bytes: Vec<u8>,
    consumed: u64,
    terminated: bool,
}

enum BoundedLineReadError {
    Io(io::Error),
    TooLarge,
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    max_bytes: u64,
) -> Result<Option<BoundedLine>, BoundedLineReadError> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(BoundedLineReadError::Io)?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            return Ok(Some(BoundedLine {
                consumed: u64::try_from(bytes.len()).map_err(|_| BoundedLineReadError::TooLarge)?,
                bytes,
                terminated: false,
            }));
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |newline| newline + 1);
        let next_len = bytes
            .len()
            .checked_add(take)
            .ok_or(BoundedLineReadError::TooLarge)?;
        if u64::try_from(next_len).map_or(true, |next_len| next_len > max_bytes) {
            return Err(BoundedLineReadError::TooLarge);
        }
        let terminated = available.get(take.saturating_sub(1)) == Some(&b'\n');
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if terminated {
            return Ok(Some(BoundedLine {
                consumed: u64::try_from(bytes.len()).map_err(|_| BoundedLineReadError::TooLarge)?,
                bytes,
                terminated: true,
            }));
        }
    }
}

fn strip_newline(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\n").unwrap_or(line)
}

pub(super) fn write_atomic_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = persistence_parent(path)?;
    fs::create_dir_all(parent)?;
    let temporary = prepare_atomic_file(parent, contents)?;
    temporary.persist(path).map_err(|error| error.error)?;
    sync_parent_directory(parent)
}

fn prepare_atomic_file(parent: &Path, contents: &[u8]) -> io::Result<NamedTempFile> {
    let mut temporary = NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(contents)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    Ok(temporary)
}

fn persistence_parent(path: &Path) -> io::Result<&Path> {
    path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "operation journal path has no parent directory",
        )
    })
}

fn sync_parent_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

#[cfg(test)]
mod bounded_serialization_tests {
    use super::*;

    #[test]
    fn bounded_vec_writer_never_buffers_beyond_its_limit() {
        let mut writer = BoundedVecWriter::new(4, 0);
        writer.write_all(b"1234").expect("write up to the limit");

        let error = writer
            .write_all(b"5")
            .expect_err("writes beyond the limit must fail");
        assert_eq!(error.kind(), io::ErrorKind::WriteZero);
        assert_eq!(writer.len(), 4);
    }
}
