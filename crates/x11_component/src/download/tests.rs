use std::time::Duration;

use crate::X11ComponentError;

#[tokio::test]
async fn http_timeout_is_a_download_error() {
    let pending = std::future::pending::<Result<Vec<u8>, X11ComponentError>>();
    let error = super::with_download_timeout(Duration::from_millis(1), pending)
        .await
        .unwrap_err();
    assert!(matches!(error, X11ComponentError::Download(_)));
}
