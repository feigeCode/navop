mod drain;

use std::collections::BTreeMap;
use std::time::Instant;

use gpui::{App, BorrowAppContext, Context, Global, WeakEntity};
use windows_rdp_host::{
    WindowsRdpRegistration, WindowsRdpRegistrationError, WindowsRdpShutdownCompletion,
    WindowsRdpShutdownRegistry, WindowsRdpTerminalOutcome,
};

use super::{WindowsNativeRdpShutdownReport, WindowsNativeRdpTerminalDispatch};
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

pub(crate) fn mark_windows_native_rdp_detached_async(
    registration: WindowsRdpRegistration,
    cx: &gpui::AsyncApp,
) -> WindowsNativeRdpTerminalDispatch {
    let result = try_update_windows_native_rdp_shutdown(cx, |controller, _| {
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
    if result.is_none() {
        tracing::error!(
            token = registration.token(),
            generation = registration.generation(),
            "Windows native RDP detached dispatcher rejected ownership transfer"
        );
    }
    WindowsNativeRdpTerminalDispatch::from_option(result)
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

fn try_update_windows_native_rdp_shutdown<R>(
    cx: &gpui::AsyncApp,
    update: impl FnOnce(&mut GlobalWindowsNativeRdpShutdown, &mut App) -> R,
) -> Option<R> {
    cx.try_read_global::<GlobalWindowsNativeRdpShutdown, _>(|_, _| ())?;
    Some(cx.update_global(update))
}

pub(crate) fn record_windows_native_rdp_terminal_async(
    registration: WindowsRdpRegistration,
    outcome: WindowsRdpTerminalOutcome,
    cx: &gpui::AsyncApp,
) -> WindowsNativeRdpTerminalDispatch {
    let result = try_update_windows_native_rdp_shutdown(cx, |controller, _| {
        record_terminal(controller, registration, outcome);
    });
    if result.is_none() {
        tracing::error!(
            token = registration.token(),
            generation = registration.generation(),
            ?outcome,
            "Windows native RDP terminal dispatcher rejected the completion"
        );
    }
    WindowsNativeRdpTerminalDispatch::from_option(result)
}

pub(super) fn record_windows_native_rdp_view_owner_lost_async(
    registration: WindowsRdpRegistration,
    cx: &gpui::AsyncApp,
) -> WindowsNativeRdpTerminalDispatch {
    let result = try_update_windows_native_rdp_shutdown(cx, |controller, _| {
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
    if result.is_none() {
        tracing::error!(
            token = registration.token(),
            generation = registration.generation(),
            "Windows native RDP terminal dispatcher rejected view-owner loss"
        );
    }
    WindowsNativeRdpTerminalDispatch::from_option(result)
}

pub use drain::{fail_closed_windows_native_rdp_for_platform_quit, shutdown_windows_native_rdp};
