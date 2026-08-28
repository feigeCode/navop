use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui::{AppContext, Subscription, TestAppContext};
use one_core::background_tasks::{
    BackgroundTask, BackgroundTaskId, BackgroundTaskProgressUnit, BackgroundTaskStatus,
};
use one_core::storage::models::{SshAuthMethod, SshParams, StoredConnection};
use sftp::TransferProgress;

use super::{
    super::{SftpConnectionIdentity, SftpTransferEvent, SftpTransferState},
    support::{TestProvider, new_executor, upload_request, wait_until},
};

struct TestObserver {
    _subscription: Subscription,
}

#[gpui::test]
fn reserved_transfer_emits_events_only_after_commit(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let events = Arc::new(Mutex::new(Vec::new()));
    let observer = cx.update(|cx| {
        let source = executor.clone();
        let events = events.clone();
        cx.new(|cx| TestObserver {
            _subscription: cx.subscribe(
                &source,
                move |_this, _source, event: &SftpTransferEvent, _cx| {
                    events.lock().unwrap().push(event.clone());
                },
            ),
        })
    });
    let reservation = executor.update(cx, |executor, _| {
        executor.reserve(upload_request(
            SftpConnectionIdentity::Local(7),
            "reserved-events",
        ))
    });
    let transfer = reservation.id();

    cx.run_until_parked();
    assert!(events.lock().unwrap().is_empty());
    assert!(provider.started().is_empty());

    let committed = executor.update(cx, |executor, cx| {
        match executor.commit_reserved(reservation, cx) {
            Ok(id) => id,
            Err(_) => panic!("the reserving executor must accept its reservation"),
        }
    });
    assert_eq!(committed, transfer);
    wait_until(cx, |_| provider.started() == vec![transfer]);
    {
        let events = events.lock().unwrap();
        assert!(matches!(
            events.first(),
            Some(SftpTransferEvent::Added(id)) if *id == transfer
        ));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, SftpTransferEvent::Updated(id) if *id == transfer))
        );
    }

    provider.complete(transfer, Ok(()));
    wait_until(cx, |_| {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, SftpTransferEvent::Finished(id) if *id == transfer))
    });
    drop(observer);
}

#[gpui::test]
fn dropping_upload_observer_does_not_abort_transfer(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let observer = cx.update(|cx| {
        let source = executor.clone();
        cx.new(|cx| TestObserver {
            _subscription: cx.subscribe(
                &source,
                |_this, _source, _event: &SftpTransferEvent, _cx| {},
            ),
        })
    });
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Local(7), "observer"),
            cx,
        )
    });
    wait_until(cx, |_| provider.started() == vec![transfer]);

    let observer_weak = observer.downgrade();
    drop(observer);
    assert!(observer_weak.upgrade().is_none());

    provider.complete(transfer, Ok(()));
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.snapshot(transfer).unwrap().state == SftpTransferState::Succeeded
        })
    });
}

#[test]
fn stored_connection_identity_prefers_local_id() {
    let mut connection = stored_connection();
    connection.id = Some(7);
    connection.cloud_id = Some("cloud-7".to_string());

    assert_eq!(
        SftpConnectionIdentity::from_stored(&connection),
        Some(SftpConnectionIdentity::Local(7))
    );
}

#[test]
fn stored_connection_identity_falls_back_to_cloud_id() {
    let mut connection = stored_connection();
    connection.cloud_id = Some("cloud-7".to_string());

    assert_eq!(
        SftpConnectionIdentity::from_stored(&connection),
        Some(SftpConnectionIdentity::Cloud("cloud-7".to_string()))
    );
}

#[test]
fn stored_connection_without_ids_needs_runtime_identity() {
    assert_eq!(
        SftpConnectionIdentity::from_stored(&stored_connection()),
        None
    );
}

#[gpui::test]
fn background_manager_cancel_of_queued_task_finishes_cancelled(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let connection = SftpConnectionIdentity::Local(7);
    let first = executor.update(cx, |executor, cx| {
        executor.submit(upload_request(connection.clone(), "first"), cx)
    });
    let queued = executor.update(cx, |executor, cx| {
        executor.submit(upload_request(connection, "queued"), cx)
    });
    wait_until(cx, |_| provider.started() == vec![first]);

    let manager = cx.update(one_core::background_tasks::global);
    let task_id = background_task_by_title(&manager, "Upload queued", cx).id;
    assert!(manager.update(cx, |manager, cx| manager.request_cancel(task_id, cx)));

    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.snapshot(queued).unwrap().state == SftpTransferState::Cancelled
        })
    });
    assert_eq!(
        background_task(&manager, task_id, cx).status,
        BackgroundTaskStatus::Cancelled
    );

    provider.complete(first, Ok(()));
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.snapshot(first).unwrap().state == SftpTransferState::Succeeded
        })
    });
    assert_eq!(provider.started(), vec![first]);
}

#[gpui::test]
fn running_cancel_sets_cooperative_atomic_flag(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Local(7), "running"),
            cx,
        )
    });
    wait_until(cx, |_| provider.started() == vec![transfer]);

    assert!(executor.update(cx, |executor, cx| executor.cancel(transfer, cx)));
    wait_until(cx, |_| provider.is_cancelled(transfer));
    assert_eq!(
        executor.read_with(cx, |executor, _| executor.snapshot(transfer).unwrap().state),
        SftpTransferState::Cancelling
    );

    provider.complete(transfer, Ok(()));
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.snapshot(transfer).unwrap().state == SftpTransferState::Cancelled
        })
    });
}

#[gpui::test]
fn progress_updates_snapshot_and_background_task(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Local(7), "progress"),
            cx,
        )
    });
    wait_until(cx, |_| provider.started() == vec![transfer]);

    provider.report_progress(
        transfer,
        TransferProgress {
            transferred: 64,
            total: 128,
            speed: 32.0,
            current_file: Some("nested/file.txt".to_string()),
            current_file_transferred: 64,
            current_file_total: 128,
        },
    );
    cx.background_executor
        .advance_clock(Duration::from_millis(50));
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor
                .snapshot(transfer)
                .is_some_and(|snapshot| snapshot.transferred == 64)
        })
    });

    let snapshot = executor
        .read_with(cx, |executor, _| executor.snapshot(transfer))
        .unwrap();
    assert_eq!(snapshot.total, Some(128));
    assert_eq!(snapshot.speed, 32.0);
    assert_eq!(snapshot.current_file.as_deref(), Some("nested/file.txt"));

    let manager = cx.update(one_core::background_tasks::global);
    let task = background_task_by_title(&manager, "Upload progress", cx);
    assert_eq!(task.group.as_deref(), Some("Test SFTP"));
    let progress = task.progress.expect("background progress should exist");
    assert_eq!(progress.current, 64);
    assert_eq!(progress.total, Some(128));
    assert_eq!(progress.unit, BackgroundTaskProgressUnit::Bytes);
    assert_eq!(progress.message.as_deref(), Some("32 B/s"));
    assert_eq!(task.detail.as_deref(), Some("nested/file.txt"));
}

#[gpui::test]
fn global_init_is_idempotent_and_strongly_held(cx: &mut TestAppContext) {
    let first = cx.update(|cx| {
        super::super::init(cx);
        super::super::global(cx)
    });
    let second = cx.update(|cx| {
        super::super::init(cx);
        super::super::global(cx)
    });
    assert_eq!(first.entity_id(), second.entity_id());

    let weak = first.downgrade();
    drop(first);
    drop(second);
    assert!(weak.upgrade().is_some());
}

fn background_task_by_title(
    manager: &gpui::Entity<one_core::background_tasks::BackgroundTaskManager>,
    title: &str,
    cx: &TestAppContext,
) -> BackgroundTask {
    manager
        .read_with(cx, |manager, _| {
            manager
                .tasks()
                .into_iter()
                .find(|task| task.kind.as_ref() == "sftp-upload" && task.title.as_ref() == title)
        })
        .expect("background task should exist")
}

fn background_task(
    manager: &gpui::Entity<one_core::background_tasks::BackgroundTaskManager>,
    id: BackgroundTaskId,
    cx: &TestAppContext,
) -> BackgroundTask {
    manager
        .read_with(cx, |manager, _| {
            manager.tasks().into_iter().find(|task| task.id == id)
        })
        .expect("background task should exist")
}

fn stored_connection() -> StoredConnection {
    StoredConnection::new_ssh(
        "test".to_string(),
        SshParams {
            host: "localhost".to_string(),
            port: 22,
            username: "user".to_string(),
            auth_method: SshAuthMethod::Agent,
            sftp_account: None,
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            keyboard_interactive: None,
            terminal_encoding: Default::default(),
            terminal_type: Default::default(),
            account_expect: Default::default(),
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            x11_forwarding: None,
            allow_legacy_algorithms: None,
            jump_server: None,
            proxy: None,
            os_id: None,
            icon: None,
            icon_file_path: None,
        },
        None,
    )
}
