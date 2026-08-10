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
    pub(super) token: u64,
    pub(super) generation: u64,
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
    /// Adapter ownership or its owner-thread completion path became unavailable
    /// before destruction or deliberate quarantine was confirmed.
    ///
    /// This is an invariant-failure terminal state: waiting longer cannot recover
    /// an owner-thread cleanup path, so the drain converges while reporting
    /// incomplete cleanup instead of hanging application exit indefinitely.
    OwnerLost,
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
    pub(super) pending: Vec<WindowsRdpRegistration>,
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
    pub(super) requested: usize,
    pub(super) destroyed: usize,
    pub(super) timed_out_registrations: Vec<WindowsRdpRegistration>,
    pub(super) owner_lost_registrations: Vec<WindowsRdpRegistration>,
}

impl WindowsRdpShutdownReport {
    /// Whether at least one registration did not confirm native destruction.
    pub const fn incomplete(&self) -> bool {
        self.timed_out() || !self.owner_lost_registrations.is_empty()
    }

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

    /// Number of registrations whose owner-thread completion became unavailable.
    pub const fn owner_lost(&self) -> usize {
        self.owner_lost_registrations.len()
    }

    /// Registrations whose owner-thread completion became unavailable.
    pub fn owner_lost_registrations(&self) -> &[WindowsRdpRegistration] {
        &self.owner_lost_registrations
    }
}
