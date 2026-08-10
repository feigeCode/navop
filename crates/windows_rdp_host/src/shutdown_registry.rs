use std::collections::BTreeMap;

/// Application-level admission lifecycle for Windows native RDP hosts.
///
/// This state machine owns only scalar registration metadata. Native hosts,
/// parent windows, GPUI entities, and COM objects remain owned and operated by
/// their foreground/UI-thread controller.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WindowsRdpShutdownLifecycle {
    /// New native host registrations are accepted.
    #[default]
    Running,
    /// Admission is closed while existing registrations converge.
    Draining,
    /// Every registration in the drain snapshot reached a terminal outcome.
    Drained,
}

/// Opaque identity for one native host registration.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WindowsRdpRegistration {
    token: u64,
    generation: u64,
}

impl WindowsRdpRegistration {
    /// Return the unique application-level registration token.
    pub const fn token(self) -> u64 {
        self.token
    }

    /// Return the native host generation bound to this registration.
    pub const fn generation(self) -> u64 {
        self.generation
    }
}

/// Error returned when a native host registration cannot be admitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsRdpRegistrationError {
    /// Shutdown already began, so the caller must not create or attach a host.
    AdmissionClosed,
    /// The process-lifetime registration token space was exhausted.
    TokenExhausted,
}

/// Confirmed terminal result for one native host registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsRdpTerminalOutcome {
    /// Callback unregistration and native host destruction completed.
    Destroyed,
    /// The complete owner-thread adapter was deliberately leaked after timeout.
    TimedOutLeaked,
}

/// Result of attempting to record a terminal registration outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsRdpShutdownCompletion {
    /// The live registration reached its first terminal outcome.
    Recorded,
    /// The exact registration had already reached a terminal outcome.
    AlreadyTerminal,
    /// The token or generation did not identify the current registration.
    Stale,
}

/// Foreground close work captured when shutdown closes admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowsRdpDrainSnapshot {
    pending: Vec<WindowsRdpRegistration>,
}

impl WindowsRdpDrainSnapshot {
    /// Registrations that still require owner-thread cleanup.
    pub fn pending(&self) -> &[WindowsRdpRegistration] {
        &self.pending
    }
}

/// Stable, native-pointer-free result of the application RDP drain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowsRdpShutdownReport {
    requested: usize,
    destroyed: usize,
    timed_out_registrations: Vec<WindowsRdpRegistration>,
}

impl WindowsRdpShutdownReport {
    /// Whether at least one complete adapter had to be leaked at the deadline.
    pub const fn timed_out(&self) -> bool {
        !self.timed_out_registrations.is_empty()
    }

    /// Number of live registrations captured when admission closed.
    pub const fn requested(&self) -> usize {
        self.requested
    }

    /// Number of captured registrations confirmed destroyed.
    pub const fn destroyed(&self) -> usize {
        self.destroyed
    }

    /// Number of captured registrations deliberately leaked after timeout.
    pub const fn timed_out_leaked(&self) -> usize {
        self.timed_out_registrations.len()
    }

    /// Complete registrations deliberately leaked after timeout.
    pub fn timed_out_registrations(&self) -> &[WindowsRdpRegistration] {
        &self.timed_out_registrations
    }
}

#[derive(Debug)]
struct DrainProgress {
    requested: usize,
    destroyed: usize,
    timed_out_registrations: Vec<WindowsRdpRegistration>,
}

/// Pure registration and shutdown-admission state for Windows native RDP.
///
/// The registry intentionally does not own `WindowsRdpHost` or any UI object.
/// Callers must perform close/retry/leak actions on the host's owner thread,
/// then record exactly one terminal outcome with the returned registration.
///
/// Terminal registrations are retained as process-lifetime scalar tombstones
/// so duplicate completions remain idempotent and stale generations cannot
/// complete a newer registration. They contain no native pointer, credential,
/// endpoint, or UI object.
#[derive(Debug, Default)]
pub struct WindowsRdpShutdownRegistry {
    lifecycle: WindowsRdpShutdownLifecycle,
    next_token: u64,
    active: BTreeMap<u64, WindowsRdpRegistration>,
    terminal: BTreeMap<u64, WindowsRdpRegistration>,
    drain: Option<DrainProgress>,
    report: Option<WindowsRdpShutdownReport>,
}

impl WindowsRdpShutdownRegistry {
    /// Construct an empty registry with admission open.
    pub fn new() -> Self {
        Self {
            next_token: 1,
            ..Self::default()
        }
    }

    /// Return the current admission/drain lifecycle.
    pub const fn lifecycle(&self) -> WindowsRdpShutdownLifecycle {
        self.lifecycle
    }

    /// Register one native host generation while admission remains open.
    pub fn register(
        &mut self,
        generation: u64,
    ) -> Result<WindowsRdpRegistration, WindowsRdpRegistrationError> {
        if self.lifecycle != WindowsRdpShutdownLifecycle::Running {
            return Err(WindowsRdpRegistrationError::AdmissionClosed);
        }

        let Some(next_token) = self.next_token.checked_add(1) else {
            return Err(WindowsRdpRegistrationError::TokenExhausted);
        };
        let registration = WindowsRdpRegistration {
            token: self.next_token,
            generation,
        };
        self.next_token = next_token;
        let replaced = self.active.insert(registration.token, registration);
        debug_assert!(replaced.is_none(), "registration tokens must be unique");
        Ok(registration)
    }

    /// Close admission and return the registrations still requiring cleanup.
    ///
    /// Repeated calls are idempotent. While draining they return the remaining
    /// registrations; after convergence they return an empty snapshot.
    pub fn begin_drain(&mut self) -> WindowsRdpDrainSnapshot {
        if self.lifecycle == WindowsRdpShutdownLifecycle::Running {
            self.lifecycle = WindowsRdpShutdownLifecycle::Draining;
            self.drain = Some(DrainProgress {
                requested: self.active.len(),
                destroyed: 0,
                timed_out_registrations: Vec::new(),
            });
            self.finish_drain_if_ready();
        }

        WindowsRdpDrainSnapshot {
            pending: self.pending_registrations(),
        }
    }

    /// Return all currently live registrations in token order.
    pub fn pending_registrations(&self) -> Vec<WindowsRdpRegistration> {
        self.active.values().copied().collect()
    }

    /// Return the number of registrations without a terminal outcome.
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// Record a confirmed destroy or a deliberate timeout leak.
    pub fn record_terminal(
        &mut self,
        registration: WindowsRdpRegistration,
        outcome: WindowsRdpTerminalOutcome,
    ) -> WindowsRdpShutdownCompletion {
        if let Some(terminal) = self.terminal.get(&registration.token) {
            return if terminal == &registration {
                WindowsRdpShutdownCompletion::AlreadyTerminal
            } else {
                WindowsRdpShutdownCompletion::Stale
            };
        }

        let Some(active) = self.active.get(&registration.token) else {
            return WindowsRdpShutdownCompletion::Stale;
        };
        if active != &registration {
            return WindowsRdpShutdownCompletion::Stale;
        }

        self.active.remove(&registration.token);
        self.terminal.insert(registration.token, registration);

        if let Some(drain) = self.drain.as_mut() {
            match outcome {
                WindowsRdpTerminalOutcome::Destroyed => drain.destroyed += 1,
                WindowsRdpTerminalOutcome::TimedOutLeaked => {
                    drain.timed_out_registrations.push(registration);
                }
            }
        }
        self.finish_drain_if_ready();
        WindowsRdpShutdownCompletion::Recorded
    }

    /// Return the stable report after the drain converges.
    pub fn report(&self) -> Option<&WindowsRdpShutdownReport> {
        self.report.as_ref()
    }

    fn finish_drain_if_ready(&mut self) {
        if self.lifecycle != WindowsRdpShutdownLifecycle::Draining || !self.active.is_empty() {
            return;
        }

        let mut drain = self
            .drain
            .take()
            .expect("draining lifecycle must own progress");
        drain.timed_out_registrations.sort_unstable();
        debug_assert_eq!(
            drain.requested,
            drain.destroyed + drain.timed_out_registrations.len(),
            "every drain registration must have one terminal outcome"
        );
        self.report = Some(WindowsRdpShutdownReport {
            requested: drain.requested,
            destroyed: drain.destroyed,
            timed_out_registrations: drain.timed_out_registrations,
        });
        self.lifecycle = WindowsRdpShutdownLifecycle::Drained;
    }
}

#[cfg(test)]
mod tests {
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
}
