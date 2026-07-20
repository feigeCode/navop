use std::sync::Arc;

use gpui::http_client::{AsyncBody, HttpClient, http};

use super::{download_update_file, download_update_file_from_sources, partial_download_path};
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
