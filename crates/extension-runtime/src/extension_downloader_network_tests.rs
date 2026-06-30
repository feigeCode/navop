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
#[cfg(feature = "github-marketplace")]
use crate::extension_downloader::DEFAULT_EXTENSION_MANIFEST_URL;
use crate::extension_downloader::{
    DownloadProgress, MarketplaceEntry, download_marketplace_entry_to_staging,
    download_marketplace_entry_to_staging_with_progress, fetch_manifest_url,
    install_marketplace_entry_generic,
};
#[cfg(not(feature = "github-marketplace"))]
use crate::extension_downloader::{
    GITHUB_EXTENSION_MANIFEST_URL, fetch_manifest_url_with_fallback,
    manifest_urls_for_configured_url,
    manifest_urls_for_configured_url_with_github_fallback,
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
                "release_tag": "fake_pg-v1.2.3",
                "artifacts": {
                    "universal": {
                        "file": "fake_pg-driver-universal.tar.gz",
                        "sha256": "abc"
                    }
                }
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
        Some("https://example.test/fake_pg/1.2.3/fake_pg-driver-universal.tar.gz".to_string()),
        entries[0].asset_url()
    );
    assert_eq!(
        "https://example.test/manifest.json",
        client.take_requests()[0].uri
    );
}

#[cfg(not(feature = "github-marketplace"))]
#[test]
fn missing_extension_manifest_env_uses_github_only() {
    assert_eq!(
        vec![GITHUB_EXTENSION_MANIFEST_URL.to_string()],
        manifest_urls_for_configured_url(None)
    );
}


#[cfg(not(feature = "github-marketplace"))]
#[test]
fn configured_manifest_urls_use_injected_github_fallback() {
    assert_eq!(
        vec![
            "https://onetcli.test.cn/extensions/manifest.json".to_string(),
            "https://raw.githubusercontent.example.test/feigeCode/onetcli-extensions/main/manifest.json".to_string(),
        ],
        manifest_urls_for_configured_url_with_github_fallback(
            Some("https://onetcli.test.cn/extensions/manifest.json".to_string()),
            "https://raw.githubusercontent.example.test/feigeCode/onetcli-extensions/main/manifest.json".to_string(),
        )
    );
}

#[cfg(feature = "github-marketplace")]
#[test]
fn github_marketplace_feature_points_to_github_manifest() {
    assert_eq!(
        "https://raw.githubusercontent.com/feigeCode/onetcli-extensions/main/manifest.json",
        DEFAULT_EXTENSION_MANIFEST_URL
    );
}

#[cfg(not(feature = "github-marketplace"))]
#[test]
fn fetch_default_manifest_url_falls_back_to_github_when_r2_fails() {
    let r2_manifest_url = "https://onetcli.test.cn/extensions/manifest.json".to_string();
    let client = Arc::new(FakeHttpClient::new(vec![
        Err(anyhow::anyhow!("r2 unavailable")),
        FakeHttpClient::response(
            200,
            r#"{
                "release_version": "github-fallback",
                "extensions": [{
                    "id": "rust",
                    "kind": "language",
                    "name": "Rust",
                    "version": "1.0.0",
                    "release_tag": "rust-v1.0.0",
                    "artifacts": {
                        "universal": {
                            "file": "rust-universal.tar.gz"
                        }
                    }
                }]
            }"#,
        ),
    ]));

    let manifest = smol::block_on(fetch_manifest_url_with_fallback(
        client.clone(),
        Some(r2_manifest_url),
    ))
    .unwrap();

    assert_eq!("github-fallback", manifest.release_version);
    let requests = client.take_requests();
    assert_eq!(
        "https://onetcli.test.cn/extensions/manifest.json",
        requests[0].uri
    );
    assert_eq!(
        "https://raw.githubusercontent.com/feigeCode/onetcli-extensions/main/manifest.json",
        requests[1].uri
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
fn download_marketplace_entry_to_staging_reports_progress() {
    let tarball = database_driver_tarball_bytes();
    let sha256 = sha256_hex(&tarball);
    let client = Arc::new(FakeHttpClient::new(vec![binary_response(
        200,
        tarball.clone(),
    )]));
    let entry = marketplace_database_driver_entry(Some(sha256));
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured_events = Arc::clone(&events);

    let staging = smol::block_on(download_marketplace_entry_to_staging_with_progress(
        client,
        &entry,
        Arc::new(move |progress| {
            captured_events
                .lock()
                .expect("events 锁失败")
                .push(progress);
        }),
    ))
    .unwrap();

    let events = events.lock().expect("events 锁失败");
    assert!(matches!(
        events.first(),
        Some(DownloadProgress::Started { url }) if url == "https://example.test/fake_pg.tar.gz"
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        DownloadProgress::Bytes {
            downloaded,
            total: Some(total),
        } if *downloaded > 0 && *total == tarball.len() as u64
    )));
    assert_eq!(Some(&DownloadProgress::Finished), events.last());
    fs::remove_dir_all(staging).unwrap();
}

#[test]
fn download_marketplace_entry_to_staging_reports_failed_source_before_fallback() {
    let tarball = database_driver_tarball_bytes();
    let sha256 = sha256_hex(&tarball);
    let client = Arc::new(FakeHttpClient::new(vec![
        binary_response(200, b"not the expected tarball".to_vec()),
        binary_response(200, tarball),
    ]));
    let entry = MarketplaceEntry::from_resolved_urls(
        "fake_pg",
        ExtensionKind::DatabaseDriver,
        "Fake PostgreSQL",
        "1.2.3",
        "",
        Vec::new(),
        vec![
            "https://onetcli.test.cn/extensions/fake_pg.tar.gz".to_string(),
            "https://github.example.test/fake_pg.tar.gz".to_string(),
        ],
        Some(sha256),
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured_events = Arc::clone(&events);

    let staging = smol::block_on(download_marketplace_entry_to_staging_with_progress(
        client,
        &entry,
        Arc::new(move |progress| {
            captured_events
                .lock()
                .expect("events 锁失败")
                .push(progress);
        }),
    ))
    .unwrap();

    let events = events.lock().expect("events 锁失败");
    assert!(events.iter().any(|event| matches!(
        event,
        DownloadProgress::Failed {
            url,
            retrying: true,
            ..
        } if url == "https://onetcli.test.cn/extensions/fake_pg.tar.gz"
    )));
    let failed_index = events
        .iter()
        .position(|event| matches!(event, DownloadProgress::Failed { .. }))
        .expect("missing failed progress event");
    assert!(
        !events[..failed_index]
            .iter()
            .any(|event| matches!(event, DownloadProgress::Finished))
    );
    let github_started_index = events
        .iter()
        .position(|event| {
            matches!(
                event,
                DownloadProgress::Started { url }
                    if url == "https://github.example.test/fake_pg.tar.gz"
            )
        })
        .expect("missing github started progress event");
    assert!(failed_index < github_started_index);
    fs::remove_dir_all(staging).unwrap();
}

#[test]
fn download_marketplace_entry_to_staging_falls_back_to_github_asset() {
    let tarball = database_driver_tarball_bytes();
    let sha256 = sha256_hex(&tarball);
    let client = Arc::new(FakeHttpClient::new(vec![
        Err(anyhow::anyhow!("r2 unavailable")),
        binary_response(200, tarball),
    ]));
    let entry = MarketplaceEntry::from_resolved_urls(
        "fake_pg",
        ExtensionKind::DatabaseDriver,
        "Fake PostgreSQL",
        "1.2.3",
        "",
        Vec::new(),
        vec![
            "https://onetcli.test.cn/extensions/fake_pg.tar.gz".to_string(),
            "https://github.example.test/fake_pg.tar.gz".to_string(),
        ],
        Some(sha256),
    );

    let staging = smol::block_on(download_marketplace_entry_to_staging(
        client.clone(),
        &entry,
    ))
    .unwrap();

    assert!(staging.join("driver.json").exists());
    let requests = client.take_requests();
    assert_eq!(
        "https://onetcli.test.cn/extensions/fake_pg.tar.gz",
        requests[0].uri
    );
    assert_eq!(
        "https://github.example.test/fake_pg.tar.gz",
        requests[1].uri
    );
    fs::remove_dir_all(staging).unwrap();
}

#[test]
fn download_marketplace_entry_to_staging_resolves_v2_primary_url_from_manifest_prefix() {
    let tarball = database_driver_tarball_bytes();
    let sha256 = sha256_hex(&tarball);
    let manifest = format!(
        r#"{{
            "release_version": "relative-assets",
            "extensions": [{{
                "id": "fake_pg",
                "kind": "database_driver",
                "name": "Fake PostgreSQL",
                "version": "1.2.3",
                "release_tag": "fake_pg-v1.2.3",
                "artifacts": {{
                    "universal": {{
                        "file": "fake_pg-driver-universal.tar.gz",
                        "sha256": "{sha256}"
                    }}
                }}
            }}]
        }}"#,
    );
    let client = Arc::new(FakeHttpClient::new(vec![
        FakeHttpClient::response(200, &manifest),
        Err(anyhow::anyhow!("primary unavailable")),
        binary_response(200, tarball),
    ]));

    let manifest = smol::block_on(fetch_manifest_url(
        client.clone(),
        "https://onetcli.test.cn/extensions/manifest.json",
    ))
    .unwrap();
    let entries = manifest.into_entries();

    let staging = smol::block_on(download_marketplace_entry_to_staging(
        client.clone(),
        &entries[0],
    ))
    .unwrap();

    assert!(staging.join("driver.json").exists());
    let requests = client.take_requests();
    assert_eq!(
        "https://onetcli.test.cn/extensions/manifest.json",
        requests[0].uri
    );
    assert_eq!(
        "https://onetcli.test.cn/extensions/fake_pg/1.2.3/fake_pg-driver-universal.tar.gz",
        requests[1].uri
    );
    assert_eq!(
        "https://github.com/feigeCode/onetcli-extensions/releases/download/fake_pg-v1.2.3/fake_pg-driver-universal.tar.gz",
        requests[2].uri
    );
    fs::remove_dir_all(staging).unwrap();
}

#[test]
fn download_marketplace_entry_to_staging_resolves_v2_artifact_and_host_github_fallback() {
    let tarball = database_driver_tarball_bytes();
    let sha256 = sha256_hex(&tarball);
    let marketplace_manifest = r#"{
        "schema_version": 2,
        "extensions": [{
            "id": "fake_pg",
            "kind": "database_driver",
            "name": "Fake PostgreSQL",
            "version": "1.2.3",
            "manifest": "fake_pg/manifest.json"
        }]
    }"#;
    let extension_manifest = format!(
        r#"{{
            "schema_version": 2,
            "extensions": [{{
                "id": "fake_pg",
                "kind": "database_driver",
                "name": "Fake PostgreSQL",
                "version": "1.2.3",
                "release_tag": "fake_pg-v1.2.3",
                "artifacts": {{
                    "universal": {{
                        "file": "fake_pg-driver-universal.tar.gz",
                        "sha256": "{sha256}"
                    }}
                }}
            }}]
        }}"#,
    );
    let client = Arc::new(FakeHttpClient::new(vec![
        FakeHttpClient::response(200, marketplace_manifest),
        FakeHttpClient::response(200, &extension_manifest),
        Err(anyhow::anyhow!("primary unavailable")),
        binary_response(200, tarball),
    ]));

    let manifest = smol::block_on(fetch_manifest_url(
        client.clone(),
        "https://onetcli.test.cn/extensions/manifest.json",
    ))
    .unwrap();
    let entries = manifest.into_entries();

    let staging = smol::block_on(download_marketplace_entry_to_staging(
        client.clone(),
        &entries[0],
    ))
    .unwrap();

    assert!(staging.join("driver.json").exists());
    let requests = client.take_requests();
    assert_eq!(
        "https://onetcli.test.cn/extensions/manifest.json",
        requests[0].uri
    );
    assert_eq!(
        "https://onetcli.test.cn/extensions/fake_pg/manifest.json",
        requests[1].uri
    );
    assert_eq!(
        "https://onetcli.test.cn/extensions/fake_pg/1.2.3/fake_pg-driver-universal.tar.gz",
        requests[2].uri
    );
    assert_eq!(
        "https://github.com/feigeCode/onetcli-extensions/releases/download/fake_pg-v1.2.3/fake_pg-driver-universal.tar.gz",
        requests[3].uri
    );
    fs::remove_dir_all(staging).unwrap();
}

#[test]
fn download_marketplace_entry_to_staging_falls_back_to_github_extension_manifest() {
    let tarball = database_driver_tarball_bytes();
    let sha256 = sha256_hex(&tarball);
    let marketplace_manifest = r#"{
        "schema_version": 2,
        "extensions": [{
            "id": "fake_pg",
            "kind": "database_driver",
            "name": "Fake PostgreSQL",
            "version": "1.2.3",
            "release_tag": "fake_pg-v1.2.3",
            "manifest": "fake_pg/manifest.json"
        }]
    }"#;
    let extension_manifest = format!(
        r#"{{
            "schema_version": 2,
            "extensions": [{{
                "id": "fake_pg",
                "kind": "database_driver",
                "name": "Fake PostgreSQL",
                "version": "1.2.3",
                "release_tag": "fake_pg-v1.2.3",
                "artifacts": {{
                    "universal": {{
                        "file": "fake_pg-driver-universal.tar.gz",
                        "sha256": "{sha256}"
                    }}
                }}
            }}]
        }}"#,
    );
    let client = Arc::new(FakeHttpClient::new(vec![
        FakeHttpClient::response(200, marketplace_manifest),
        Err(anyhow::anyhow!("r2 extension manifest unavailable")),
        FakeHttpClient::response(200, &extension_manifest),
        binary_response(200, tarball),
    ]));

    let manifest = smol::block_on(fetch_manifest_url(
        client.clone(),
        "https://onetcli.test.cn/extensions/manifest.json",
    ))
    .unwrap();
    let entries = manifest.into_entries();

    let staging = smol::block_on(download_marketplace_entry_to_staging(
        client.clone(),
        &entries[0],
    ))
    .unwrap();

    assert!(staging.join("driver.json").exists());
    let requests = client.take_requests();
    assert_eq!(
        "https://onetcli.test.cn/extensions/fake_pg/manifest.json",
        requests[1].uri
    );
    assert_eq!(
        "https://github.com/feigeCode/onetcli-extensions/releases/download/fake_pg-v1.2.3/extension-manifest.json",
        requests[2].uri
    );
    assert_eq!(
        "https://github.com/feigeCode/onetcli-extensions/releases/download/fake_pg-v1.2.3/fake_pg-driver-universal.tar.gz",
        requests[3].uri
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
    MarketplaceEntry::from_resolved_urls(
        "fake_pg",
        ExtensionKind::DatabaseDriver,
        "Fake PostgreSQL",
        "1.2.3",
        "",
        Vec::new(),
        vec!["https://example.test/fake_pg.tar.gz".to_string()],
        sha256,
    )
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
        .header(http::header::CONTENT_LENGTH, body.len().to_string())
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
    fn user_agent(&self) -> Option<&http::HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&Url> {
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
