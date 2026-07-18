use std::sync::Arc;

use crate::extension::{ExtensionKind, ExtensionRegistry, ExtensionSummary};
use crate::extension_downloader::{
    DownloadProgressCallback, MarketplaceEntry,
    download_marketplace_entry_to_staging_with_progress, fetch_default_manifest_url,
    fetch_manifest_url, install_from_staging_generic,
};

pub const NAVOP_MCP_HELPER_ID: &str = "navop-mcp";

pub fn find_mcp_helper_entry(entries: &[MarketplaceEntry]) -> Option<&MarketplaceEntry> {
    entries
        .iter()
        .find(|entry| entry.kind == ExtensionKind::McpHelper && entry.id == NAVOP_MCP_HELPER_ID)
}

pub async fn install_mcp_helper_from_marketplace_with_registry(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
    manifest_url: &str,
    registry: &ExtensionRegistry,
) -> anyhow::Result<ExtensionSummary> {
    install_mcp_helper_from_marketplace_with_registry_and_progress(
        http_client,
        manifest_url,
        registry,
        Arc::new(|_| {}),
    )
    .await
}

pub async fn install_mcp_helper_from_marketplace_with_registry_and_progress(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
    manifest_url: &str,
    registry: &ExtensionRegistry,
    on_progress: DownloadProgressCallback,
) -> anyhow::Result<ExtensionSummary> {
    let manifest = fetch_manifest_url(http_client.clone(), manifest_url).await?;
    let entries = manifest.into_entries();
    let entry = find_mcp_helper_entry(&entries)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("扩展市场未找到 MCP helper {NAVOP_MCP_HELPER_ID}"))?;
    install_mcp_helper_marketplace_entry(http_client, &entry, registry, on_progress).await
}

pub async fn install_mcp_helper_from_marketplace(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
) -> anyhow::Result<ExtensionSummary> {
    install_mcp_helper_from_marketplace_with_progress(http_client, Arc::new(|_| {})).await
}

pub async fn install_mcp_helper_from_marketplace_with_progress(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
    on_progress: DownloadProgressCallback,
) -> anyhow::Result<ExtensionSummary> {
    let manifest = fetch_default_manifest_url(http_client.clone()).await?;
    let entries = manifest.into_entries();
    let entry = find_mcp_helper_entry(&entries)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("扩展市场未找到 MCP helper {NAVOP_MCP_HELPER_ID}"))?;
    let registry =
        ExtensionRegistry::global().ok_or_else(|| anyhow::anyhow!("扩展系统未初始化"))?;
    let registry = registry
        .read()
        .map_err(|error| anyhow::anyhow!("registry lock poisoned: {error}"))?;
    install_mcp_helper_marketplace_entry(http_client, &entry, &registry, on_progress).await
}

async fn install_mcp_helper_marketplace_entry(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
    entry: &MarketplaceEntry,
    registry: &ExtensionRegistry,
    on_progress: DownloadProgressCallback,
) -> anyhow::Result<ExtensionSummary> {
    let staging =
        download_marketplace_entry_to_staging_with_progress(http_client, entry, on_progress)
            .await?;
    let result = install_staged_mcp_helper(&staging, registry);
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn install_staged_mcp_helper(
    staging: &std::path::Path,
    registry: &ExtensionRegistry,
) -> anyhow::Result<ExtensionSummary> {
    install_from_staging_generic(staging, registry, Some(ExtensionKind::McpHelper))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use futures::FutureExt;
    use gpui::http_client::{self, AsyncBody, HttpClient, Url, http};

    use crate::extension::{ExtensionKind, ExtensionRegistry, McpHelperExtensionProvider};
    use crate::extension_downloader::MarketplaceEntry;

    #[test]
    fn find_mcp_helper_entry_matches_kind_and_stable_id() {
        let entries = vec![
            entry("navop-mcp", ExtensionKind::DatabaseDriver),
            entry("other-helper", ExtensionKind::McpHelper),
            entry("navop-mcp", ExtensionKind::McpHelper),
        ];

        let found = super::find_mcp_helper_entry(&entries);

        assert_eq!(Some("navop-mcp"), found.map(|entry| entry.id.as_str()));
    }

    #[test]
    fn install_mcp_helper_from_marketplace_installs_matching_entry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tarball = mcp_helper_tarball_bytes();
        let sha256 = sha256_hex(&tarball);
        let manifest = format!(
            r#"{{
                "extensions": [{{
                    "id": "navop-mcp",
                    "kind": "mcp_helper",
                    "name": "Navop MCP Helper",
                    "version": "1.2.3",
                    "release_tag": "mcp-helper-v1.2.3",
                    "artifacts": {{
                        "universal": {{
                            "file": "navop-mcp-mcp-helper-universal.tar.gz",
                            "sha256": "{sha256}"
                        }}
                    }}
                }}]
            }}"#
        );
        let client = Arc::new(FakeHttpClient::new(vec![
            FakeHttpClient::response(200, &manifest),
            binary_response(200, tarball),
        ]));
        let mut registry = ExtensionRegistry::new(tmp.path().join("extensions"));
        registry.register_provider(Arc::new(McpHelperExtensionProvider));

        let summary = smol::block_on(super::install_mcp_helper_from_marketplace_with_registry(
            client,
            "https://example.test/manifest.json",
            &registry,
        ))
        .unwrap();

        assert_eq!(ExtensionKind::McpHelper, summary.kind);
        assert_eq!("navop-mcp", summary.name);
        assert!(summary.path.join("mcp_helper.json").exists());
        assert!(summary.path.join("navop-mcp-1.2.3.tgz").exists());
    }

    #[test]
    fn install_mcp_helper_from_marketplace_reports_download_progress() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tarball = mcp_helper_tarball_bytes();
        let sha256 = sha256_hex(&tarball);
        let manifest = format!(
            r#"{{
                "extensions": [{{
                    "id": "navop-mcp",
                    "kind": "mcp_helper",
                    "name": "Navop MCP Helper",
                    "version": "1.2.3",
                    "release_tag": "mcp-helper-v1.2.3",
                    "artifacts": {{
                        "universal": {{
                            "file": "navop-mcp-mcp-helper-universal.tar.gz",
                            "sha256": "{sha256}"
                        }}
                    }}
                }}]
            }}"#
        );
        let client = Arc::new(FakeHttpClient::new(vec![
            FakeHttpClient::response(200, &manifest),
            binary_response(200, tarball),
        ]));
        let mut registry = ExtensionRegistry::new(tmp.path().join("extensions"));
        registry.register_provider(Arc::new(McpHelperExtensionProvider));
        let events = Arc::new(Mutex::new(Vec::new()));
        let callback_events = Arc::clone(&events);

        let summary = smol::block_on(
            super::install_mcp_helper_from_marketplace_with_registry_and_progress(
                client,
                "https://example.test/manifest.json",
                &registry,
                Arc::new(move |progress| {
                    callback_events.lock().unwrap().push(progress);
                }),
            ),
        )
        .unwrap();

        assert_eq!("navop-mcp", summary.name);
        let events = events.lock().unwrap();
        assert!(events.iter().any(|event| {
            matches!(
                event,
                crate::extension_downloader::DownloadProgress::Started { .. }
            )
        }));
        assert!(events.iter().any(|event| matches!(
            event,
            crate::extension_downloader::DownloadProgress::Finished
        )));
    }

    fn entry(id: &str, kind: ExtensionKind) -> MarketplaceEntry {
        MarketplaceEntry::from_resolved_urls(
            id,
            kind,
            id,
            "1.0.0",
            "",
            Vec::new(),
            vec![format!("https://example.test/{id}.tar.gz")],
            Some("hash".to_string()),
        )
    }

    fn mcp_helper_tarball_bytes() -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        append_bytes(&mut archive, "navop-mcp-1.2.3.tgz", b"npm-package");
        append_bytes(
            &mut archive,
            "mcp_helper.json",
            br#"{
                "id": "navop-mcp",
                "name": "Navop MCP",
                "description": "TypeScript MCP bridge and domain CLI",
                "version": "1.2.3",
                "entry": { "command": "npx", "args": ["-y", "@navop/mcp@1.2.3", "mcp"] },
                "distribution": { "type": "npm", "package": "@navop/mcp", "version": "1.2.3" }
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
        append_bytes_with_mode(archive, name, bytes, 0o644);
    }

    fn append_bytes_with_mode(
        archive: &mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>,
        name: &str,
        bytes: &[u8],
        mode: u32,
    ) {
        let mut header = tar::Header::new_gnu();
        header.set_path(name).unwrap();
        header.set_mode(mode);
        header.set_size(bytes.len() as u64);
        header.set_cksum();
        archive.append(&header, bytes).unwrap();
    }

    struct FakeHttpClient {
        responses: Mutex<VecDeque<anyhow::Result<http_client::Response<AsyncBody>>>>,
    }

    impl FakeHttpClient {
        fn new(responses: Vec<anyhow::Result<http_client::Response<AsyncBody>>>) -> Self {
            Self {
                responses: Mutex::new(VecDeque::from(responses)),
            }
        }

        fn response(status: u16, body: &str) -> anyhow::Result<http_client::Response<AsyncBody>> {
            http::Response::builder()
                .status(status)
                .body(AsyncBody::from(body.as_bytes().to_vec()))
                .map_err(|error| anyhow::anyhow!("构建响应失败: {}", error))
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
            _req: http::Request<AsyncBody>,
        ) -> futures::future::BoxFuture<'static, anyhow::Result<http_client::Response<AsyncBody>>>
        {
            let result = self
                .responses
                .lock()
                .expect("responses 锁失败")
                .pop_front()
                .unwrap_or_else(|| Err(anyhow::anyhow!("缺少 fake response")));

            async move { result }.boxed()
        }
    }
}
