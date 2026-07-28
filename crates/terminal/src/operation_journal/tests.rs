use super::{
    OPERATION_JOURNAL_SCHEMA_VERSION, OperationGenerationId, OperationJournal,
    OperationJournalError, OperationJournalSessionId, OperationKind, OperationStatus,
    OperationTransitionOutcome,
};

fn generation(value: u64) -> OperationGenerationId {
    OperationGenerationId::new(value).expect("generation must be non-zero")
}

fn journal() -> OperationJournal {
    OperationJournal::new(
        OperationJournalSessionId::from_string("terminal_session_test"),
        generation(1),
        1_000,
    )
}

#[test]
fn journal_ids_are_stable_typed_and_unique() {
    let session_a = OperationJournalSessionId::new();
    let session_b = OperationJournalSessionId::new();
    assert_ne!(session_a, session_b);
    assert!(session_a.as_str().starts_with("terminal_session_"));

    let operation_a = super::OperationId::new();
    let operation_b = super::OperationId::new();
    assert_ne!(operation_a, operation_b);
    assert!(operation_a.as_str().starts_with("terminal_operation_"));

    assert!(OperationGenerationId::new(0).is_none());
    assert_eq!(generation(7).get(), 7);
    assert!(serde_json::from_str::<OperationGenerationId>("0").is_err());
}

#[test]
fn queueing_an_operation_records_the_initial_transition() {
    let mut journal = journal();

    let operation_id = journal
        .queue_operation(OperationKind::Command, None, 1_010)
        .expect("queue operation");
    let operation = journal.operation(&operation_id).expect("queued operation");

    assert_eq!(journal.schema_version(), OPERATION_JOURNAL_SCHEMA_VERSION);
    assert_eq!(journal.session_id().as_str(), "terminal_session_test");
    assert_eq!(operation.generation_id(), generation(1));
    assert_eq!(operation.parent_operation_id(), None);
    assert_eq!(operation.kind(), OperationKind::Command);
    assert_eq!(operation.status(), OperationStatus::Queued);
    assert_eq!(operation.transitions().len(), 1);
    assert_eq!(operation.transitions()[0].status(), OperationStatus::Queued);
    assert_eq!(operation.transitions()[0].occurred_at_unix_ms(), 1_010);
}

#[test]
fn legal_transitions_append_history_and_duplicate_status_is_idempotent() {
    let mut journal = journal();
    let operation_id = journal
        .queue_operation(OperationKind::ApplicationOperation, None, 1_010)
        .expect("queue operation");

    assert_eq!(
        journal
            .transition_operation(&operation_id, OperationStatus::Sent, 1_020)
            .expect("mark sent"),
        OperationTransitionOutcome::Changed
    );
    assert_eq!(
        journal
            .transition_operation(&operation_id, OperationStatus::Sent, 1_021)
            .expect("duplicate sent is idempotent"),
        OperationTransitionOutcome::Unchanged
    );
    journal
        .transition_operation(&operation_id, OperationStatus::Acknowledged, 1_030)
        .expect("mark acknowledged");
    journal
        .transition_operation(&operation_id, OperationStatus::Succeeded, 1_040)
        .expect("mark succeeded");

    let statuses = journal
        .operation(&operation_id)
        .expect("operation")
        .transitions()
        .iter()
        .map(|transition| transition.status())
        .collect::<Vec<_>>();
    assert_eq!(
        statuses,
        vec![
            OperationStatus::Queued,
            OperationStatus::Sent,
            OperationStatus::Acknowledged,
            OperationStatus::Succeeded,
        ]
    );
}

#[test]
fn invalid_transitions_and_backdated_updates_do_not_mutate_history() {
    let mut journal = journal();
    let operation_id = journal
        .queue_operation(OperationKind::UserInput, None, 1_010)
        .expect("queue operation");

    let invalid = journal
        .transition_operation(&operation_id, OperationStatus::Succeeded, 1_020)
        .expect_err("queued operation cannot jump directly to succeeded");
    assert!(matches!(
        invalid,
        OperationJournalError::InvalidStatusTransition {
            from: OperationStatus::Queued,
            to: OperationStatus::Succeeded,
            ..
        }
    ));

    journal
        .transition_operation(&operation_id, OperationStatus::Sent, 1_030)
        .expect("mark sent");
    let backdated = journal
        .transition_operation(&operation_id, OperationStatus::Failed, 1_029)
        .expect_err("transition timestamps cannot move backwards");
    assert!(matches!(
        backdated,
        OperationJournalError::TransitionTimestampMovedBackwards { .. }
    ));

    let operation = journal.operation(&operation_id).expect("operation");
    assert_eq!(operation.status(), OperationStatus::Sent);
    assert_eq!(operation.transitions().len(), 2);
}

#[test]
fn reconnect_seals_the_previous_generation_conservatively() {
    let mut journal = journal();
    let in_flight = journal
        .queue_operation(OperationKind::Paste, None, 1_010)
        .expect("queue in-flight operation");
    journal
        .transition_operation(&in_flight, OperationStatus::Sent, 1_020)
        .expect("mark in-flight operation sent");
    let completed = journal
        .queue_operation(OperationKind::FileOperation, None, 1_030)
        .expect("queue completed operation");
    journal
        .transition_operation(&completed, OperationStatus::Sent, 1_040)
        .expect("mark completed operation sent");
    journal
        .transition_operation(&completed, OperationStatus::Succeeded, 1_050)
        .expect("mark completed operation succeeded");

    journal
        .begin_generation(generation(2), 2_000)
        .expect("begin reconnect generation");

    let generations = journal.generations();
    assert_eq!(generations.len(), 2);
    assert_eq!(generations[0].ended_at_unix_ms(), Some(2_000));
    assert!(generations[0].is_closed());
    assert_eq!(
        journal.operation(&in_flight).expect("in-flight").status(),
        OperationStatus::Unknown
    );
    assert_eq!(
        journal.operation(&completed).expect("completed").status(),
        OperationStatus::Succeeded
    );
    assert_eq!(journal.current_generation().id(), generation(2));
    assert!(!journal.current_generation().is_closed());

    let new_operation = journal
        .queue_operation(OperationKind::ControlSequence, None, 2_010)
        .expect("queue operation in new generation");
    assert_eq!(
        journal
            .operation(&new_operation)
            .expect("new generation operation")
            .generation_id(),
        generation(2)
    );

    assert!(matches!(
        journal
            .transition_operation(&in_flight, OperationStatus::Succeeded, 2_020)
            .expect_err("old generations are read-only"),
        OperationJournalError::OperationNotInCurrentGeneration { .. }
    ));
}

#[test]
fn generation_changes_must_be_forward_and_time_ordered() {
    let mut journal = journal();
    let operation_id = journal
        .queue_operation(OperationKind::Command, None, 1_100)
        .expect("queue operation");

    assert!(matches!(
        journal
            .begin_generation(generation(1), 2_000)
            .expect_err("generation must increase"),
        OperationJournalError::GenerationDidNotAdvance { .. }
    ));
    assert!(matches!(
        journal
            .begin_generation(generation(2), 1_099)
            .expect_err("generation cannot close before its latest transition"),
        OperationJournalError::GenerationTimestampMovedBackwards { .. }
    ));

    assert_eq!(journal.generations().len(), 1);
    assert_eq!(
        journal
            .operation(&operation_id)
            .expect("operation")
            .status(),
        OperationStatus::Queued
    );
}

#[test]
fn parent_links_require_a_terminal_original_and_never_reuse_its_id() {
    let mut journal = journal();
    let original = journal
        .queue_operation(OperationKind::Command, None, 1_010)
        .expect("queue original");

    assert!(matches!(
        journal
            .queue_operation(OperationKind::Command, Some(&original), 1_020)
            .expect_err("an in-flight operation cannot be a retry parent"),
        OperationJournalError::ParentOperationNotTerminal { .. }
    ));

    journal
        .transition_operation(&original, OperationStatus::Sent, 1_030)
        .expect("mark sent");
    journal
        .transition_operation(&original, OperationStatus::Failed, 1_040)
        .expect("mark failed");

    let retry = journal
        .queue_operation(OperationKind::Command, Some(&original), 1_050)
        .expect("queue linked retry");
    assert_ne!(retry, original);
    assert_eq!(
        journal
            .operation(&retry)
            .expect("retry")
            .parent_operation_id(),
        Some(&original)
    );
    assert_eq!(
        journal.operation(&original).expect("original").status(),
        OperationStatus::Failed
    );

    let missing = super::OperationId::from_string("terminal_operation_missing");
    assert!(matches!(
        journal
            .queue_operation(OperationKind::Command, Some(&missing), 1_060)
            .expect_err("retry parent must exist"),
        OperationJournalError::ParentOperationNotFound { .. }
    ));
}

#[test]
fn terminal_statuses_cannot_be_rewritten() {
    for terminal_status in [
        OperationStatus::Succeeded,
        OperationStatus::Failed,
        OperationStatus::Unknown,
        OperationStatus::NeedsReview,
        OperationStatus::Canceled,
    ] {
        assert!(terminal_status.is_terminal());
    }
    for active_status in [
        OperationStatus::Queued,
        OperationStatus::Sent,
        OperationStatus::Acknowledged,
    ] {
        assert!(!active_status.is_terminal());
    }

    let mut journal = journal();
    let operation_id = journal
        .queue_operation(OperationKind::Unconfirmable, None, 1_010)
        .expect("queue operation");
    journal
        .transition_operation(&operation_id, OperationStatus::NeedsReview, 1_020)
        .expect("mark needs review");

    assert!(matches!(
        journal
            .transition_operation(&operation_id, OperationStatus::Failed, 1_030)
            .expect_err("terminal status is immutable"),
        OperationJournalError::InvalidStatusTransition {
            from: OperationStatus::NeedsReview,
            to: OperationStatus::Failed,
            ..
        }
    ));
}

#[test]
fn versioned_journal_round_trips_with_all_operation_kinds_and_status_names() {
    let kinds = [
        OperationKind::UserInput,
        OperationKind::Command,
        OperationKind::Paste,
        OperationKind::ControlSequence,
        OperationKind::FileOperation,
        OperationKind::ApplicationOperation,
        OperationKind::Unconfirmable,
    ];
    let mut journal = journal();
    for (offset, kind) in kinds.into_iter().enumerate() {
        journal
            .queue_operation(kind, None, 1_010 + offset as u64)
            .expect("queue operation kind");
    }

    let json = serde_json::to_string(&journal).expect("serialize journal");
    assert!(json.contains("\"schema_version\":1"));
    assert!(json.contains("\"user_input\""));
    assert!(json.contains("\"control_sequence\""));
    assert!(json.contains("\"application_operation\""));

    let restored: OperationJournal = serde_json::from_str(&json).expect("deserialize journal");
    restored.validate().expect("validate restored journal");
    assert_eq!(restored, journal);
}

#[test]
fn deserialization_rejects_unknown_schema_versions() {
    let journal = journal();
    let mut value = serde_json::to_value(journal).expect("serialize journal");
    value["schema_version"] = serde_json::json!(OPERATION_JOURNAL_SCHEMA_VERSION + 1);
    let error = serde_json::from_value::<OperationJournal>(value)
        .expect_err("future schema must be rejected while deserializing");
    assert!(
        error
            .to_string()
            .contains("unsupported operation journal schema version")
    );
}

#[test]
fn deserialization_rejects_snapshots_that_could_panic_public_mutators() {
    let mut no_generations = serde_json::to_value(journal()).expect("serialize journal");
    no_generations["generations"] = serde_json::json!([]);
    let error = serde_json::from_value::<OperationJournal>(no_generations)
        .expect_err("journals without generations must be rejected while deserializing");
    assert!(error.to_string().contains("journal has no generations"));

    let mut journal = journal();
    journal
        .queue_operation(OperationKind::Command, None, 1_010)
        .expect("queue operation");
    let mut no_transitions =
        serde_json::to_value(journal).expect("serialize journal with operation");
    no_transitions["generations"][0]["operations"][0]["transitions"] = serde_json::json!([]);
    let error = serde_json::from_value::<OperationJournal>(no_transitions)
        .expect_err("operations without transitions must be rejected while deserializing");
    assert!(error.to_string().contains("operation has no transitions"));
}

#[test]
fn deserialization_rejects_reordered_transition_history_and_retry_links() {
    let mut journal = journal();
    let original = journal
        .queue_operation(OperationKind::Command, None, 1_010)
        .expect("queue original");
    journal
        .transition_operation(&original, OperationStatus::Sent, 1_020)
        .expect("mark original sent");
    journal
        .transition_operation(&original, OperationStatus::Failed, 1_030)
        .expect("mark original failed");

    let mut reordered_history =
        serde_json::to_value(&journal).expect("serialize transition history");
    reordered_history["generations"][0]["operations"][0]["transitions"][1]["sequence"] =
        serde_json::json!(3);
    reordered_history["generations"][0]["operations"][0]["transitions"][2]["sequence"] =
        serde_json::json!(2);
    let error = serde_json::from_value::<OperationJournal>(reordered_history)
        .expect_err("per-operation transition order must be rejected while deserializing");
    assert!(
        error
            .to_string()
            .contains("operation transition sequences are not increasing")
    );

    let retry = journal
        .queue_operation(OperationKind::Command, Some(&original), 1_040)
        .expect("queue retry");
    let mut reordered_parent = serde_json::to_value(&journal).expect("serialize parent history");
    reordered_parent["generations"][0]["operations"][0]["transitions"][2]["sequence"] =
        serde_json::json!(4);
    reordered_parent["generations"][0]["operations"][1]["transitions"][0]["sequence"] =
        serde_json::json!(3);
    let error = serde_json::from_value::<OperationJournal>(reordered_parent)
        .expect_err("parent transition order must be rejected while deserializing");
    assert!(
        error
            .to_string()
            .contains("parent terminal transition does not precede child")
    );

    assert_eq!(
        journal
            .operation(&retry)
            .expect("unmodified retry")
            .parent_operation_id(),
        Some(&original)
    );
}
