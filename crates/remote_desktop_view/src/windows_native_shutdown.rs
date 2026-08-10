use gpui::{App, Task};

/// Stable, native-resource-free result of the application RDP shutdown drain.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WindowsNativeRdpShutdownReport {
    requested: usize,
    destroyed: usize,
    timed_out_leaked: usize,
    owner_lost: usize,
    controller_unavailable: bool,
}

impl WindowsNativeRdpShutdownReport {
    #[cfg(any(test, all(feature = "windows-native-rdp", target_os = "windows")))]
    pub(crate) const fn unavailable_controller() -> Self {
        Self {
            requested: 0,
            destroyed: 0,
            timed_out_leaked: 0,
            owner_lost: 0,
            controller_unavailable: true,
        }
    }

    /// Number of registrations captured when shutdown closed admission.
    pub const fn requested(&self) -> usize {
        self.requested
    }

    /// Number of registrations confirmed destroyed on their owner thread.
    pub const fn destroyed(&self) -> usize {
        self.destroyed
    }

    /// Number of complete adapters deliberately leaked after the deadline.
    pub const fn timed_out_leaked(&self) -> usize {
        self.timed_out_leaked
    }

    /// Number of registrations whose owner-thread completion became unavailable.
    pub const fn owner_lost(&self) -> usize {
        self.owner_lost
    }

    /// Whether the application-owned shutdown controller was unavailable.
    pub const fn controller_unavailable(&self) -> bool {
        self.controller_unavailable
    }

    /// Whether the drain did not confirm destruction for every registration.
    pub const fn incomplete(&self) -> bool {
        self.timed_out_leaked != 0 || self.owner_lost != 0 || self.controller_unavailable
    }

    /// Whether the drain required at least one deliberate timeout leak.
    pub const fn timed_out(&self) -> bool {
        self.timed_out_leaked != 0
    }
}

#[cfg(test)]
mod tests {
    use super::WindowsNativeRdpShutdownReport;

    #[test]
    fn unavailable_controller_is_reported_as_incomplete_without_inventing_a_registration() {
        let report = WindowsNativeRdpShutdownReport::unavailable_controller();

        assert_eq!(report.requested(), 0);
        assert_eq!(report.destroyed(), 0);
        assert_eq!(report.timed_out_leaked(), 0);
        assert_eq!(report.owner_lost(), 0);
        assert!(report.controller_unavailable());
        assert!(report.incomplete());
    }
}

#[cfg(not(all(feature = "windows-native-rdp", target_os = "windows")))]
pub(crate) fn init(_cx: &mut App) {}

#[cfg(not(all(feature = "windows-native-rdp", target_os = "windows")))]
pub fn shutdown_windows_native_rdp(_cx: &mut App) -> Task<WindowsNativeRdpShutdownReport> {
    Task::ready(WindowsNativeRdpShutdownReport::default())
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
mod platform;

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
pub use platform::shutdown_windows_native_rdp;
#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
pub(crate) use platform::{
    detached_cleanup_deadline, init, mark_windows_native_rdp_detached,
    record_windows_native_rdp_terminal, record_windows_native_rdp_terminal_async,
    register_windows_native_rdp,
};
