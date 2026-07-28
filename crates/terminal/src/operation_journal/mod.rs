mod model;
mod persistence;
mod redaction;
mod session_history;

pub use model::{
    OPERATION_JOURNAL_SCHEMA_VERSION, OperationGeneration, OperationGenerationId, OperationId,
    OperationJournal, OperationJournalError, OperationJournalSessionId, OperationKind,
    OperationRecord, OperationStatus, OperationStatusTransition, OperationTransitionOutcome,
};
pub use persistence::{
    OPERATION_JOURNAL_PERSISTENCE_SCHEMA_VERSION, OperationJournalFileStore,
    OperationJournalPersistOutcome, OperationJournalPersistenceConfig,
    OperationJournalPersistenceCorruption, OperationJournalPersistenceError,
    OperationJournalPersistencePaths, OperationJournalRecovery, OperationJournalRecoverySource,
};
pub use redaction::{
    OPERATION_PAYLOAD_PREVIEW_CHARS, OperationPayloadCompleteness, OperationPayloadFormat,
    RedactedOperationPayload, SensitiveOperationPayload,
};
pub use session_history::{
    OPERATION_JOURNAL_SESSION_MANIFEST_SCHEMA_VERSION, OperationJournalHistoryConfig,
    OperationJournalHistoryDiscovery, OperationJournalHistoryError,
    OperationJournalHistorySnapshot, OperationJournalHistoryStore, OperationJournalRecoveryWarning,
    OperationJournalRecoveryWarningKind, OperationJournalScope, OperationJournalScopeError,
    OperationJournalScopeKind, OperationJournalSessionManifest,
};

#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod redaction_tests;
#[cfg(test)]
mod session_history_tests;
#[cfg(test)]
mod tests;
