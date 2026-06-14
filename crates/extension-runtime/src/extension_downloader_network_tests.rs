use std::{
    collections::VecDeque,
    fs,
    sync::{Arc, Mutex},
};

use futures::FutureExt;
use gpui::http_client::{self, AsyncBody, HttpClient, Url, http};

use crate::extension::{
    DatabaseDriverExtensionProvider, ExtensionKind, ExtensionRegistry, ExtensionSummary,
};
use crate::extension_downloader::{
    MarketplaceEntry, download_marketplace_entry_to_staging, fetch_manifest_url,
    install_marketplace_entry_generic,
};

#[test]
fn fetch_manifest_url_parses_marketplace_manifest() {
    let client = Arc::new(FakeHttpClient::new(vec![FakeHttpClient::response(
        200,
        r#"{
            "release_version": "2026.06",
            "extensions": [{
                "id": "fake_pg",
                "kind": "database_driver",
                "name": "Fake PostgreSQL",
                "version": "1.2.3",
                "asset_url": "https://example.test/fake_pg.tar.gz",
                "sha256": "abc"
            }]
        }"#,
    )]));

    let manifest = smol::block_on(fetch_manifest_url(
        client.clone(),
        "https://example.test/manifest.json",
    ))
    .unwrap();

    assert_eq!("2026.06", manifest.release_version);
    let entries = manifest.into_entries();
    assert_eq!(1, entries.len());
    assert_eq!(ExtensionKind::DatabaseDriver, entries[0].kind);
    assert_eq!(
        "https://example.test/manifest.json",
        client.take_requests()[0].uri
    );
}

#[test]
fn download_marketplace_entry_to_staging_verifies_sha256_and_extracts_tarball() {
    let tarball = database_driver_tarball_bytes();
    let sha256 = sha256_hex(&tarball);
    let client = Arc::new(FakeHttpClient::new(vec![binary_response(200, tarball)]));
    let entry = marketplace_database_driver_entry(Some(sha256));

    let staging = smol::block_on(download_marketplace_entry_to_staging(
        client.clone(),
        &entry,
    ))
    .unwrap();

    assert!(staging.join("driver.json").exists());
    assert!(staging.join("driver-bin").exists());
    assert_eq!(
        "https://example.test/fake_pg.tar.gz",
        client.take_requests()[0].uri
    );
    fs::remove_dir_all(staging).unwrap();
}

#[test]
fn download_marketplace_entry_to_staging_requires_sha256_for_non_language_entries() {
    let client = Arc::new(FakeHttpClient::new(Vec::new()));
    let entry = marketplace_database_driver_entry(None);

    let err = smol::block_on(download_marketplace_entry_to_staging(
        client.clone(),
        &entry,
    ))
    .unwrap_err();

    assert!(err.to_string().contains("缺少 sha256"));
    assert!(client.take_requests().is_empty());
}

#[test]
fn install_marketplace_entry_generic_downloads_and_installs_database_driver() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tarball = database_driver_tarball_bytes();
    let sha256 = sha256_hex(&tarball);
    let client = Arc::new(FakeHttpClient::new(vec![binary_response(200, tarball)]));
    let entry = marketplace_database_driver_entry(Some(sha256));
    let mut registry = ExtensionRegistry::new(tmp.path().join("extensions"));
    registry.register_provider(Arc::new(DatabaseDriverExtensionProvider));

    let summary = smol::block_on(install_marketplace_entry_generic(
        client.clone(),
        &entry,
        &registry,
    ))
    .unwrap();

    assert_eq!(
        ExtensionSummary::new(
            ExtensionKind::DatabaseDriver,
            "fake_pg",
            "1.2.3",
            tmp.path().join("extensions/database_drivers/fake_pg")
        )
        .with_description("Test database driver")
        .with_driver_id("fake_pg"),
        summary
    );
    assert!(summary.path.join("driver.json").exists());
    assert_eq!(
        "https://example.test/fake_pg.tar.gz",
        client.take_requests()[0].uri
    );
}

fn marketplace_database_driver_entry(sha256: Option<String>) -> MarketplaceEntry {
    MarketplaceEntry {
        id: "fake_pg".to_string(),
        kind: ExtensionKind::DatabaseDriver,
        name: "Fake PostgreSQL".to_string(),
        version: "1.2.3".to_string(),
        description: String::new(),
        file_extensions: Vec::new(),
        asset_url: "https://example.test/fake_pg.tar.gz".to_string(),
        sha256,
    }
}

fn database_driver_tarball_bytes() -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    append_bytes(&mut archive, "driver-bin", b"driver");
    append_bytes(
        &mut archive,
        "driver.json",
        br#"{
            "id": "fake_pg",
            "name": "Fake PostgreSQL",
            "description": "Test database driver",
            "version": "1.2.3",
            "entry": { "command": "./driver-bin" },
            "transport": { "name": "fake_pg.sock" }
        }"#,
    );
    let encoder = archive.into_inner().unwrap();
    encoder.finish().unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn binary_response(status: u16, body: Vec<u8>) -> anyhow::Result<http::Response<AsyncBody>> {
    http::Response::builder()
        .status(status)
        .body(AsyncBody::from(body))
        .map_err(|error| anyhow::anyhow!("构建响应失败: {}", error))
}

fn append_bytes(
    archive: &mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>,
    name: &str,
    bytes: &[u8],
) {
    let mut header = tar::Header::new_gnu();
    header.set_path(name).unwrap();
    header.set_size(bytes.len() as u64);
    header.set_cksum();
    archive.append(&header, bytes).unwrap();
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CapturedRequest {
    uri: String,
}

struct FakeHttpClient {
    responses: Mutex<VecDeque<anyhow::Result<http_client::Response<AsyncBody>>>>,
    requests: Mutex<Vec<CapturedRequest>>,
}

impl FakeHttpClient {
    fn new(responses: Vec<anyhow::Result<http_client::Response<AsyncBody>>>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn response(status: u16, body: &str) -> anyhow::Result<http_client::Response<AsyncBody>> {
        http::Response::builder()
            .status(status)
            .body(AsyncBody::from(body.as_bytes().to_vec()))
            .map_err(|error| anyhow::anyhow!("构建响应失败: {}", error))
    }

    fn take_requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().expect("requests 锁失败").clone()
    }
}

impl HttpClient for FakeHttpClient {
    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn user_agent(&self) -> Option<&http::HeaderValue> {
        None
    }

    fn send(
        &self,
        req: http::Request<AsyncBody>,
    ) -> futures::future::BoxFuture<'static, anyhow::Result<http_client::Response<AsyncBody>>> {
        self.requests
            .lock()
            .expect("requests 锁失败")
            .push(CapturedRequest {
                uri: req.uri().to_string(),
            });
        let result = self
            .responses
            .lock()
            .expect("responses 锁失败")
            .pop_front()
            .unwrap_or_else(|| Err(anyhow::anyhow!("缺少 fake response")));

        async move { result }.boxed()
    }
}
