use std::fmt;

/// The presentation state of the current session's framebuffer.
///
/// A dirty-rectangle update is meaningful only after a complete base frame
/// for the same dimensions has been accepted. Once that invariant is broken,
/// all subsequent deltas are ignored until another complete frame establishes
/// a new synchronization point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum FrameSyncPhase {
    #[default]
    AwaitingBase,
    Ready,
    Recovering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeltaDisposition {
    Applied,
    Rejected { recovery_started: bool },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct FrameSyncSnapshot {
    pub(super) session_generation: u64,
    pub(super) phase: FrameSyncPhase,
    pub(super) base_size: Option<(u16, u16)>,
    pub(super) full_frames: u64,
    pub(super) deltas: u64,
    pub(super) dropped_deltas: u64,
    pub(super) recoveries: u64,
}

/// Small, protocol-neutral state machine for base/delta presentation.
///
/// This deliberately does not own pixel data. The view owns the framebuffer
/// and commits a delta only after validating and applying it atomically. The
/// tracker records the predecessor relationship and exposes bounded
/// diagnostics without making the GPUI render tree depend on frame pixels.
pub(super) struct FrameSyncTracker {
    snapshot: FrameSyncSnapshot,
}

impl Default for FrameSyncTracker {
    fn default() -> Self {
        Self {
            snapshot: FrameSyncSnapshot::default(),
        }
    }
}

impl fmt::Debug for FrameSyncTracker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.snapshot.fmt(formatter)
    }
}

impl FrameSyncTracker {
    pub(super) fn reset_session(&mut self) {
        self.snapshot.session_generation = self.snapshot.session_generation.wrapping_add(1);
        self.snapshot.phase = FrameSyncPhase::AwaitingBase;
        self.snapshot.base_size = None;
        self.snapshot.full_frames = 0;
        self.snapshot.deltas = 0;
        self.snapshot.dropped_deltas = 0;
        self.snapshot.recoveries = 0;
    }

    pub(super) fn connected(&mut self) {
        // A Connected event negotiates the session, but it is not itself a
        // base frame. Keep waiting for a complete frame even if the helper
        // reports its dimensions here.
        self.snapshot.phase = FrameSyncPhase::AwaitingBase;
        self.snapshot.base_size = None;
    }

    pub(super) fn accept_base(&mut self, size: (u16, u16)) {
        self.snapshot.phase = FrameSyncPhase::Ready;
        self.snapshot.base_size = Some(size);
        self.snapshot.full_frames = self.snapshot.full_frames.saturating_add(1);
    }

    pub(super) fn can_apply_delta(&self, size: (u16, u16)) -> bool {
        self.snapshot.phase == FrameSyncPhase::Ready && self.snapshot.base_size == Some(size)
    }

    /// Commit a delta that has already been validated and patched into a
    /// temporary framebuffer.
    pub(super) fn accept_delta(&mut self, size: (u16, u16)) -> DeltaDisposition {
        if self.can_apply_delta(size) {
            self.snapshot.deltas = self.snapshot.deltas.saturating_add(1);
            return DeltaDisposition::Applied;
        }

        self.reject_delta()
    }

    /// Record a delta that could not be safely applied.
    pub(super) fn reject_delta(&mut self) -> DeltaDisposition {
        self.snapshot.dropped_deltas = self.snapshot.dropped_deltas.saturating_add(1);
        let recovery_started = self.snapshot.phase != FrameSyncPhase::Recovering;
        if recovery_started {
            self.snapshot.recoveries = self.snapshot.recoveries.saturating_add(1);
        }
        self.snapshot.phase = FrameSyncPhase::Recovering;
        DeltaDisposition::Rejected { recovery_started }
    }

    pub(super) fn snapshot(&self) -> FrameSyncSnapshot {
        self.snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::{DeltaDisposition, FrameSyncPhase, FrameSyncTracker};

    #[test]
    fn session_starts_awaiting_a_base_frame() {
        let mut tracker = FrameSyncTracker::default();

        tracker.reset_session();

        let snapshot = tracker.snapshot();
        assert_eq!(1, snapshot.session_generation);
        assert_eq!(FrameSyncPhase::AwaitingBase, snapshot.phase);
        assert_eq!(None, snapshot.base_size);
    }

    #[test]
    fn connected_is_not_a_base_frame() {
        let mut tracker = FrameSyncTracker::default();
        tracker.reset_session();
        tracker.connected();

        assert_eq!(FrameSyncPhase::AwaitingBase, tracker.snapshot().phase);
        assert_eq!(
            DeltaDisposition::Rejected {
                recovery_started: true
            },
            tracker.accept_delta((1280, 720))
        );
    }

    #[test]
    fn matching_base_and_delta_are_accepted() {
        let mut tracker = FrameSyncTracker::default();
        tracker.reset_session();
        tracker.accept_base((1280, 720));

        assert!(tracker.can_apply_delta((1280, 720)));
        assert!(!tracker.can_apply_delta((1024, 768)));
        assert_eq!(DeltaDisposition::Applied, tracker.accept_delta((1280, 720)));
        let snapshot = tracker.snapshot();
        assert_eq!(FrameSyncPhase::Ready, snapshot.phase);
        assert_eq!(1, snapshot.full_frames);
        assert_eq!(1, snapshot.deltas);
    }

    #[test]
    fn delta_preflight_rejects_unsynchronized_and_recovering_sessions() {
        let mut tracker = FrameSyncTracker::default();
        tracker.reset_session();
        assert!(!tracker.can_apply_delta((1280, 720)));

        tracker.accept_base((1280, 720));
        assert!(tracker.can_apply_delta((1280, 720)));
        assert!(matches!(
            tracker.reject_delta(),
            DeltaDisposition::Rejected { .. }
        ));
        assert!(!tracker.can_apply_delta((1280, 720)));

        tracker.accept_base((1280, 720));
        assert!(tracker.can_apply_delta((1280, 720)));
    }

    #[test]
    fn mismatched_or_invalid_delta_starts_one_recovery_epoch() {
        let mut tracker = FrameSyncTracker::default();
        tracker.reset_session();
        tracker.accept_base((1280, 720));

        assert_eq!(
            DeltaDisposition::Rejected {
                recovery_started: true
            },
            tracker.accept_delta((1024, 768))
        );
        assert_eq!(
            DeltaDisposition::Rejected {
                recovery_started: false
            },
            tracker.accept_delta((1280, 720))
        );
        assert_eq!(
            DeltaDisposition::Rejected {
                recovery_started: false
            },
            tracker.reject_delta()
        );

        let snapshot = tracker.snapshot();
        assert_eq!(FrameSyncPhase::Recovering, snapshot.phase);
        assert_eq!(1, snapshot.recoveries);
        assert_eq!(3, snapshot.dropped_deltas);
    }

    #[test]
    fn a_new_base_recovers_and_accepts_following_deltas() {
        let mut tracker = FrameSyncTracker::default();
        tracker.reset_session();
        tracker.accept_base((1280, 720));
        assert!(matches!(
            tracker.accept_delta((640, 480)),
            DeltaDisposition::Rejected { .. }
        ));

        tracker.accept_base((640, 480));
        assert_eq!(FrameSyncPhase::Ready, tracker.snapshot().phase);
        assert_eq!(DeltaDisposition::Applied, tracker.accept_delta((640, 480)));
        assert_eq!(2, tracker.snapshot().full_frames);
    }
}
