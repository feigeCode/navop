mod model;
mod redaction;

pub use model::{
    OPERATION_JOURNAL_SCHEMA_VERSION, OperationGeneration, OperationGenerationId, OperationId,
    OperationJournal, OperationJournalError, OperationJournalSessionId, OperationKind,
    OperationRecord, OperationStatus, OperationStatusTransition, OperationTransitionOutcome,
};
pub use redaction::{
    OPERATION_PAYLOAD_PREVIEW_CHARS, OperationPayloadCompleteness, OperationPayloadFormat,
    RedactedOperationPayload, SensitiveOperationPayload,
};

#[cfg(test)]
mod redaction_tests;
#[cfg(test)]
mod tests;
