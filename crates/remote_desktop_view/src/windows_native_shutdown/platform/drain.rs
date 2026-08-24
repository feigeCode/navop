use std::time::{Duration, Instant};

use gpui::{App, BorrowAppContext, Task};
use windows_rdp_host::{WindowsRdpRegistration, WindowsRdpTerminalOutcome};

use super::{
    GlobalWindowsNativeRdpShutdown, WindowsNativeRdpOwner,
    record_windows_native_rdp_terminal_async, record_windows_native_rdp_view_owner_lost_async,
};
use crate::windows_native_shutdown::{
    WindowsNativeRdpShutdownReport, WindowsNativeRdpTerminalDispatch,
};

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
) -> WindowsNativeRdpTerminalDispatch {
    if deadline_elapsed {
        tracing::error!(
            token = registration.token(),
            generation = registration.generation(),
            "Windows native RDP shutdown registration has no owner; \
             completing the bounded drain as owner-lost"
        );
        return record_windows_native_rdp_terminal_async(
            registration,
            WindowsRdpTerminalOutcome::OwnerLost,
            cx,
        );
    }
    tracing::warn!(
        token = registration.token(),
        generation = registration.generation(),
        "Windows native RDP shutdown registration has no owner"
    );
    WindowsNativeRdpTerminalDispatch::Delivered
}

fn record_stalled_detached_owner(
    registration: WindowsRdpRegistration,
    deadline_elapsed: bool,
    cx: &gpui::AsyncApp,
) -> WindowsNativeRdpTerminalDispatch {
    if !deadline_elapsed {
        return WindowsNativeRdpTerminalDispatch::Delivered;
    }
    tracing::error!(
        token = registration.token(),
        generation = registration.generation(),
        "Windows native RDP detached cleanup did not report a terminal outcome before the \
         deadline; completing the bounded drain as owner-lost"
    );
    record_windows_native_rdp_terminal_async(registration, WindowsRdpTerminalOutcome::OwnerLost, cx)
}

/// Two-phase view-owner close: the entity borrow only performs the pure-Rust
/// ownership take (phase A); every native COM/Win32 call runs after the update
/// closure returns (phase B) and the terminal outcome is recorded through the
/// shared registry (phase C).
async fn poll_view_owner(
    owner: &gpui::WeakEntity<crate::view::RemoteDesktopView>,
    registration: WindowsRdpRegistration,
    drain_deadline: Instant,
    deadline_elapsed: bool,
    cx: &mut gpui::AsyncApp,
) -> WindowsNativeRdpTerminalDispatch {
    use crate::view::WindowsNativeCloseTake;

    // Phase A: pure ownership transfer inside the borrow.
    let taken = owner.update(cx, |view, _| {
        view.take_windows_native_close_operation(registration)
    });
    let taken = match taken {
        Ok(taken) => taken,
        Err(error) => {
            tracing::debug!(
                ?error,
                token = registration.token(),
                generation = registration.generation(),
                "Windows native RDP view released while shutdown drain was polling"
            );
            return record_released_view_owner(registration, deadline_elapsed, cx);
        }
    };
    match taken {
        // Nothing left to close (or a close task already owns the adapter and
        // its terminal outcome).
        WindowsNativeCloseTake::Closed => WindowsNativeRdpTerminalDispatch::Delivered,
        // A maintenance operation temporarily owns the adapter; its commit (or
        // rejected-commit cleanup) owns the terminal outcome. Retry on the
        // next drain poll instead of double-closing.
        WindowsNativeCloseTake::Pending => {
            if deadline_elapsed {
                tracing::error!(
                    token = registration.token(),
                    generation = registration.generation(),
                    "Windows native RDP shutdown drain deadline hit while a maintenance \
                     operation owns the adapter; deferring to its own cleanup"
                );
            }
            WindowsNativeRdpTerminalDispatch::Delivered
        }
        WindowsNativeCloseTake::Failed => {
            record_mismatched_view_owner(registration, deadline_elapsed, cx)
        }
        WindowsNativeCloseTake::Ready(operation) => {
            if deadline_elapsed {
                // Leaking performs no COM calls; pending callbacks must never
                // observe a dropped host during teardown.
                let _ = Box::leak(Box::new(operation.into_leaked_adapter()));
                tracing::error!(
                    token = registration.token(),
                    generation = registration.generation(),
                    "Windows native RDP shutdown drain deadline hit before close; \
                     quarantining the adapter as timed-out-leaked"
                );
                let dispatch = record_windows_native_rdp_terminal_async(
                    registration,
                    WindowsRdpTerminalOutcome::TimedOutLeaked,
                    cx,
                );
                crate::view::RemoteDesktopView::finish_windows_native_close_in_view(
                    owner,
                    registration,
                    cx,
                );
                return dispatch;
            }
            // Phase B: borrow-free force close bounded by the drain deadline.
            // The runner records the terminal outcome (phase C) itself.
            crate::view::close_windows_native_operation(
                owner,
                operation,
                crate::view::WindowsNativeCloseRetryMode::ForceClose,
                drain_deadline,
                cx,
            )
            .await;
            WindowsNativeRdpTerminalDispatch::Delivered
        }
    }
}

fn record_mismatched_view_owner(
    registration: WindowsRdpRegistration,
    deadline_elapsed: bool,
    cx: &gpui::AsyncApp,
) -> WindowsNativeRdpTerminalDispatch {
    if !deadline_elapsed {
        return WindowsNativeRdpTerminalDispatch::Delivered;
    }
    tracing::error!(
        token = registration.token(),
        generation = registration.generation(),
        "Windows native RDP view no longer owns its registered adapter at the drain deadline"
    );
    record_windows_native_rdp_view_owner_lost_async(registration, cx)
}

fn record_released_view_owner(
    registration: WindowsRdpRegistration,
    deadline_elapsed: bool,
    cx: &gpui::AsyncApp,
) -> WindowsNativeRdpTerminalDispatch {
    if deadline_elapsed {
        tracing::error!(
            token = registration.token(),
            generation = registration.generation(),
            "Windows native RDP view released before the drain deadline completed"
        );
        return record_windows_native_rdp_view_owner_lost_async(registration, cx);
    }
    WindowsNativeRdpTerminalDispatch::Delivered
}

async fn poll_registration(
    registration: WindowsRdpRegistration,
    owner: Option<&WindowsNativeRdpOwner>,
    deadline: Instant,
    deadline_elapsed: bool,
    cx: &mut gpui::AsyncApp,
) -> WindowsNativeRdpTerminalDispatch {
    match owner {
        None => record_missing_owner(registration, deadline_elapsed, cx),
        Some(WindowsNativeRdpOwner::Detached) => {
            record_stalled_detached_owner(registration, deadline_elapsed, cx)
        }
        Some(WindowsNativeRdpOwner::View(owner)) => {
            poll_view_owner(owner, registration, deadline, deadline_elapsed, cx).await
        }
    }
}

fn completed_report(cx: &gpui::AsyncApp) -> Option<WindowsNativeRdpShutdownReport> {
    cx.try_read_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| controller.report())
        .flatten()
}

async fn drain(
    cx: &mut gpui::AsyncApp,
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
            if poll_registration(
                registration,
                snapshot.owners.get(&registration),
                snapshot.deadline,
                deadline_elapsed,
                cx,
            )
            .await
            .was_rejected()
            {
                tracing::error!(
                    token = registration.token(),
                    generation = registration.generation(),
                    "Windows native RDP shutdown dispatcher became unavailable; \
                     returning the last fail-closed report"
                );
                return fail_closed_report;
            }
        }
        if let Some(report) = completed_report(cx) {
            return report;
        }
        cx.background_executor()
            .timer(WINDOWS_NATIVE_RDP_DRAIN_POLL_INTERVAL)
            .await;
    }
}

/// Close Native RDP admission during a platform-driven GPUI shutdown and
/// synchronously return the latest conservative report.
///
/// GPUI invokes quit observers after the platform has committed to quitting,
/// but before it releases windows and entities. Their returned futures receive
/// only a short fixed budget, so this synchronous fallback must not start owner
/// polling or native cleanup that could be cancelled during teardown.
pub fn fail_closed_windows_native_rdp_for_platform_quit(
    cx: &mut App,
) -> WindowsNativeRdpShutdownReport {
    if !cx.has_global::<GlobalWindowsNativeRdpShutdown>() {
        tracing::error!("Windows native RDP shutdown controller is unavailable");
        return WindowsNativeRdpShutdownReport::unavailable_controller();
    }
    let start = begin_drain(cx);
    start.completed_report.unwrap_or(start.fail_closed_report)
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

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui::BorrowAppContext as _;
    use windows_rdp_host::{
        WindowsRdpShutdownCompletion, WindowsRdpShutdownLifecycle, WindowsRdpTerminalOutcome,
    };

    use super::{
        GlobalWindowsNativeRdpShutdown, WINDOWS_NATIVE_RDP_DRAIN_POLL_INTERVAL,
        WindowsNativeRdpOwner, begin_drain, fail_closed_windows_native_rdp_for_platform_quit,
        shutdown_windows_native_rdp,
    };

    #[gpui::test]
    fn dropping_shutdown_task_preserves_fail_closed_registry_state(cx: &mut gpui::TestAppContext) {
        let pending = cx.update(|cx| {
            super::super::init(cx);
            cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
                controller
                    .registry
                    .register(1)
                    .expect("pending registration")
            })
        });

        let task = cx.update(shutdown_windows_native_rdp);
        drop(task);
        cx.run_until_parked();

        cx.read_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
            assert_eq!(
                WindowsRdpShutdownLifecycle::Draining,
                controller.registry.lifecycle()
            );
            assert_eq!(vec![pending], controller.registry.pending_registrations());
            assert_eq!(1, controller.registry.active_count());
            assert!(controller.registry.report().is_none());
        });

        let report = cx.update(fail_closed_windows_native_rdp_for_platform_quit);

        assert_eq!(1, report.requested());
        assert_eq!(0, report.destroyed());
        assert_eq!(0, report.timed_out_leaked());
        assert_eq!(1, report.owner_lost());
        assert!(report.incomplete());
        cx.read_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
            assert_eq!(
                WindowsRdpShutdownLifecycle::Draining,
                controller.registry.lifecycle()
            );
            assert_eq!(vec![pending], controller.registry.pending_registrations());
            assert_eq!(1, controller.registry.active_count());
            assert!(controller.registry.report().is_none());
        });
    }

    #[gpui::test]
    fn app_shutdown_quit_observer_closes_admission_before_async_teardown(
        cx: &mut gpui::TestAppContext,
    ) {
        let pending = cx.update(|cx| {
            super::super::init(cx);
            cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
                controller
                    .registry
                    .register(1)
                    .expect("pending registration")
            })
        });
        let observed_report = Rc::new(RefCell::new(None));
        let report_for_observer = observed_report.clone();
        cx.update(|cx| {
            cx.on_app_quit(move |cx| {
                report_for_observer
                    .borrow_mut()
                    .replace(fail_closed_windows_native_rdp_for_platform_quit(cx));
                async {}
            })
            .detach();
        });

        cx.quit();

        let report = observed_report
            .borrow_mut()
            .take()
            .expect("the GPUI quit observer should run during app shutdown");
        assert_eq!(1, report.requested());
        assert_eq!(0, report.destroyed());
        assert_eq!(0, report.timed_out_leaked());
        assert_eq!(1, report.owner_lost());
        assert!(!report.controller_unavailable());
        assert!(report.incomplete());
        cx.read_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
            assert_eq!(
                WindowsRdpShutdownLifecycle::Draining,
                controller.registry.lifecycle()
            );
            assert_eq!(vec![pending], controller.registry.pending_registrations());
            assert_eq!(1, controller.registry.active_count());
            assert!(controller.registry.report().is_none());
        });
    }

    #[gpui::test]
    fn dropping_polled_shutdown_task_preserves_progress_for_platform_quit(
        cx: &mut gpui::TestAppContext,
    ) {
        let (destroyed, pending) = cx.update(|cx| {
            super::super::init(cx);
            cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
                let destroyed = controller
                    .registry
                    .register(1)
                    .expect("destroyed registration");
                let pending = controller
                    .registry
                    .register(2)
                    .expect("pending registration");
                assert!(
                    controller
                        .owners
                        .insert(pending, WindowsNativeRdpOwner::Detached)
                        .is_none()
                );
                (destroyed, pending)
            })
        });

        let task = cx.update(shutdown_windows_native_rdp);
        cx.update(|cx| {
            cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
                assert_eq!(
                    WindowsRdpShutdownCompletion::Recorded,
                    controller
                        .registry
                        .record_terminal(destroyed, WindowsRdpTerminalOutcome::Destroyed)
                );
            });
        });

        assert!(!task.is_ready());
        assert!(
            cx.executor().tick(),
            "the spawned drain task should be runnable"
        );
        assert!(
            !task.is_ready(),
            "the first drain poll should wait on the bounded poll timer"
        );
        drop(task);

        cx.read_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
            assert_eq!(
                WindowsRdpShutdownLifecycle::Draining,
                controller.registry.lifecycle()
            );
            assert_eq!(vec![pending], controller.registry.pending_registrations());
            assert_eq!(1, controller.registry.active_count());
            assert!(controller.registry.report().is_none());
            assert!(matches!(
                controller.owners.get(&pending),
                Some(WindowsNativeRdpOwner::Detached)
            ));
        });

        let report = cx.update(fail_closed_windows_native_rdp_for_platform_quit);

        assert_eq!(2, report.requested());
        assert_eq!(1, report.destroyed());
        assert_eq!(0, report.timed_out_leaked());
        assert_eq!(1, report.owner_lost());
        assert!(!report.controller_unavailable());
        assert!(report.incomplete());
        cx.read_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
            assert_eq!(
                WindowsRdpShutdownLifecycle::Draining,
                controller.registry.lifecycle()
            );
            assert_eq!(vec![pending], controller.registry.pending_registrations());
            assert_eq!(1, controller.registry.active_count());
            assert!(controller.registry.report().is_none());
            assert!(matches!(
                controller.owners.get(&pending),
                Some(WindowsNativeRdpOwner::Detached)
            ));
        });
    }

    #[gpui::test]
    fn polled_shutdown_task_returns_latest_fail_closed_report_after_controller_loss(
        cx: &mut gpui::TestAppContext,
    ) {
        let (destroyed, pending) = cx.update(|cx| {
            super::super::init(cx);
            cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
                let destroyed = controller
                    .registry
                    .register(1)
                    .expect("destroyed registration");
                let pending = controller
                    .registry
                    .register(2)
                    .expect("pending registration");
                assert!(
                    controller
                        .owners
                        .insert(pending, WindowsNativeRdpOwner::Detached)
                        .is_none()
                );
                (destroyed, pending)
            })
        });

        let task = cx.update(shutdown_windows_native_rdp);
        cx.update(|cx| {
            cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
                assert_eq!(
                    WindowsRdpShutdownCompletion::Recorded,
                    controller
                        .registry
                        .record_terminal(destroyed, WindowsRdpTerminalOutcome::Destroyed)
                );
            });
        });

        assert!(!task.is_ready());
        assert!(
            cx.executor().tick(),
            "the spawned drain task should be runnable"
        );
        assert!(
            !task.is_ready(),
            "the first drain poll should wait on the bounded poll timer"
        );

        let controller = cx.update(|cx| cx.remove_global::<GlobalWindowsNativeRdpShutdown>());
        cx.executor()
            .advance_clock(WINDOWS_NATIVE_RDP_DRAIN_POLL_INTERVAL);
        cx.executor().run_until_parked();
        assert!(
            task.is_ready(),
            "the drain task should complete after the bounded poll interval"
        );
        let report = futures::executor::block_on(task);

        assert_eq!(2, report.requested());
        assert_eq!(1, report.destroyed());
        assert_eq!(0, report.timed_out_leaked());
        assert_eq!(1, report.owner_lost());
        assert!(!report.controller_unavailable());
        assert!(report.incomplete());
        assert_eq!(
            WindowsRdpShutdownLifecycle::Draining,
            controller.registry.lifecycle()
        );
        assert_eq!(vec![pending], controller.registry.pending_registrations());
        assert_eq!(1, controller.registry.active_count());
        assert!(controller.registry.report().is_none());
        assert!(matches!(
            controller.owners.get(&pending),
            Some(WindowsNativeRdpOwner::Detached)
        ));

        let fallback = cx.update(fail_closed_windows_native_rdp_for_platform_quit);

        assert_eq!(0, fallback.requested());
        assert_eq!(0, fallback.destroyed());
        assert_eq!(0, fallback.timed_out_leaked());
        assert_eq!(0, fallback.owner_lost());
        assert!(fallback.controller_unavailable());
        assert!(fallback.incomplete());
    }

    #[gpui::test]
    fn platform_quit_fail_closed_report_preserves_progress_without_mutating_pending_registration(
        cx: &mut gpui::TestAppContext,
    ) {
        let (destroyed, pending) = cx.update(|cx| {
            super::super::init(cx);
            cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
                (
                    controller
                        .registry
                        .register(1)
                        .expect("destroyed registration"),
                    controller
                        .registry
                        .register(2)
                        .expect("pending registration"),
                )
            })
        });

        cx.update(begin_drain);
        cx.update(|cx| {
            cx.update_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
                assert_eq!(
                    WindowsRdpShutdownCompletion::Recorded,
                    controller
                        .registry
                        .record_terminal(destroyed, WindowsRdpTerminalOutcome::Destroyed)
                );
            });
        });

        let report = cx.update(fail_closed_windows_native_rdp_for_platform_quit);

        assert_eq!(2, report.requested());
        assert_eq!(1, report.destroyed());
        assert_eq!(0, report.timed_out_leaked());
        assert_eq!(1, report.owner_lost());
        assert!(report.incomplete());
        cx.read_global::<GlobalWindowsNativeRdpShutdown, _>(|controller, _| {
            assert_eq!(
                WindowsRdpShutdownLifecycle::Draining,
                controller.registry.lifecycle()
            );
            assert_eq!(vec![pending], controller.registry.pending_registrations());
            assert_eq!(1, controller.registry.active_count());
            assert!(controller.registry.report().is_none());
        });
    }
}
