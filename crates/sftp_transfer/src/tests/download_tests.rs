use std::path::PathBuf;

use gpui::TestAppContext;
use one_core::background_tasks::{BackgroundTaskProgressUnit, BackgroundTaskStatus};
use sftp::TransferProgress;

use super::{
    super::{SftpConnectionIdentity, SftpTransferOperation, SftpTransferState, download_task_key},
    support::{
        TestProvider, TestTransferOperation, download_request, new_executor, upload_request,
        wait_until,
    },
};

#[test]
fn download_task_key_is_stable_connection_scoped_and_direction_specific() {
    let local_path = PathBuf::from("/tmp/archive.tar");
    let first = download_task_key(
        &SftpConnectionIdentity::Local(7),
        "/remote/archive.tar",
        &local_path,
    );
    let repeated = download_task_key(
        &SftpConnectionIdentity::Local(7),
        "/remote/archive.tar",
        &local_path,
    );
    let other_connection = download_task_key(
        &SftpConnectionIdentity::Cloud("cloud-7".to_string()),
        "/remote/archive.tar",
        &local_path,
    );

    assert_eq!(first, repeated);
    assert_ne!(first, other_connection);
    assert_ne!(
        first.as_ref(),
        "sftp-upload:local:7:16:/tmp/archive.tar:19:/remote/archive.tar"
    );
    assert_eq!(
        first.as_ref(),
        "sftp-download:local:7:19:/remote/archive.tar:16:/tmp/archive.tar"
    );
}

#[gpui::test]
fn download_reaches_provider_with_paths_and_config_source(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit_download(
            download_request(SftpConnectionIdentity::Local(7), "archive"),
            cx,
        )
    });

    wait_until(cx, |_| provider.started() == vec![transfer]);
    assert_eq!(
        provider.operation(transfer),
        Some(TestTransferOperation::Download)
    );
    assert_eq!(
        provider.paths(transfer),
        Some((
            "/remote/archive".to_string(),
            PathBuf::from("/tmp/archive"),
            false,
        ))
    );
    assert_eq!(
        executor
            .read_with(cx, |executor, _| executor.snapshot(transfer))
            .map(|snapshot| (snapshot.operation, snapshot.local_path)),
        Some((
            SftpTransferOperation::Download,
            PathBuf::from("/tmp/archive"),
        ))
    );
}

#[gpui::test]
fn upload_and_download_share_connection_fifo_lane(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let connection = SftpConnectionIdentity::Local(7);
    let upload = executor.update(cx, |executor, cx| {
        executor.submit(upload_request(connection.clone(), "upload"), cx)
    });
    let download = executor.update(cx, |executor, cx| {
        executor.submit_download(download_request(connection, "download"), cx)
    });

    wait_until(cx, |_| provider.started() == vec![upload]);
    provider.complete(upload, Ok(()));
    wait_until(cx, |_| provider.started() == vec![upload, download]);
    assert_eq!(
        provider.operation(download),
        Some(TestTransferOperation::Download)
    );
}

#[gpui::test]
fn running_download_cancel_sets_cooperative_atomic_flag(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit_download(
            download_request(SftpConnectionIdentity::Local(7), "cancelled"),
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
fn download_progress_updates_snapshot_and_background_task(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit_download(
            download_request(SftpConnectionIdentity::Local(7), "progress"),
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
        .advance_clock(std::time::Duration::from_millis(50));
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor
                .snapshot(transfer)
                .is_some_and(|snapshot| snapshot.transferred == 64)
        })
    });

    let manager = cx.update(one_core::background_tasks::global);
    let task = manager
        .read_with(cx, |manager, _| {
            manager.tasks().into_iter().find(|task| {
                task.kind.as_ref() == "sftp-download" && task.title.as_ref() == "Download progress"
            })
        })
        .expect("download background task should exist");
    assert_eq!(task.status, BackgroundTaskStatus::Running);
    assert_eq!(task.group.as_deref(), Some("Test SFTP"));
    assert_eq!(task.detail.as_deref(), Some("nested/file.txt"));
    assert_eq!(task.open_folder, Some(PathBuf::from("/tmp")));
    let progress = task.progress.expect("background progress should exist");
    assert_eq!(progress.current, 64);
    assert_eq!(progress.total, Some(128));
    assert_eq!(progress.unit, BackgroundTaskProgressUnit::Bytes);
    assert_eq!(progress.message.as_deref(), Some("32 B/s"));
}
