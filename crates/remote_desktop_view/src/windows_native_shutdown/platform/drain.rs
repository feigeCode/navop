use std::time::{Duration, Instant};

use gpui::{App, Task};
use windows_rdp_host::{WindowsRdpRegistration, WindowsRdpTerminalOutcome};

use super::{
    GlobalWindowsNativeRdpShutdown, WindowsNativeRdpOwner,
    record_windows_native_rdp_terminal_async, record_windows_native_rdp_view_owner_lost_async,
};
use crate::windows_native_shutdown::WindowsNativeRdpShutdownReport;

const WINDOWS_NATIVE_RDP_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const WINDOWS_NATIVE_RDP_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(16);

struct DrainStart {
    completed_report: Option<WindowsNativeRdpShutdownReport>,
    fail_closed_report: WindowsNativeRdpShutdownReport,
}

struct DrainSnapshot {
    pending: Vec<WindowsRdpRegistration>,
    owners: std::collections::BTreeMap<WindowsRdpRegistration, WindowsNativeRdpOwner>,
    deadline: Instant,
    completed_report: Option<WindowsNativeRdpShutdownReport>,
    fail_closed_report: WindowsNativeRdpShutdownReport,
}

fn begin_drain(cx: &mut App) -> DrainStart {
    cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
        let snapshot = controller.registry.begin_drain();
        controller
            .drain_deadline
            .get_or_insert_with(|| Instant::now() + WINDOWS_NATIVE_RDP_DRAIN_TIMEOUT);
        let completed_report = snapshot
            .pending()
            .is_empty()
            .then(|| controller.report())
            .flatten();
        let fail_closed_report = controller
            .registry
            .fail_closed_report()
            .expect("draining registry must expose a fail-closed report");
        DrainStart {
            completed_report,
            fail_closed_report: WindowsNativeRdpShutdownReport::from(&fail_closed_report),
        }
    })
}

fn read_snapshot(cx: &gpui::AsyncApp) -> Option<DrainSnapshot> {
    cx.try_read_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| DrainSnapshot {
        pending: controller.registry.pending_registrations(),
        owners: controller.owners.clone(),
        deadline: controller
            .drain_deadline
            .expect("draining controller must have a deadline"),
        completed_report: controller.report(),
        fail_closed_report: WindowsNativeRdpShutdownReport::from(
            &controller
                .registry
                .fail_closed_report()
                .expect("draining registry must expose a fail-closed report"),
        ),
    })
}

fn record_missing_owner(
    registration: WindowsRdpRegistration,
    deadline_elapsed: bool,
    cx: &gpui::AsyncApp,
) {
    if deadline_elapsed {
        tracing::error!(
            token = registration.token(),
            generation = registration.generation(),
            "Windows native RDP shutdown registration has no owner; \
             completing the bounded drain as owner-lost"
        );
        record_windows_native_rdp_terminal_async(
            registration,
            WindowsRdpTerminalOutcome::OwnerLost,
            cx,
        );
    } else {
        tracing::warn!(
            token = registration.token(),
            generation = registration.generation(),
            "Windows native RDP shutdown registration has no owner"
        );
    }
}

fn record_stalled_detached_owner(
    registration: WindowsRdpRegistration,
    deadline_elapsed: bool,
    cx: &gpui::AsyncApp,
) {
    if !deadline_elapsed {
        return;
    }
    tracing::error!(
        token = registration.token(),
        generation = registration.generation(),
        "Windows native RDP detached cleanup did not report a terminal outcome before the \
         deadline; completing the bounded drain as owner-lost"
    );
    record_windows_native_rdp_terminal_async(
        registration,
        WindowsRdpTerminalOutcome::OwnerLost,
        cx,
    );
}

fn poll_view_owner(
    owner: &gpui::WeakEntity<crate::view::RemoteDesktopView>,
    registration: WindowsRdpRegistration,
    deadline_elapsed: bool,
    cx: &gpui::AsyncApp,
) {
    let result = owner.update(cx, |view, cx| {
        if deadline_elapsed {
            view.quarantine_windows_native_for_shutdown(registration, cx)
        } else {
            view.force_close_windows_native_for_shutdown(registration, cx);
            true
        }
    });
    match result {
        Ok(true) => {}
        Ok(false) => record_mismatched_view_owner(registration, deadline_elapsed, cx),
        Err(error) => {
            tracing::debug!(
                ?error,
                token = registration.token(),
                generation = registration.generation(),
                "Windows native RDP view released while shutdown drain was polling"
            );
            record_released_view_owner(registration, deadline_elapsed, cx);
        }
    }
}

fn record_mismatched_view_owner(
    registration: WindowsRdpRegistration,
    deadline_elapsed: bool,
    cx: &gpui::AsyncApp,
) {
    if !deadline_elapsed {
        return;
    }
    tracing::error!(
        token = registration.token(),
        generation = registration.generation(),
        "Windows native RDP view no longer owns its registered adapter at the drain deadline"
    );
    record_windows_native_rdp_view_owner_lost_async(registration, cx);
}

fn record_released_view_owner(
    registration: WindowsRdpRegistration,
    deadline_elapsed: bool,
    cx: &gpui::AsyncApp,
) {
    if deadline_elapsed {
        tracing::error!(
            token = registration.token(),
            generation = registration.generation(),
            "Windows native RDP view released before the drain deadline completed"
        );
        record_windows_native_rdp_view_owner_lost_async(registration, cx);
    }
}

fn poll_registration(
    registration: WindowsRdpRegistration,
    owner: Option<&WindowsNativeRdpOwner>,
    deadline_elapsed: bool,
    cx: &gpui::AsyncApp,
) {
    match owner {
        None => record_missing_owner(registration, deadline_elapsed, cx),
        Some(WindowsNativeRdpOwner::Detached) => {
            record_stalled_detached_owner(registration, deadline_elapsed, cx);
        }
        Some(WindowsNativeRdpOwner::View(owner)) => {
            poll_view_owner(owner, registration, deadline_elapsed, cx);
        }
    }
}

fn completed_report(cx: &gpui::AsyncApp) -> Option<WindowsNativeRdpShutdownReport> {
    cx.try_read_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| controller.report())
        .flatten()
}

async fn drain(
    cx: &gpui::AsyncApp,
    mut fail_closed_report: WindowsNativeRdpShutdownReport,
) -> WindowsNativeRdpShutdownReport {
    loop {
        let Some(snapshot) = read_snapshot(cx) else {
            tracing::error!("Windows native RDP shutdown controller disappeared during drain");
            return fail_closed_report;
        };
        fail_closed_report = snapshot.fail_closed_report;
        if let Some(report) = snapshot.completed_report {
            return report;
        }

        let deadline_elapsed = Instant::now() >= snapshot.deadline;
        for registration in snapshot.pending {
            poll_registration(
                registration,
                snapshot.owners.get(&registration),
                deadline_elapsed,
                cx,
            );
        }
        if let Some(report) = completed_report(cx) {
            return report;
        }
        cx.background_executor()
            .timer(WINDOWS_NATIVE_RDP_DRAIN_POLL_INTERVAL)
            .await;
    }
}

pub fn shutdown_windows_native_rdp(cx: &mut App) -> Task<WindowsNativeRdpShutdownReport> {
    if !cx.has_global::<GlobalWindowsNativeRdpShutdown>() {
        tracing::error!("Windows native RDP shutdown controller is unavailable");
        return Task::ready(WindowsNativeRdpShutdownReport::unavailable_controller());
    }
    let start = begin_drain(cx);
    if let Some(report) = start.completed_report {
        return Task::ready(report);
    }
    cx.spawn(async move |cx| drain(cx, start.fail_closed_report).await)
}
