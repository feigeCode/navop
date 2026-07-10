use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use super::GlobalDbState;
use crate::import_export::{
    ExportConfig, ExportProgressRequest, ImportConfig, ImportProgressRequest,
};

fn poll_once<T>(future: impl Future<Output = anyhow::Result<T>>) -> anyhow::Error {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());

    match future.as_mut().poll(&mut cx) {
        Poll::Ready(Err(error)) => error,
        Poll::Ready(Ok(_)) => panic!("runtime-bound database operation unexpectedly succeeded"),
        Poll::Pending => panic!("runtime contract must be checked before the first await"),
    }
}

#[test]
fn export_core_rejects_missing_tokio_runtime_before_connection_lookup() {
    let state = GlobalDbState::new();
    let error = poll_once(
        state.export_data_with_progress_on_tokio(ExportProgressRequest {
            connection_id: "missing-export-connection".to_string(),
            config: ExportConfig::default(),
            progress_tx: None,
        }),
    );

    assert_eq!(
        "database export requires the application Tokio runtime",
        error.to_string()
    );
}

#[test]
fn import_core_rejects_missing_tokio_runtime_before_connection_lookup() {
    let state = GlobalDbState::new();
    let error = poll_once(
        state.import_data_with_progress_on_tokio(ImportProgressRequest {
            connection_id: "missing-import-connection".to_string(),
            config: ImportConfig::default(),
            data: String::new(),
            file_name: String::new(),
            progress_tx: None,
        }),
    );

    assert_eq!(
        "database import requires the application Tokio runtime",
        error.to_string()
    );
}
