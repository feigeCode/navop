mod drain;

use std::collections::BTreeMap;
use std::time::Instant;

use gpui::{App, BorrowAppContext, Context, Global, WeakEntity};
use windows_rdp_host::{
    WindowsRdpRegistration, WindowsRdpRegistrationError, WindowsRdpShutdownCompletion,
    WindowsRdpShutdownRegistry, WindowsRdpTerminalOutcome,
};

use super::WindowsNativeRdpShutdownReport;
use crate::view::RemoteDesktopView;

#[derive(Clone, Debug)]
pub(super) enum WindowsNativeRdpOwner {
    View(WeakEntity<RemoteDesktopView>),
    Detached,
}

#[derive(Debug)]
pub(super) struct GlobalWindowsNativeRdpShutdown {
    pub(super) registry: WindowsRdpShutdownRegistry,
    pub(super) owners: BTreeMap<WindowsRdpRegistration, WindowsNativeRdpOwner>,
    pub(super) drain_deadline: Option<Instant>,
}

impl Global for GlobalWindowsNativeRdpShutdown {}

impl GlobalWindowsNativeRdpShutdown {
    fn new() -> Self {
        Self {
            registry: WindowsRdpShutdownRegistry::new(),
            owners: BTreeMap::new(),
            drain_deadline: None,
        }
    }

    pub(super) fn report(&self) -> Option<WindowsNativeRdpShutdownReport> {
        self.registry
            .report()
            .map(WindowsNativeRdpShutdownReport::from)
    }
}

impl From<&windows_rdp_host::WindowsRdpShutdownReport> for WindowsNativeRdpShutdownReport {
    fn from(report: &windows_rdp_host::WindowsRdpShutdownReport) -> Self {
        Self {
            requested: report.requested(),
            destroyed: report.destroyed(),
            timed_out_leaked: report.timed_out_leaked(),
            owner_lost: report.owner_lost(),
            controller_unavailable: false,
        }
    }
}

pub(crate) fn init(cx: &mut App) {
    if !cx.has_global::<GlobalWindowsNativeRdpShutdown>() {
        cx.set_global(GlobalWindowsNativeRdpShutdown::new());
    }
}

pub(crate) fn register_windows_native_rdp(
    owner: WeakEntity<RemoteDesktopView>,
    generation: u64,
    cx: &mut Context<RemoteDesktopView>,
) -> Result<WindowsRdpRegistration, WindowsRdpRegistrationError> {
    cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
        let registration = controller.registry.register(generation)?;
        let replaced = controller
            .owners
            .insert(registration, WindowsNativeRdpOwner::View(owner));
        debug_assert!(replaced.is_none(), "registration owners must be unique");
        Ok(registration)
    })
}

pub(crate) fn mark_windows_native_rdp_detached<C>(registration: WindowsRdpRegistration, cx: &mut C)
where
    C: BorrowAppContext,
{
    cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
        if let Some(owner) = controller.owners.get_mut(&registration) {
            *owner = WindowsNativeRdpOwner::Detached;
        } else {
            tracing::warn!(
                token = registration.token(),
                generation = registration.generation(),
                "Windows native RDP registration lost before detached cleanup"
            );
        }
    });
}

fn record_terminal(
    controller: &mut GlobalWindowsNativeRdpShutdown,
    registration: WindowsRdpRegistration,
    outcome: WindowsRdpTerminalOutcome,
) {
    match controller.registry.record_terminal(registration, outcome) {
        WindowsRdpShutdownCompletion::Recorded | WindowsRdpShutdownCompletion::AlreadyTerminal => {
            controller.owners.remove(&registration);
        }
        WindowsRdpShutdownCompletion::Stale => {
            tracing::warn!(
                token = registration.token(),
                generation = registration.generation(),
                ?outcome,
                "ignored stale Windows native RDP terminal completion"
            );
        }
    }
}

pub(crate) fn record_windows_native_rdp_terminal<C>(
    registration: WindowsRdpRegistration,
    outcome: WindowsRdpTerminalOutcome,
    cx: &mut C,
) where
    C: BorrowAppContext,
{
    cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
        record_terminal(controller, registration, outcome);
    });
}

pub(crate) fn detached_cleanup_deadline(local_deadline: Instant, cx: &gpui::AsyncApp) -> Instant {
    cx.try_read_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
        controller
            .drain_deadline
            .map(|deadline| deadline.min(local_deadline))
            .unwrap_or(local_deadline)
    })
    .unwrap_or(local_deadline)
}

pub(crate) fn record_windows_native_rdp_terminal_async(
    registration: WindowsRdpRegistration,
    outcome: WindowsRdpTerminalOutcome,
    cx: &gpui::AsyncApp,
) {
    let result = cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
        record_terminal(controller, registration, outcome);
    });
    if result.is_err() {
        tracing::error!(
            token = registration.token(),
            generation = registration.generation(),
            ?outcome,
            "application released before Windows native RDP terminal completion was recorded"
        );
    }
}

pub(super) fn record_windows_native_rdp_view_owner_lost_async(
    registration: WindowsRdpRegistration,
    cx: &gpui::AsyncApp,
) {
    let result = cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
        if matches!(
            controller.owners.get(&registration),
            Some(WindowsNativeRdpOwner::View(_))
        ) {
            record_terminal(
                controller,
                registration,
                WindowsRdpTerminalOutcome::OwnerLost,
            );
        }
    });
    if result.is_err() {
        tracing::error!(
            token = registration.token(),
            generation = registration.generation(),
            "application released before Windows native RDP view owner loss was recorded"
        );
    }
}

pub use drain::shutdown_windows_native_rdp;
