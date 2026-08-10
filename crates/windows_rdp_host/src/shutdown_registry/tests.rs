use super::*;

#[test]
fn registration_is_unique_and_generation_bound() {
    let mut registry = WindowsRdpShutdownRegistry::new();

    let first = registry.register(41).expect("first registration");
    let second = registry.register(41).expect("second registration");

    assert_ne!(first.token(), second.token());
    assert_eq!(first.generation(), 41);
    assert_eq!(second.generation(), 41);
    assert_eq!(registry.active_count(), 2);
}

#[test]
fn registration_token_exhaustion_is_reported_without_mutating_state() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    registry.next_token = u64::MAX;

    assert_eq!(
        registry.register(42),
        Err(WindowsRdpRegistrationError::TokenExhausted)
    );
    assert_eq!(registry.lifecycle(), WindowsRdpShutdownLifecycle::Running);
    assert_eq!(registry.active_count(), 0);
}

#[test]
fn shutdown_closes_admission_before_snapshotting_hosts() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let first = registry.register(11).expect("registration");

    let snapshot = registry.begin_drain();

    assert_eq!(registry.lifecycle(), WindowsRdpShutdownLifecycle::Draining);
    assert_eq!(snapshot.pending(), &[first]);
    assert_eq!(
        registry.register(12),
        Err(WindowsRdpRegistrationError::AdmissionClosed)
    );
}

#[test]
fn completion_is_idempotent() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let registration = registry.register(21).expect("registration");

    assert_eq!(
        registry.record_terminal(registration, WindowsRdpTerminalOutcome::Destroyed),
        WindowsRdpShutdownCompletion::Recorded
    );
    assert_eq!(
        registry.record_terminal(registration, WindowsRdpTerminalOutcome::Destroyed),
        WindowsRdpShutdownCompletion::AlreadyTerminal
    );
    assert_eq!(registry.active_count(), 0);
}

#[test]
fn stale_completion_cannot_finish_a_live_registration() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let registration = registry.register(31).expect("registration");
    let stale = WindowsRdpRegistration {
        token: registration.token,
        generation: registration.generation + 1,
    };

    assert_eq!(
        registry.record_terminal(stale, WindowsRdpTerminalOutcome::Destroyed),
        WindowsRdpShutdownCompletion::Stale
    );
    assert_eq!(registry.pending_registrations(), vec![registration]);
}

#[test]
fn unknown_token_with_same_generation_cannot_finish_a_live_registration() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let registration = registry.register(32).expect("registration");
    let stale = WindowsRdpRegistration {
        token: registration.token + 1,
        generation: registration.generation,
    };

    assert_eq!(
        registry.record_terminal(stale, WindowsRdpTerminalOutcome::Destroyed),
        WindowsRdpShutdownCompletion::Stale
    );
    assert_eq!(registry.pending_registrations(), vec![registration]);
}

#[test]
fn pending_callbacks_remain_live_until_a_terminal_outcome_is_recorded() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let registration = registry.register(51).expect("registration");

    let first = registry.begin_drain();
    let second = registry.begin_drain();

    assert_eq!(first.pending(), &[registration]);
    assert_eq!(second.pending(), &[registration]);
    assert_eq!(registry.lifecycle(), WindowsRdpShutdownLifecycle::Draining);
    assert!(registry.report().is_none());
}

#[test]
fn timeout_reports_complete_registrations_without_claiming_destroy() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let destroyed = registry.register(61).expect("destroyed registration");
    let leaked = registry.register(62).expect("leaked registration");
    registry.begin_drain();

    assert_eq!(
        registry.record_terminal(destroyed, WindowsRdpTerminalOutcome::Destroyed),
        WindowsRdpShutdownCompletion::Recorded
    );
    assert_eq!(
        registry.record_terminal(leaked, WindowsRdpTerminalOutcome::TimedOutLeaked),
        WindowsRdpShutdownCompletion::Recorded
    );

    let report = registry.report().expect("stable terminal report");
    assert!(report.timed_out());
    assert_eq!(report.requested(), 2);
    assert_eq!(report.destroyed(), 1);
    assert_eq!(report.timed_out_leaked(), 1);
    assert_eq!(report.timed_out_registrations(), &[leaked]);
    assert_eq!(registry.lifecycle(), WindowsRdpShutdownLifecycle::Drained);
}

#[test]
fn owner_loss_converges_the_drain_without_claiming_destroy_or_leak() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let lost = registry.register(63).expect("lost registration");
    registry.begin_drain();

    assert_eq!(
        registry.record_terminal(lost, WindowsRdpTerminalOutcome::OwnerLost),
        WindowsRdpShutdownCompletion::Recorded
    );

    let report = registry.report().expect("stable terminal report");
    assert!(report.incomplete());
    assert!(!report.timed_out());
    assert_eq!(report.requested(), 1);
    assert_eq!(report.destroyed(), 0);
    assert_eq!(report.timed_out_leaked(), 0);
    assert_eq!(report.owner_lost(), 1);
    assert_eq!(report.owner_lost_registrations(), &[lost]);
    assert_eq!(registry.lifecycle(), WindowsRdpShutdownLifecycle::Drained);
}

#[test]
fn empty_registry_drain_completes_immediately_and_is_stable() {
    let mut registry = WindowsRdpShutdownRegistry::new();

    let first_snapshot = registry.begin_drain();
    let first_report = registry.report().expect("empty drain report").clone();
    let second_snapshot = registry.begin_drain();
    let second_report = registry.report().expect("stable drain report");

    assert!(first_snapshot.pending().is_empty());
    assert!(second_snapshot.pending().is_empty());
    assert_eq!(registry.lifecycle(), WindowsRdpShutdownLifecycle::Drained);
    assert_eq!(first_report.requested(), 0);
    assert_eq!(first_report.destroyed(), 0);
    assert_eq!(first_report.timed_out_leaked(), 0);
    assert!(!first_report.timed_out());
    assert_eq!(&first_report, second_report);
}

#[test]
fn host_closed_before_drain_is_not_counted_as_shutdown_work() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let registration = registry.register(63).expect("registration");

    assert_eq!(
        registry.record_terminal(registration, WindowsRdpTerminalOutcome::Destroyed),
        WindowsRdpShutdownCompletion::Recorded
    );
    let snapshot = registry.begin_drain();
    let report = registry.report().expect("terminal report");

    assert!(snapshot.pending().is_empty());
    assert_eq!(report.requested(), 0);
    assert_eq!(report.destroyed(), 0);
    assert_eq!(report.timed_out_leaked(), 0);
}

#[test]
fn timeout_completion_is_idempotent_and_cannot_be_reclassified() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let registration = registry.register(64).expect("registration");
    registry.begin_drain();

    assert_eq!(
        registry.record_terminal(registration, WindowsRdpTerminalOutcome::TimedOutLeaked),
        WindowsRdpShutdownCompletion::Recorded
    );
    assert_eq!(
        registry.record_terminal(registration, WindowsRdpTerminalOutcome::TimedOutLeaked),
        WindowsRdpShutdownCompletion::AlreadyTerminal
    );
    assert_eq!(
        registry.record_terminal(registration, WindowsRdpTerminalOutcome::Destroyed),
        WindowsRdpShutdownCompletion::AlreadyTerminal
    );

    let report = registry.report().expect("terminal report");
    assert_eq!(report.requested(), 1);
    assert_eq!(report.destroyed(), 0);
    assert_eq!(report.timed_out_registrations(), &[registration]);
}

#[test]
fn repeated_drain_after_partial_completion_returns_only_remaining_work() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let first = registry.register(65).expect("first registration");
    let second = registry.register(66).expect("second registration");
    assert_eq!(registry.begin_drain().pending(), &[first, second]);

    registry.record_terminal(first, WindowsRdpTerminalOutcome::Destroyed);

    assert_eq!(registry.begin_drain().pending(), &[second]);
    assert_eq!(registry.lifecycle(), WindowsRdpShutdownLifecycle::Draining);
    assert!(registry.report().is_none());

    registry.record_terminal(second, WindowsRdpTerminalOutcome::Destroyed);
    let report = registry.report().expect("terminal report");
    assert_eq!(report.requested(), 2);
    assert_eq!(report.destroyed(), 2);
}

#[test]
fn timeout_report_order_is_stable_across_completion_timing() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let first = registry.register(67).expect("first registration");
    let second = registry.register(67).expect("second registration");
    registry.begin_drain();

    registry.record_terminal(second, WindowsRdpTerminalOutcome::TimedOutLeaked);
    registry.record_terminal(first, WindowsRdpTerminalOutcome::TimedOutLeaked);

    assert_eq!(
        registry
            .report()
            .expect("terminal report")
            .timed_out_registrations(),
        &[first, second]
    );
}

#[test]
fn repeated_shutdown_returns_the_same_stable_result() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let registration = registry.register(71).expect("registration");
    registry.begin_drain();
    registry.record_terminal(registration, WindowsRdpTerminalOutcome::Destroyed);

    let first = registry.report().expect("first report").clone();
    let snapshot = registry.begin_drain();
    let second = registry.report().expect("second report").clone();

    assert!(snapshot.pending().is_empty());
    assert_eq!(first, second);
    assert_eq!(
        registry.register(72),
        Err(WindowsRdpRegistrationError::AdmissionClosed)
    );
}

#[test]
fn fail_closed_report_preserves_progress_and_classifies_pending_as_owner_lost() {
    let mut registry = WindowsRdpShutdownRegistry::new();
    let destroyed = registry.register(73).expect("destroyed registration");
    let pending = registry.register(74).expect("pending registration");
    registry.begin_drain();
    registry.record_terminal(destroyed, WindowsRdpTerminalOutcome::Destroyed);

    let report = registry
        .fail_closed_report()
        .expect("draining registry must expose a fail-closed report");

    assert_eq!(report.requested(), 2);
    assert_eq!(report.destroyed(), 1);
    assert_eq!(report.timed_out_leaked(), 0);
    assert_eq!(report.owner_lost_registrations(), &[pending]);
    assert!(report.incomplete());
    assert!(registry.report().is_none());
}
