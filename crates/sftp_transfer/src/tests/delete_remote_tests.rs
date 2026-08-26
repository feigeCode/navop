use std::path::PathBuf;

use gpui::TestAppContext;
use sftp::TransferProgress;

use super::{
    super::{
        SftpConnectionIdentity, SftpRemoteDeleteEntry, SftpTransferOperation, SftpTransferState,
        delete_remote_task_key,
    },
    support::{
        TestProvider, TestTransferOperation, delete_remote_request, download_request, new_executor,
        upload_request, wait_until,
    },
};

#[test]
fn delete_remote_task_key_is_connection_scoped_and_direction_specific() {
    let entries = vec![
        SftpRemoteDeleteEntry {
            remote_path: "/remote/archive".to_string(),
            is_dir: true,
        },
        SftpRemoteDeleteEntry {
            remote_path: "/remote/readme.txt".to_string(),
            is_dir: false,
        },
    ];
    let first = delete_remote_task_key(&SftpConnectionIdentity::Local(7), "/remote", &entries);
    let repeated = delete_remote_task_key(&SftpConnectionIdentity::Local(7), "/remote", &entries);
    let other_connection = delete_remote_task_key(
        &SftpConnectionIdentity::Cloud("cloud-7".to_string()),
        "/remote",
        &entries,
    );

    assert_eq!(first, repeated);
    assert_ne!(first, other_connection);
    assert!(first.as_ref().starts_with("sftp-delete-remote:local:7:"));
    assert!(!first.as_ref().starts_with("sftp-upload:"));
    assert!(!first.as_ref().starts_with("sftp-download:"));
}

#[gpui::test]
fn delete_remote_reaches_provider_with_entries_and_config_source(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let request = delete_remote_request(SftpConnectionIdentity::Local(7), "selection");
    let expected_entries = request.entries.clone();
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit_delete_remote(request, cx)
    });

    wait_until(cx, |_| provider.started() == vec![transfer]);
    assert_eq!(
        provider.operation(transfer),
        Some(TestTransferOperation::DeleteRemote)
    );
    assert_eq!(provider.delete_entries(transfer), Some(expected_entries));
    assert_eq!(
        executor
            .read_with(cx, |executor, _| executor.snapshot(transfer))
            .map(|snapshot| {
                (
                    snapshot.operation,
                    snapshot.local_path,
                    snapshot.remote_path,
                )
            }),
        Some((
            SftpTransferOperation::DeleteRemote,
            PathBuf::new(),
            "/remote".to_string(),
        ))
    );
}

#[gpui::test]
fn upload_delete_download_share_connection_fifo_lane(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let connection = SftpConnectionIdentity::Local(7);
    let upload = executor.update(cx, |executor, cx| {
        executor.submit(upload_request(connection.clone(), "upload"), cx)
    });
    let delete = executor.update(cx, |executor, cx| {
        executor.submit_delete_remote(delete_remote_request(connection.clone(), "delete"), cx)
    });
    let download = executor.update(cx, |executor, cx| {
        executor.submit_download(download_request(connection, "download"), cx)
    });

    wait_until(cx, |_| provider.started() == vec![upload]);
    provider.complete(upload, Ok(()));
    wait_until(cx, |_| provider.started() == vec![upload, delete]);
    provider.complete(delete, Ok(()));
    wait_until(cx, |_| provider.started() == vec![upload, delete, download]);
}

#[gpui::test]
fn running_delete_remote_cancel_sets_cooperative_atomic_flag(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit_delete_remote(
            delete_remote_request(SftpConnectionIdentity::Local(7), "cancelled"),
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
fn delete_remote_progress_updates_snapshot(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit_delete_remote(
            delete_remote_request(SftpConnectionIdentity::Local(7), "progress"),
            cx,
        )
    });
    wait_until(cx, |_| provider.started() == vec![transfer]);

    provider.report_progress(
        transfer,
        TransferProgress {
            transferred: 1,
            total: 2,
            speed: 0.0,
            current_file: Some("/remote/progress.txt".to_string()),
            current_file_transferred: 1,
            current_file_total: 1,
        },
    );
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(50));
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.snapshot(transfer).is_some_and(|snapshot| {
                snapshot.transferred == 1
                    && snapshot.total == Some(2)
                    && snapshot.current_file.as_deref() == Some("/remote/progress.txt")
            })
        })
    });
}
