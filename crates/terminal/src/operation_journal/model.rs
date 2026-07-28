use super::redaction::{OperationPayloadFormat, RedactedOperationPayload};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::num::NonZeroU64;

pub const OPERATION_JOURNAL_SCHEMA_VERSION: u16 = 1;

macro_rules! journal_string_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(format!("{}_{}", $prefix, uuid::Uuid::new_v4().simple()))
            }

            pub fn from_string(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({})", stringify!($name), self.0)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }
    };
}

journal_string_id!(OperationJournalSessionId, "terminal_session");
journal_string_id!(OperationId, "terminal_operation");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OperationGenerationId(NonZeroU64);

impl OperationGenerationId {
    pub fn new(value: u64) -> Option<Self> {
        NonZeroU64::new(value).map(Self)
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Display for OperationGenerationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    UserInput,
    Command,
    Paste,
    ControlSequence,
    FileOperation,
    ApplicationOperation,
    Unconfirmable,
}

impl OperationKind {
    fn allows_structured_payload(self) -> bool {
        matches!(self, Self::FileOperation | Self::ApplicationOperation)
    }

    pub(super) fn validate_redacted_payload(
        self,
        payload: Option<&RedactedOperationPayload>,
    ) -> Result<(), OperationJournalError> {
        if payload.is_some_and(|payload| {
            payload.format() == OperationPayloadFormat::StructuredJson
                && !self.allows_structured_payload()
        }) {
            return Err(OperationJournalError::StructuredPayloadNotAllowed {
                operation_kind: self,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Queued,
    Sent,
    Acknowledged,
    Succeeded,
    Failed,
    Unknown,
    NeedsReview,
    Canceled,
}

impl OperationStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Unknown | Self::NeedsReview | Self::Canceled
        )
    }

    fn can_transition_to(self, next: Self) -> bool {
        match self {
            Self::Queued => matches!(
                next,
                Self::Sent | Self::Failed | Self::Unknown | Self::NeedsReview | Self::Canceled
            ),
            Self::Sent => matches!(
                next,
                Self::Acknowledged
                    | Self::Succeeded
                    | Self::Failed
                    | Self::Unknown
                    | Self::NeedsReview
            ),
            Self::Acknowledged => matches!(
                next,
                Self::Succeeded | Self::Failed | Self::Unknown | Self::NeedsReview
            ),
            Self::Succeeded | Self::Failed | Self::Unknown | Self::NeedsReview | Self::Canceled => {
                false
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationTransitionOutcome {
    Changed,
    Unchanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationStatusTransition {
    sequence: u64,
    status: OperationStatus,
    occurred_at_unix_ms: u64,
}

impl OperationStatusTransition {
    fn new(sequence: u64, status: OperationStatus, occurred_at_unix_ms: u64) -> Self {
        Self {
            sequence,
            status,
            occurred_at_unix_ms,
        }
    }

    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn status(&self) -> OperationStatus {
        self.status
    }

    pub fn occurred_at_unix_ms(&self) -> u64 {
        self.occurred_at_unix_ms
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRecord {
    operation_id: OperationId,
    generation_id: OperationGenerationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_operation_id: Option<OperationId>,
    kind: OperationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    redacted_payload: Option<RedactedOperationPayload>,
    transitions: Vec<OperationStatusTransition>,
}

impl OperationRecord {
    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    pub fn generation_id(&self) -> OperationGenerationId {
        self.generation_id
    }

    pub fn parent_operation_id(&self) -> Option<&OperationId> {
        self.parent_operation_id.as_ref()
    }

    pub fn kind(&self) -> OperationKind {
        self.kind
    }

    pub fn redacted_payload(&self) -> Option<&RedactedOperationPayload> {
        self.redacted_payload.as_ref()
    }

    pub fn transitions(&self) -> &[OperationStatusTransition] {
        &self.transitions
    }

    pub fn status(&self) -> OperationStatus {
        self.transitions
            .last()
            .map(OperationStatusTransition::status)
            .unwrap_or(OperationStatus::Queued)
    }

    fn last_transition_at(&self) -> Option<u64> {
        self.transitions
            .last()
            .map(OperationStatusTransition::occurred_at_unix_ms)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationGeneration {
    id: OperationGenerationId,
    started_at_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ended_at_unix_ms: Option<u64>,
    operations: Vec<OperationRecord>,
}

impl OperationGeneration {
    fn new(id: OperationGenerationId, started_at_unix_ms: u64) -> Self {
        Self {
            id,
            started_at_unix_ms,
            ended_at_unix_ms: None,
            operations: Vec::new(),
        }
    }

    pub fn id(&self) -> OperationGenerationId {
        self.id
    }

    pub fn started_at_unix_ms(&self) -> u64 {
        self.started_at_unix_ms
    }

    pub fn ended_at_unix_ms(&self) -> Option<u64> {
        self.ended_at_unix_ms
    }

    pub fn is_closed(&self) -> bool {
        self.ended_at_unix_ms.is_some()
    }

    pub fn operations(&self) -> &[OperationRecord] {
        &self.operations
    }

    fn latest_transition_at(&self) -> Option<u64> {
        self.operations
            .iter()
            .filter_map(OperationRecord::last_transition_at)
            .max()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OperationJournal {
    schema_version: u16,
    session_id: OperationJournalSessionId,
    generations: Vec<OperationGeneration>,
    last_transition_sequence: u64,
}

#[derive(Deserialize)]
struct OperationJournalSnapshot {
    schema_version: u16,
    session_id: OperationJournalSessionId,
    generations: Vec<OperationGeneration>,
    last_transition_sequence: u64,
}

impl<'de> Deserialize<'de> for OperationJournal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let snapshot = OperationJournalSnapshot::deserialize(deserializer)?;
        let journal = Self {
            schema_version: snapshot.schema_version,
            session_id: snapshot.session_id,
            generations: snapshot.generations,
            last_transition_sequence: snapshot.last_transition_sequence,
        };
        journal.validate().map_err(serde::de::Error::custom)?;
        Ok(journal)
    }
}

impl OperationJournal {
    pub fn new(
        session_id: OperationJournalSessionId,
        initial_generation_id: OperationGenerationId,
        started_at_unix_ms: u64,
    ) -> Self {
        Self {
            schema_version: OPERATION_JOURNAL_SCHEMA_VERSION,
            session_id,
            generations: vec![OperationGeneration::new(
                initial_generation_id,
                started_at_unix_ms,
            )],
            last_transition_sequence: 0,
        }
    }

    pub fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub fn session_id(&self) -> &OperationJournalSessionId {
        &self.session_id
    }

    pub fn generations(&self) -> &[OperationGeneration] {
        &self.generations
    }

    pub fn current_generation(&self) -> &OperationGeneration {
        self.generations
            .last()
            .expect("operation journals always contain an initial generation")
    }

    pub fn operation(&self, operation_id: &OperationId) -> Option<&OperationRecord> {
        self.generations
            .iter()
            .flat_map(OperationGeneration::operations)
            .find(|operation| operation.operation_id == *operation_id)
    }

    pub fn queue_operation(
        &mut self,
        kind: OperationKind,
        parent_operation_id: Option<&OperationId>,
        occurred_at_unix_ms: u64,
    ) -> Result<OperationId, OperationJournalError> {
        self.queue_operation_inner(
            self.unique_operation_id(),
            kind,
            parent_operation_id,
            None,
            occurred_at_unix_ms,
        )
    }

    pub fn queue_operation_with_payload(
        &mut self,
        kind: OperationKind,
        parent_operation_id: Option<&OperationId>,
        payload: RedactedOperationPayload,
        occurred_at_unix_ms: u64,
    ) -> Result<OperationId, OperationJournalError> {
        kind.validate_redacted_payload(Some(&payload))?;
        self.queue_operation_inner(
            self.unique_operation_id(),
            kind,
            parent_operation_id,
            Some(payload),
            occurred_at_unix_ms,
        )
    }

    pub(super) fn queue_operation_with_id(
        &mut self,
        operation_id: OperationId,
        kind: OperationKind,
        parent_operation_id: Option<&OperationId>,
        redacted_payload: Option<RedactedOperationPayload>,
        occurred_at_unix_ms: u64,
    ) -> Result<(), OperationJournalError> {
        kind.validate_redacted_payload(redacted_payload.as_ref())?;
        self.queue_operation_inner(
            operation_id,
            kind,
            parent_operation_id,
            redacted_payload,
            occurred_at_unix_ms,
        )
        .map(|_| ())
    }

    fn queue_operation_inner(
        &mut self,
        operation_id: OperationId,
        kind: OperationKind,
        parent_operation_id: Option<&OperationId>,
        redacted_payload: Option<RedactedOperationPayload>,
        occurred_at_unix_ms: u64,
    ) -> Result<OperationId, OperationJournalError> {
        let current_generation = self.current_generation();
        if current_generation.is_closed() {
            return Err(OperationJournalError::CurrentGenerationClosed {
                generation_id: current_generation.id,
            });
        }
        if occurred_at_unix_ms < current_generation.started_at_unix_ms {
            return Err(OperationJournalError::OperationTimestampBeforeGeneration {
                generation_id: current_generation.id,
                generation_started_at_unix_ms: current_generation.started_at_unix_ms,
                occurred_at_unix_ms,
            });
        }
        if self.operation(&operation_id).is_some() {
            return Err(OperationJournalError::OperationIdAlreadyExists { operation_id });
        }

        let parent_operation_id = if let Some(parent_operation_id) = parent_operation_id {
            let parent = self.operation(parent_operation_id).ok_or_else(|| {
                OperationJournalError::ParentOperationNotFound {
                    parent_operation_id: parent_operation_id.clone(),
                }
            })?;
            if !parent.status().is_terminal() {
                return Err(OperationJournalError::ParentOperationNotTerminal {
                    parent_operation_id: parent_operation_id.clone(),
                    status: parent.status(),
                });
            }
            let parent_terminal_at_unix_ms = parent
                .last_transition_at()
                .expect("validated operations always contain an initial transition");
            if occurred_at_unix_ms < parent_terminal_at_unix_ms {
                return Err(OperationJournalError::OperationTimestampBeforeParent {
                    parent_operation_id: parent_operation_id.clone(),
                    parent_terminal_at_unix_ms,
                    occurred_at_unix_ms,
                });
            }
            Some(parent_operation_id.clone())
        } else {
            None
        };

        let sequence = self.next_transition_sequence()?;
        let generation_id = self.current_generation().id;
        let operation = OperationRecord {
            operation_id: operation_id.clone(),
            generation_id,
            parent_operation_id,
            kind,
            redacted_payload,
            transitions: vec![OperationStatusTransition::new(
                sequence,
                OperationStatus::Queued,
                occurred_at_unix_ms,
            )],
        };

        self.generations
            .last_mut()
            .expect("operation journals always contain an initial generation")
            .operations
            .push(operation);
        self.last_transition_sequence = sequence;
        Ok(operation_id)
    }

    pub fn transition_operation(
        &mut self,
        operation_id: &OperationId,
        next_status: OperationStatus,
        occurred_at_unix_ms: u64,
    ) -> Result<OperationTransitionOutcome, OperationJournalError> {
        let (generation_index, operation_index) = self
            .operation_location(operation_id)
            .ok_or_else(|| OperationJournalError::OperationNotFound {
                operation_id: operation_id.clone(),
            })?;
        let current_generation_index = self.generations.len() - 1;
        if generation_index != current_generation_index
            || self.generations[generation_index].is_closed()
        {
            return Err(OperationJournalError::OperationNotInCurrentGeneration {
                operation_id: operation_id.clone(),
                operation_generation_id: self.generations[generation_index].id,
                current_generation_id: self.current_generation().id,
            });
        }

        let operation = &self.generations[generation_index].operations[operation_index];
        let current_status = operation.status();
        if current_status == next_status {
            return Ok(OperationTransitionOutcome::Unchanged);
        }
        if !current_status.can_transition_to(next_status) {
            return Err(OperationJournalError::InvalidStatusTransition {
                operation_id: operation_id.clone(),
                from: current_status,
                to: next_status,
            });
        }
        let previous_at_unix_ms = operation
            .last_transition_at()
            .expect("queued operations always contain an initial transition");
        if occurred_at_unix_ms < previous_at_unix_ms {
            return Err(OperationJournalError::TransitionTimestampMovedBackwards {
                operation_id: operation_id.clone(),
                previous_at_unix_ms,
                occurred_at_unix_ms,
            });
        }

        let sequence = self.next_transition_sequence()?;
        self.generations[generation_index].operations[operation_index]
            .transitions
            .push(OperationStatusTransition::new(
                sequence,
                next_status,
                occurred_at_unix_ms,
            ));
        self.last_transition_sequence = sequence;
        Ok(OperationTransitionOutcome::Changed)
    }

    pub fn begin_generation(
        &mut self,
        generation_id: OperationGenerationId,
        started_at_unix_ms: u64,
    ) -> Result<(), OperationJournalError> {
        let current = self.current_generation();
        if generation_id <= current.id {
            return Err(OperationJournalError::GenerationDidNotAdvance {
                current_generation_id: current.id,
                requested_generation_id: generation_id,
            });
        }

        let minimum_started_at_unix_ms = current
            .ended_at_unix_ms
            .into_iter()
            .chain(current.latest_transition_at())
            .chain(std::iter::once(current.started_at_unix_ms))
            .max()
            .expect("generation start timestamp is always present");
        if started_at_unix_ms < minimum_started_at_unix_ms {
            return Err(OperationJournalError::GenerationTimestampMovedBackwards {
                generation_id: current.id,
                latest_recorded_at_unix_ms: minimum_started_at_unix_ms,
                requested_at_unix_ms: started_at_unix_ms,
            });
        }

        let active_operation_count = if current.is_closed() {
            0
        } else {
            current
                .operations
                .iter()
                .filter(|operation| !operation.status().is_terminal())
                .count()
        };
        let active_operation_count = u64::try_from(active_operation_count)
            .map_err(|_| OperationJournalError::TransitionSequenceOverflow)?;
        self.last_transition_sequence
            .checked_add(active_operation_count)
            .ok_or(OperationJournalError::TransitionSequenceOverflow)?;

        if !self.current_generation().is_closed() {
            let current_index = self.generations.len() - 1;
            let current = &mut self.generations[current_index];
            for operation in &mut current.operations {
                if operation.status().is_terminal() {
                    continue;
                }
                self.last_transition_sequence += 1;
                operation.transitions.push(OperationStatusTransition::new(
                    self.last_transition_sequence,
                    OperationStatus::Unknown,
                    started_at_unix_ms,
                ));
            }
            current.ended_at_unix_ms = Some(started_at_unix_ms);
        }

        self.generations
            .push(OperationGeneration::new(generation_id, started_at_unix_ms));
        Ok(())
    }

    pub fn validate(&self) -> Result<(), OperationJournalError> {
        if self.schema_version != OPERATION_JOURNAL_SCHEMA_VERSION {
            return Err(OperationJournalError::UnsupportedSchemaVersion {
                expected: OPERATION_JOURNAL_SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        if self.session_id.as_str().is_empty() {
            return Err(OperationJournalError::InvalidSnapshot {
                reason: "session id is empty",
            });
        }
        if self.generations.is_empty() {
            return Err(OperationJournalError::InvalidSnapshot {
                reason: "journal has no generations",
            });
        }

        #[derive(Clone, Copy)]
        struct OperationValidationInfo {
            first_sequence: u64,
            queued_at_unix_ms: u64,
            terminal_sequence: Option<u64>,
            terminal_at_unix_ms: Option<u64>,
        }

        let mut operation_ids = HashSet::new();
        let mut operation_info = HashMap::new();
        let mut parent_links = Vec::new();
        let mut transition_sequences = Vec::new();
        let mut previous_generation: Option<&OperationGeneration> = None;

        for (generation_index, generation) in self.generations.iter().enumerate() {
            if let Some(previous) = previous_generation {
                if generation.id <= previous.id {
                    return Err(OperationJournalError::InvalidSnapshot {
                        reason: "generation ids are not strictly increasing",
                    });
                }
                let previous_ended_at_unix_ms =
                    previous
                        .ended_at_unix_ms
                        .ok_or(OperationJournalError::InvalidSnapshot {
                            reason: "a non-final generation is not closed",
                        })?;
                if generation.started_at_unix_ms < previous_ended_at_unix_ms {
                    return Err(OperationJournalError::InvalidSnapshot {
                        reason: "generation timestamps overlap",
                    });
                }
            }
            if generation
                .ended_at_unix_ms
                .is_some_and(|ended_at| ended_at < generation.started_at_unix_ms)
            {
                return Err(OperationJournalError::InvalidSnapshot {
                    reason: "generation ends before it starts",
                });
            }
            if generation_index + 1 < self.generations.len()
                && generation.ended_at_unix_ms.is_none()
            {
                return Err(OperationJournalError::InvalidSnapshot {
                    reason: "a non-final generation is not closed",
                });
            }

            for operation in &generation.operations {
                if operation.operation_id.as_str().is_empty() {
                    return Err(OperationJournalError::InvalidSnapshot {
                        reason: "operation id is empty",
                    });
                }
                if operation.generation_id != generation.id {
                    return Err(OperationJournalError::InvalidSnapshot {
                        reason: "operation generation does not match its container",
                    });
                }
                if !operation_ids.insert(operation.operation_id.clone()) {
                    return Err(OperationJournalError::InvalidSnapshot {
                        reason: "duplicate operation id",
                    });
                }
                if let Some(payload) = &operation.redacted_payload {
                    payload
                        .validate_snapshot()
                        .map_err(|reason| OperationJournalError::InvalidSnapshot { reason })?;
                    if payload.format() == OperationPayloadFormat::StructuredJson
                        && !operation.kind.allows_structured_payload()
                    {
                        return Err(OperationJournalError::InvalidSnapshot {
                            reason: "raw terminal operation contains structured payload",
                        });
                    }
                }
                let Some(first_transition) = operation.transitions.first() else {
                    return Err(OperationJournalError::InvalidSnapshot {
                        reason: "operation has no transitions",
                    });
                };
                if first_transition.status != OperationStatus::Queued {
                    return Err(OperationJournalError::InvalidSnapshot {
                        reason: "operation does not start in queued state",
                    });
                }
                if first_transition.occurred_at_unix_ms < generation.started_at_unix_ms {
                    return Err(OperationJournalError::InvalidSnapshot {
                        reason: "operation predates its generation",
                    });
                }

                let mut previous_transition = first_transition;
                transition_sequences.push(first_transition.sequence);
                for transition in operation.transitions.iter().skip(1) {
                    if transition.sequence <= previous_transition.sequence {
                        return Err(OperationJournalError::InvalidSnapshot {
                            reason: "operation transition sequences are not increasing",
                        });
                    }
                    if transition.occurred_at_unix_ms < previous_transition.occurred_at_unix_ms {
                        return Err(OperationJournalError::InvalidSnapshot {
                            reason: "operation transition timestamps move backwards",
                        });
                    }
                    if !previous_transition
                        .status
                        .can_transition_to(transition.status)
                    {
                        return Err(OperationJournalError::InvalidSnapshot {
                            reason: "operation contains an invalid status transition",
                        });
                    }
                    transition_sequences.push(transition.sequence);
                    previous_transition = transition;
                }

                if let Some(ended_at_unix_ms) = generation.ended_at_unix_ms {
                    if previous_transition.occurred_at_unix_ms > ended_at_unix_ms {
                        return Err(OperationJournalError::InvalidSnapshot {
                            reason: "operation transition occurs after generation end",
                        });
                    }
                    if !previous_transition.status.is_terminal() {
                        return Err(OperationJournalError::InvalidSnapshot {
                            reason: "closed generation contains an active operation",
                        });
                    }
                }

                let terminal_at_unix_ms = previous_transition
                    .status
                    .is_terminal()
                    .then_some(previous_transition.occurred_at_unix_ms);
                let terminal_sequence = previous_transition
                    .status
                    .is_terminal()
                    .then_some(previous_transition.sequence);
                operation_info.insert(
                    operation.operation_id.clone(),
                    OperationValidationInfo {
                        first_sequence: first_transition.sequence,
                        queued_at_unix_ms: first_transition.occurred_at_unix_ms,
                        terminal_sequence,
                        terminal_at_unix_ms,
                    },
                );
                if let Some(parent_operation_id) = &operation.parent_operation_id {
                    parent_links
                        .push((operation.operation_id.clone(), parent_operation_id.clone()));
                }
            }

            previous_generation = Some(generation);
        }

        transition_sequences.sort_unstable();
        if transition_sequences.len() as u64 != self.last_transition_sequence {
            return Err(OperationJournalError::InvalidSnapshot {
                reason: "transition sequence cursor does not match history",
            });
        }
        for (index, sequence) in transition_sequences.iter().copied().enumerate() {
            let expected = u64::try_from(index)
                .ok()
                .and_then(|index| index.checked_add(1))
                .ok_or(OperationJournalError::TransitionSequenceOverflow)?;
            if sequence != expected {
                return Err(OperationJournalError::InvalidSnapshot {
                    reason: "transition sequences are not contiguous",
                });
            }
        }

        for (operation_id, parent_operation_id) in parent_links {
            if operation_id == parent_operation_id {
                return Err(OperationJournalError::InvalidSnapshot {
                    reason: "operation is its own parent",
                });
            }
            let child = operation_info.get(&operation_id).ok_or(
                OperationJournalError::InvalidSnapshot {
                    reason: "child operation metadata is missing",
                },
            )?;
            let parent = operation_info.get(&parent_operation_id).ok_or(
                OperationJournalError::InvalidSnapshot {
                    reason: "parent operation does not exist",
                },
            )?;
            let parent_terminal_at_unix_ms =
                parent
                    .terminal_at_unix_ms
                    .ok_or(OperationJournalError::InvalidSnapshot {
                        reason: "parent operation is not terminal",
                    })?;
            let parent_terminal_sequence =
                parent
                    .terminal_sequence
                    .ok_or(OperationJournalError::InvalidSnapshot {
                        reason: "parent operation terminal sequence is missing",
                    })?;
            if parent.first_sequence >= child.first_sequence {
                return Err(OperationJournalError::InvalidSnapshot {
                    reason: "parent operation does not precede child",
                });
            }
            if parent_terminal_sequence >= child.first_sequence {
                return Err(OperationJournalError::InvalidSnapshot {
                    reason: "parent terminal transition does not precede child",
                });
            }
            if parent_terminal_at_unix_ms > child.queued_at_unix_ms {
                return Err(OperationJournalError::InvalidSnapshot {
                    reason: "child operation predates parent terminal state",
                });
            }
        }

        Ok(())
    }

    fn next_transition_sequence(&self) -> Result<u64, OperationJournalError> {
        self.last_transition_sequence
            .checked_add(1)
            .ok_or(OperationJournalError::TransitionSequenceOverflow)
    }

    fn unique_operation_id(&self) -> OperationId {
        loop {
            let operation_id = OperationId::new();
            if self.operation(&operation_id).is_none() {
                return operation_id;
            }
        }
    }

    fn operation_location(&self, operation_id: &OperationId) -> Option<(usize, usize)> {
        self.generations
            .iter()
            .enumerate()
            .find_map(|(generation_index, generation)| {
                generation
                    .operations
                    .iter()
                    .position(|operation| operation.operation_id == *operation_id)
                    .map(|operation_index| (generation_index, operation_index))
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationJournalError {
    UnsupportedSchemaVersion {
        expected: u16,
        found: u16,
    },
    CurrentGenerationClosed {
        generation_id: OperationGenerationId,
    },
    GenerationDidNotAdvance {
        current_generation_id: OperationGenerationId,
        requested_generation_id: OperationGenerationId,
    },
    GenerationTimestampMovedBackwards {
        generation_id: OperationGenerationId,
        latest_recorded_at_unix_ms: u64,
        requested_at_unix_ms: u64,
    },
    OperationNotFound {
        operation_id: OperationId,
    },
    OperationIdAlreadyExists {
        operation_id: OperationId,
    },
    OperationNotInCurrentGeneration {
        operation_id: OperationId,
        operation_generation_id: OperationGenerationId,
        current_generation_id: OperationGenerationId,
    },
    OperationTimestampBeforeGeneration {
        generation_id: OperationGenerationId,
        generation_started_at_unix_ms: u64,
        occurred_at_unix_ms: u64,
    },
    ParentOperationNotFound {
        parent_operation_id: OperationId,
    },
    ParentOperationNotTerminal {
        parent_operation_id: OperationId,
        status: OperationStatus,
    },
    OperationTimestampBeforeParent {
        parent_operation_id: OperationId,
        parent_terminal_at_unix_ms: u64,
        occurred_at_unix_ms: u64,
    },
    InvalidStatusTransition {
        operation_id: OperationId,
        from: OperationStatus,
        to: OperationStatus,
    },
    StructuredPayloadNotAllowed {
        operation_kind: OperationKind,
    },
    TransitionTimestampMovedBackwards {
        operation_id: OperationId,
        previous_at_unix_ms: u64,
        occurred_at_unix_ms: u64,
    },
    TransitionSequenceOverflow,
    InvalidSnapshot {
        reason: &'static str,
    },
}

impl fmt::Display for OperationJournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion { expected, found } => {
                write!(
                    formatter,
                    "unsupported operation journal schema version {found}; expected {expected}"
                )
            }
            Self::CurrentGenerationClosed { generation_id } => {
                write!(
                    formatter,
                    "operation journal generation {generation_id} is closed"
                )
            }
            Self::GenerationDidNotAdvance {
                current_generation_id,
                requested_generation_id,
            } => write!(
                formatter,
                "operation journal generation must advance beyond {current_generation_id}, got {requested_generation_id}"
            ),
            Self::GenerationTimestampMovedBackwards {
                generation_id,
                latest_recorded_at_unix_ms,
                requested_at_unix_ms,
            } => write!(
                formatter,
                "generation {generation_id} cannot start at {requested_at_unix_ms} before {latest_recorded_at_unix_ms}"
            ),
            Self::OperationNotFound { operation_id } => {
                write!(formatter, "operation {operation_id} was not found")
            }
            Self::OperationIdAlreadyExists { operation_id } => {
                write!(formatter, "operation {operation_id} already exists")
            }
            Self::OperationNotInCurrentGeneration {
                operation_id,
                operation_generation_id,
                current_generation_id,
            } => write!(
                formatter,
                "operation {operation_id} belongs to generation {operation_generation_id}, not current generation {current_generation_id}"
            ),
            Self::OperationTimestampBeforeGeneration {
                generation_id,
                generation_started_at_unix_ms,
                occurred_at_unix_ms,
            } => write!(
                formatter,
                "operation timestamp {occurred_at_unix_ms} predates generation {generation_id} start {generation_started_at_unix_ms}"
            ),
            Self::ParentOperationNotFound {
                parent_operation_id,
            } => write!(
                formatter,
                "parent operation {parent_operation_id} was not found"
            ),
            Self::ParentOperationNotTerminal {
                parent_operation_id,
                status,
            } => write!(
                formatter,
                "parent operation {parent_operation_id} is not terminal ({status:?})"
            ),
            Self::OperationTimestampBeforeParent {
                parent_operation_id,
                parent_terminal_at_unix_ms,
                occurred_at_unix_ms,
            } => write!(
                formatter,
                "operation timestamp {occurred_at_unix_ms} predates parent {parent_operation_id} terminal timestamp {parent_terminal_at_unix_ms}"
            ),
            Self::InvalidStatusTransition {
                operation_id,
                from,
                to,
            } => write!(
                formatter,
                "operation {operation_id} cannot transition from {from:?} to {to:?}"
            ),
            Self::StructuredPayloadNotAllowed { operation_kind } => write!(
                formatter,
                "operation kind {operation_kind:?} requires an opaque payload summary"
            ),
            Self::TransitionTimestampMovedBackwards {
                operation_id,
                previous_at_unix_ms,
                occurred_at_unix_ms,
            } => write!(
                formatter,
                "operation {operation_id} transition timestamp {occurred_at_unix_ms} predates {previous_at_unix_ms}"
            ),
            Self::TransitionSequenceOverflow => {
                formatter.write_str("operation journal transition sequence overflow")
            }
            Self::InvalidSnapshot { reason } => {
                write!(formatter, "invalid operation journal snapshot: {reason}")
            }
        }
    }
}

impl std::error::Error for OperationJournalError {}
