use anyhow::anyhow;
use gpui::TestAppContext;

use super::{SftpConnectionIdentity, SftpTransferState, scheduler::ConnectionLanes};

mod delete_remote_tests;
mod download_tests;
mod lifecycle_tests;
mod support;
mod upload_retry_tests;

use support::*;

#[test]
fn same_connection_is_fifo() {
    let connection = SftpConnectionIdentity::Local(7);
    let first = transfer_id(1);
    let second = transfer_id(2);
    let mut lanes = ConnectionLanes::default();

    lanes.enqueue(connection.clone(), first);
    lanes.enqueue(connection.clone(), second);

    assert_eq!(lanes.take_startable(&connection), Some(first));
    assert_eq!(lanes.take_startable(&connection), None);
    assert_eq!(lanes.complete(&connection, first), Some(second));
}

#[test]
fn different_connections_are_independently_startable() {
    let first_connection = SftpConnectionIdentity::Local(7);
    let second_connection = SftpConnectionIdentity::Cloud("cloud-8".to_string());
    let first = transfer_id(1);
    let second = transfer_id(2);
    let mut lanes = ConnectionLanes::default();

    lanes.enqueue(first_connection.clone(), first);
    lanes.enqueue(second_connection.clone(), second);

    assert_eq!(lanes.take_startable(&first_connection), Some(first));
    assert_eq!(lanes.take_startable(&second_connection), Some(second));
}

#[test]
fn cancelling_pending_removes_it_without_blocking_the_lane() {
    let connection = SftpConnectionIdentity::Local(7);
    let first = transfer_id(1);
    let cancelled = transfer_id(2);
    let third = transfer_id(3);
    let mut lanes = ConnectionLanes::default();

    lanes.enqueue(connection.clone(), first);
    lanes.enqueue(connection.clone(), cancelled);
    lanes.enqueue(connection.clone(), third);
    assert_eq!(lanes.take_startable(&connection), Some(first));

    assert!(lanes.remove_pending(&connection, cancelled));
    assert_eq!(lanes.complete(&connection, first), Some(third));
}

#[test]
fn completing_a_lane_only_advances_that_connection() {
    let first_connection = SftpConnectionIdentity::Local(7);
    let second_connection = SftpConnectionIdentity::Local(8);
    let first = transfer_id(1);
    let next = transfer_id(2);
    let other = transfer_id(3);
    let mut lanes = ConnectionLanes::default();

    lanes.enqueue(first_connection.clone(), first);
    lanes.enqueue(first_connection.clone(), next);
    lanes.enqueue(second_connection.clone(), other);
    assert_eq!(lanes.take_startable(&first_connection), Some(first));
    assert_eq!(lanes.take_startable(&second_connection), Some(other));

    assert_eq!(lanes.complete(&first_connection, first), Some(next));
    assert_eq!(lanes.running(&second_connection), Some(other));
}

#[gpui::test]
fn same_connection_provider_calls_are_fifo(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let connection = SftpConnectionIdentity::Local(7);
    let first = executor.update(cx, |executor, cx| {
        executor.submit(upload_request(connection.clone(), "first"), cx)
    });
    let second = executor.update(cx, |executor, cx| {
        executor.submit(upload_request(connection, "second"), cx)
    });

    wait_until(cx, |_| provider.started() == vec![first]);
    provider.complete(first, Ok(()));
    wait_until(cx, |_| provider.started() == vec![first, second]);
}

#[gpui::test]
fn different_connections_start_provider_in_parallel(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let first = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Local(7), "first"),
            cx,
        )
    });
    let second = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Cloud("cloud-8".into()), "second"),
            cx,
        )
    });

    wait_until(cx, |_| {
        let started = provider.started();
        started.contains(&first) && started.contains(&second)
    });
}

#[gpui::test]
fn reserved_transfer_is_inert_until_committed(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let reservation = executor.update(cx, |executor, _| {
        executor.reserve(upload_request(SftpConnectionIdentity::Local(7), "reserved"))
    });
    let transfer = reservation.id();

    cx.run_until_parked();
    assert!(provider.started().is_empty());
    assert!(
        executor.read_with(cx, |executor, _| executor.snapshot(transfer).is_none()),
        "reservation must not create an active or completed transfer"
    );

    let committed = executor.update(cx, |executor, cx| {
        match executor.commit_reserved(reservation, cx) {
            Ok(id) => id,
            Err(_) => panic!("the reserving executor must accept its reservation"),
        }
    });
    assert_eq!(committed, transfer);
    wait_until(cx, |_| provider.started() == vec![transfer]);

    provider.complete(transfer, Ok(()));
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.snapshot(transfer).unwrap().state == SftpTransferState::Succeeded
        })
    });
}

#[gpui::test]
fn reservation_cannot_be_committed_by_another_executor(cx: &mut TestAppContext) {
    let first_provider = TestProvider::default();
    let second_provider = TestProvider::default();
    let first_executor = new_executor(first_provider.clone(), cx);
    let second_executor = new_executor(second_provider.clone(), cx);
    let reservation = first_executor.update(cx, |executor, _| {
        executor.reserve(upload_request(
            SftpConnectionIdentity::Local(7),
            "wrong-executor",
        ))
    });
    let transfer = reservation.id();

    let rejected =
        second_executor.update(cx, |executor, cx| executor.commit_reserved(reservation, cx));
    assert!(rejected.is_err());
    cx.run_until_parked();
    assert!(first_provider.started().is_empty());
    assert!(second_provider.started().is_empty());
    assert!(first_executor.read_with(cx, |executor, _| executor.snapshot(transfer).is_none()));
    assert!(second_executor.read_with(cx, |executor, _| executor.snapshot(transfer).is_none()));
}

#[gpui::test]
fn both_connection_sources_reach_provider(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let session_manager_transfer = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Local(7), "session-manager"),
            cx,
        )
    });
    let config_transfer = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request_with_config(SftpConnectionIdentity::Local(8), "config"),
            cx,
        )
    });

    wait_until(cx, |_| {
        provider.connection_source(session_manager_transfer)
            == Some(TestConnectionSourceKind::SessionManager)
            && provider.connection_source(config_transfer) == Some(TestConnectionSourceKind::Config)
    });
}

#[gpui::test]
fn cancelled_pending_transfer_never_reaches_provider(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let connection = SftpConnectionIdentity::Local(7);
    let first = executor.update(cx, |executor, cx| {
        executor.submit(upload_request(connection.clone(), "first"), cx)
    });
    let cancelled = executor.update(cx, |executor, cx| {
        executor.submit(upload_request(connection, "cancelled"), cx)
    });

    wait_until(cx, |_| provider.started() == vec![first]);
    assert!(executor.update(cx, |executor, cx| executor.cancel(cancelled, cx)));
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.snapshot(cancelled).unwrap().state == SftpTransferState::Cancelled
        })
    });
    provider.complete(first, Ok(()));
    cx.run_until_parked();
    assert_eq!(provider.started(), vec![first]);
}

#[gpui::test]
fn failed_transfer_does_not_block_next_in_lane(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let connection = SftpConnectionIdentity::Local(7);
    let first = executor.update(cx, |executor, cx| {
        executor.submit(upload_request(connection.clone(), "first"), cx)
    });
    let second = executor.update(cx, |executor, cx| {
        executor.submit(upload_request(connection, "second"), cx)
    });

    wait_until(cx, |_| provider.started() == vec![first]);
    provider.complete(first, Err(anyhow!("upload failed")));
    wait_until(cx, |_| provider.started() == vec![first, second]);
}

#[gpui::test]
fn completed_history_is_bounded_and_evicts_oldest(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor_with_history_limit(provider.clone(), 2, cx);
    let connection = SftpConnectionIdentity::Local(7);
    let mut completed = Vec::new();

    for name in ["first", "second", "third"] {
        let transfer = executor.update(cx, |executor, cx| {
            executor.submit(upload_request(connection.clone(), name), cx)
        });
        wait_until(cx, |_| provider.started().last() == Some(&transfer));
        provider.complete(transfer, Ok(()));
        wait_until(cx, |cx| {
            executor.read_with(cx, |executor, _| {
                executor
                    .snapshot(transfer)
                    .is_some_and(|snapshot| snapshot.state == SftpTransferState::Succeeded)
            })
        });
        completed.push(transfer);
    }

    assert!(executor.read_with(cx, |executor, _| executor.snapshot(completed[0]).is_none()));
    assert!(executor.read_with(cx, |executor, _| executor.snapshot(completed[1]).is_some()));
    assert!(executor.read_with(cx, |executor, _| executor.snapshot(completed[2]).is_some()));
    assert!(
        executor
            .read_with(cx, |executor, _| executor
                .active_for_connection(&connection))
            .is_none()
    );
    assert_eq!(
        executor.read_with(cx, |executor, _| executor.pending_count(&connection)),
        0
    );
    assert_eq!(provider.retained_transfer_resource_count(), 0);
}

#[gpui::test]
fn cancellation_watcher_exits_after_success(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Local(7), "watcher"),
            cx,
        )
    });

    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.active_cancellation_watcher_count() == 1
        })
    });
    wait_until(cx, |_| provider.started() == vec![transfer]);
    provider.complete(transfer, Ok(()));
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor
                .snapshot(transfer)
                .is_some_and(|snapshot| snapshot.state == SftpTransferState::Succeeded)
        })
    });
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.active_cancellation_watcher_count() == 0
        })
    });
}

#[gpui::test]
fn cancellation_wins_over_late_provider_success(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Local(7), "cancelled"),
            cx,
        )
    });

    wait_until(cx, |_| provider.started() == vec![transfer]);
    assert!(executor.update(cx, |executor, cx| executor.cancel(transfer, cx)));
    provider.complete(transfer, Ok(()));
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.snapshot(transfer).unwrap().state == SftpTransferState::Cancelled
        })
    });
}
