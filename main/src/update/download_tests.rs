use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use gpui::http_client::{AsyncBody, HttpClient, http};

use super::{
    DOWNLOAD_CANCELLED_ERROR, download_update_file, download_update_file_from_sources,
    download_update_file_from_sources_cancellable, partial_download_path, verify_sha256,
};
use crate::update::test_support::FakeHttpClient;

#[tokio::test]
async fn failed_download_does_not_remove_existing_archive() {
    let temp_dir = tempfile::TempDir::new().expect("创建临时目录失败");
    let download_path = temp_dir.path().join("navop.tar.gz");
    std::fs::write(&download_path, b"existing-package").expect("写入已有归档失败");
    let client = Arc::new(FakeHttpClient::new(vec![FakeHttpClient::response(
        503,
        "unavailable",
    )]));
    let http_client: Arc<dyn HttpClient> = client;

    let result = download_update_file(
        http_client,
        "https://example.test/navop.tar.gz",
        &download_path,
        |_, _| {},
    )
    .await;

    assert!(result.is_err());
    assert_eq!(
        std::fs::read(&download_path).expect("已有归档不应被失败任务删除"),
        b"existing-package"
    );
    assert!(!partial_download_path(&download_path).unwrap().exists());
}

#[tokio::test]
async fn download_update_file_from_sources_falls_back_to_second_url() {
    let temp_dir = tempfile::TempDir::new().expect("创建临时目录失败");
    let download_path = temp_dir.path().join("navop.tar.gz");
    let client = Arc::new(FakeHttpClient::new(vec![
        http::Response::builder()
            .status(503)
            .body(AsyncBody::from(Vec::new()))
            .map_err(|err| anyhow::anyhow!("构建响应失败: {}", err)),
        FakeHttpClient::response(200, "github-package"),
    ]));
    let http_client: Arc<dyn HttpClient> = client.clone();
    let urls = vec![
        "https://navop.pdyyds.cn/releases/v9.9.9/navop-x86_64-unknown-linux-gnu.tar.gz"
            .to_string(),
        "https://github.com/feigeCode/navop/releases/download/v9.9.9/navop-x86_64-unknown-linux-gnu.tar.gz"
            .to_string(),
    ];

    download_update_file_from_sources(http_client, &urls, &download_path, |_, _| {})
        .await
        .expect("应从第二个下载源成功下载");

    assert_eq!(
        std::fs::read(&download_path).expect("应读取最终归档"),
        b"github-package"
    );
    assert!(!partial_download_path(&download_path).unwrap().exists());
    let requests = client.take_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].uri, urls[0]);
    assert_eq!(requests[1].uri, urls[1]);
}

#[tokio::test]
async fn pre_cancelled_download_does_not_send_request_or_create_partial_file() {
    let temp_dir = tempfile::TempDir::new().expect("创建临时目录失败");
    let download_path = temp_dir.path().join("navop.tar.gz");
    let client = Arc::new(FakeHttpClient::new(Vec::new()));
    let http_client: Arc<dyn HttpClient> = client.clone();
    let urls = vec!["https://example.test/navop.tar.gz".to_string()];
    let cancel_requested = Arc::new(AtomicBool::new(false));
    cancel_requested.store(true, Ordering::Relaxed);

    let result = download_update_file_from_sources_cancellable(
        http_client,
        &urls,
        &download_path,
        cancel_requested,
        |_, _| {},
    )
    .await;

    assert_eq!(result, Err(DOWNLOAD_CANCELLED_ERROR.to_string()));
    assert!(client.take_requests().is_empty());
    assert!(!download_path.exists());
    assert!(!partial_download_path(&download_path).unwrap().exists());
}

#[tokio::test]
async fn local_update_file_is_copied_with_progress_and_sha_verification() {
    let temp_dir = tempfile::TempDir::new().expect("创建临时目录失败");
    let source_path = temp_dir.path().join("fixture.tar.gz");
    let download_path = temp_dir.path().join("downloaded.tar.gz");
    let source_data = (0..32_768u32)
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    std::fs::write(&source_path, &source_data).expect("写入本地更新包失败");
    let client = Arc::new(FakeHttpClient::new(Vec::new()));
    let http_client: Arc<dyn HttpClient> = client.clone();
    let mut progress = Vec::new();

    download_update_file(
        http_client,
        &source_path.to_string_lossy(),
        &download_path,
        |done, total| progress.push((done, total)),
    )
    .await
    .expect("应复制本地更新包");

    assert_eq!(
        std::fs::read(&download_path).expect("读取复制后的更新包失败"),
        source_data
    );
    assert_eq!(
        progress.last().copied(),
        Some((source_data.len() as u64, Some(source_data.len() as u64)))
    );
    assert!(client.take_requests().is_empty());
    verify_sha256(
        &download_path,
        "55cbe1972bf5e6d07c7743eb8e27ca8912e950d6a8f73a34a082610cbeefa0ee",
    )
    .expect("复制后的文件应通过 SHA256 校验");
    assert!(!partial_download_path(&download_path).unwrap().exists());
}

#[tokio::test]
async fn pre_cancelled_local_download_does_not_create_destination_or_partial_file() {
    let temp_dir = tempfile::TempDir::new().expect("创建临时目录失败");
    let source_path = temp_dir.path().join("fixture.tar.gz");
    let download_path = temp_dir.path().join("downloaded.tar.gz");
    std::fs::write(&source_path, b"local-package").expect("写入本地更新包失败");
    let client = Arc::new(FakeHttpClient::new(Vec::new()));
    let http_client: Arc<dyn HttpClient> = client.clone();
    let urls = vec![source_path.to_string_lossy().into_owned()];
    let cancel_requested = Arc::new(AtomicBool::new(true));

    let result = download_update_file_from_sources_cancellable(
        http_client,
        &urls,
        &download_path,
        cancel_requested,
        |_, _| {},
    )
    .await;

    assert_eq!(result, Err(DOWNLOAD_CANCELLED_ERROR.to_string()));
    assert!(client.take_requests().is_empty());
    assert!(!download_path.exists());
    assert!(!partial_download_path(&download_path).unwrap().exists());
}
