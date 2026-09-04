use anyhow::anyhow;
use gpui::TestAppContext;

use super::{
    super::{SftpConnectionIdentity, SftpTransferState},
    support::{TestProvider, new_executor, upload_request, wait_until},
};

#[gpui::test]
fn transient_upload_timeout_is_retried_once(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Local(7), "retry-timeout"),
            cx,
        )
    });

    wait_until(cx, |_| provider.started() == vec![transfer]);
    provider.complete(
        transfer,
        Err(anyhow!("Failed to flush remote temporary file: Timeout")),
    );
    wait_until(cx, |_| provider.started() == vec![transfer, transfer]);

    provider.complete(transfer, Ok(()));
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.snapshot(transfer).unwrap().state == SftpTransferState::Succeeded
        })
    });
}

#[gpui::test]
fn permanent_upload_error_is_not_retried(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Local(7), "no-retry-permission"),
            cx,
        )
    });

    wait_until(cx, |_| provider.started() == vec![transfer]);
    provider.complete(transfer, Err(anyhow!("Permission denied")));
    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.snapshot(transfer).unwrap().state == SftpTransferState::Failed
        })
    });
    assert_eq!(provider.started(), vec![transfer]);
}

#[gpui::test]
fn transient_upload_error_is_retried_at_most_once(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Local(7), "retry-limit"),
            cx,
        )
    });

    wait_until(cx, |_| provider.started() == vec![transfer]);
    provider.complete(transfer, Err(anyhow!("connection reset by peer")));
    wait_until(cx, |_| provider.started() == vec![transfer, transfer]);
    provider.complete(transfer, Err(anyhow!("Timeout")));

    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            let snapshot = executor.snapshot(transfer).unwrap();
            snapshot.state == SftpTransferState::Failed
                && snapshot
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("failed after 2 attempts"))
        })
    });
    assert_eq!(provider.started(), vec![transfer, transfer]);
}

#[gpui::test]
fn permanent_second_attempt_error_is_reported_without_a_third_attempt(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Local(7), "retry-permanent"),
            cx,
        )
    });

    wait_until(cx, |_| provider.started() == vec![transfer]);
    provider.complete(transfer, Err(anyhow!("Timeout")));
    wait_until(cx, |_| provider.started() == vec![transfer, transfer]);
    provider.complete(transfer, Err(anyhow!("Permission denied")));

    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            let snapshot = executor.snapshot(transfer).unwrap();
            snapshot.state == SftpTransferState::Failed
                && snapshot
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("failed after 2 attempts"))
        })
    });
    assert_eq!(provider.started(), vec![transfer, transfer]);
}

#[gpui::test]
fn cancellation_during_retry_backoff_prevents_another_attempt(cx: &mut TestAppContext) {
    let provider = TestProvider::default();
    let executor = new_executor(provider.clone(), cx);
    let transfer = executor.update(cx, |executor, cx| {
        executor.submit(
            upload_request(SftpConnectionIdentity::Local(7), "cancel-retry"),
            cx,
        )
    });

    wait_until(cx, |_| provider.started() == vec![transfer]);
    provider.complete(transfer, Err(anyhow!("Timeout")));
    assert!(executor.update(cx, |executor, cx| executor.cancel(transfer, cx)));

    wait_until(cx, |cx| {
        executor.read_with(cx, |executor, _| {
            executor.snapshot(transfer).unwrap().state == SftpTransferState::Cancelled
        })
    });
    assert_eq!(provider.started(), vec![transfer]);
}
