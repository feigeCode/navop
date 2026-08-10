use std::collections::BTreeMap;

mod report;

pub use report::{
    WindowsRdpDrainSnapshot, WindowsRdpRegistration, WindowsRdpRegistrationError,
    WindowsRdpShutdownCompletion, WindowsRdpShutdownLifecycle, WindowsRdpShutdownReport,
    WindowsRdpTerminalOutcome,
};

#[derive(Debug)]
struct DrainProgress {
    requested: usize,
    destroyed: usize,
    timed_out_registrations: Vec<WindowsRdpRegistration>,
    owner_lost_registrations: Vec<WindowsRdpRegistration>,
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
                owner_lost_registrations: Vec::new(),
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
                WindowsRdpTerminalOutcome::OwnerLost => {
                    drain.owner_lost_registrations.push(registration);
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

    /// Build a conservative report for an interrupted drain.
    ///
    /// Registrations that have not reached a terminal outcome are classified as
    /// owner-lost without mutating the registry. This preserves completed
    /// progress while ensuring controller loss cannot look like an empty,
    /// successful shutdown.
    pub fn fail_closed_report(&self) -> Option<WindowsRdpShutdownReport> {
        if let Some(report) = self.report.as_ref() {
            return Some(report.clone());
        }

        let drain = self.drain.as_ref()?;
        let mut timed_out_registrations = drain.timed_out_registrations.clone();
        let mut owner_lost_registrations = drain.owner_lost_registrations.clone();
        owner_lost_registrations.extend(self.active.values().copied());
        timed_out_registrations.sort_unstable();
        owner_lost_registrations.sort_unstable();
        Some(WindowsRdpShutdownReport {
            requested: drain.requested,
            destroyed: drain.destroyed,
            timed_out_registrations,
            owner_lost_registrations,
        })
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
        drain.owner_lost_registrations.sort_unstable();
        debug_assert_eq!(
            drain.requested,
            drain.destroyed
                + drain.timed_out_registrations.len()
                + drain.owner_lost_registrations.len(),
            "every drain registration must have one terminal outcome"
        );
        self.report = Some(WindowsRdpShutdownReport {
            requested: drain.requested,
            destroyed: drain.destroyed,
            timed_out_registrations: drain.timed_out_registrations,
            owner_lost_registrations: drain.owner_lost_registrations,
        });
        self.lifecycle = WindowsRdpShutdownLifecycle::Drained;
    }
}

#[cfg(test)]
mod tests;
