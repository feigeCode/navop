use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request, http::header};

use super::marketplace::{MarketplaceEntry, MarketplaceManifest};
use crate::extension::{ExtensionKind, ExtensionRegistry, ExtensionSummary};

pub const GITHUB_EXTENSION_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/feigeCode/onetcli-extensions/main/manifest.json";
pub const DEFAULT_EXTENSION_MANIFEST_URL: &str = GITHUB_EXTENSION_MANIFEST_URL;
const DOWNLOAD_BUFFER_SIZE: usize = 16 * 1024;
const DOWNLOAD_PROGRESS_EVENT_INTERVAL: Duration = Duration::from_millis(120);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadProgress {
    Started {
        url: String,
    },
    Bytes {
        downloaded: u64,
        total: Option<u64>,
    },
    Failed {
        url: String,
        error: String,
        retrying: bool,
    },
    Finished,
}

pub type DownloadProgressCallback = Arc<dyn Fn(DownloadProgress) + Send + Sync>;

#[cfg(not(feature = "github-marketplace"))]
const EXTENSION_MANIFEST_PATH: &str = "extensions/manifest.json";

pub async fn fetch_manifest_url(
    http_client: Arc<dyn HttpClient>,
    url: &str,
) -> Result<MarketplaceManifest> {
    let bytes = fetch_bytes(&http_client, url)
        .await
        .with_context(|| format!("fetch release manifest from {url}"))?;
    let mut manifest: MarketplaceManifest =
        serde_json::from_slice(&bytes).context("parse release manifest")?;
    manifest.resolve_downloads(url, &GITHUB_EXTENSION_MANIFEST_URL.to_string());
    Ok(manifest)
}

pub async fn fetch_default_manifest_url(
    http_client: Arc<dyn HttpClient>,
) -> Result<MarketplaceManifest> {
    fetch_manifest_url_with_fallback(http_client, configured_extension_manifest_url()).await
}

pub async fn fetch_manifest_url_with_fallback(
    http_client: Arc<dyn HttpClient>,
    configured_url: Option<String>,
) -> Result<MarketplaceManifest> {
    let urls = manifest_urls_for_configured_url(configured_url);
    let mut last_error = None;

    for url in urls {
        match fetch_manifest_url(http_client.clone(), &url).await {
            Ok(manifest) => return Ok(manifest),
            Err(err) => {
                tracing::warn!("扩展市场 manifest 加载失败，尝试下一个源: {err:#}");
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("没有可用的扩展市场 manifest 源")))
}

pub fn manifest_urls_for_configured_url(configured_url: Option<String>) -> Vec<String> {
    manifest_urls_for_configured_url_with_github_fallback(
        configured_url,
        GITHUB_EXTENSION_MANIFEST_URL.to_string(),
    )
}

pub fn manifest_urls_for_configured_url_with_github_fallback(
    configured_url: Option<String>,
    github_fallback_url: String,
) -> Vec<String> {
    #[cfg(feature = "github-marketplace")]
    {
        let _ = configured_url;
        return vec![github_fallback_url];
    }

    #[cfg(not(feature = "github-marketplace"))]
    {
        let mut urls = Vec::new();
        if let Some(url) = configured_url.and_then(|url| non_empty_trimmed(&url)) {
            push_unique_url(&mut urls, url);
        }
        push_unique_url(&mut urls, github_fallback_url);
        urls
    }
}

fn configured_extension_manifest_url() -> Option<String> {
    #[cfg(feature = "github-marketplace")]
    {
        return None;
    }

    #[cfg(not(feature = "github-marketplace"))]
    {
        extension_manifest_url_from_public_base()
    }
}

#[cfg(not(feature = "github-marketplace"))]
fn extension_manifest_url_from_public_base() -> Option<String> {
    one_core::config::public_base_url().map(|base_url| {
        format!(
            "{}/{}",
            base_url.trim_end_matches('/'),
            EXTENSION_MANIFEST_PATH
        )
    })
}

fn non_empty_trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(not(feature = "github-marketplace"))]
fn push_unique_url(urls: &mut Vec<String>, url: String) {
    if !urls.contains(&url) {
        urls.push(url);
    }
}

pub async fn download_marketplace_entry_to_staging(
    http_client: Arc<dyn HttpClient>,
    entry: &MarketplaceEntry,
) -> Result<PathBuf> {
    download_marketplace_entry_to_staging_with_progress(http_client, entry, Arc::new(|_| {})).await
}

pub async fn download_marketplace_entry_to_staging_with_progress(
    http_client: Arc<dyn HttpClient>,
    entry: &MarketplaceEntry,
    on_progress: DownloadProgressCallback,
) -> Result<PathBuf> {
    let entry = resolve_entry_downloads_if_needed(&http_client, entry).await?;
    let download_urls = entry.download_urls();
    if download_urls.is_empty() {
        anyhow::bail!("marketplace entry {} 缺少可下载 artifact", entry.id);
    }
    let expected_sha256 = entry.sha256();
    if expected_sha256.is_none() && entry.kind != ExtensionKind::Language {
        anyhow::bail!("marketplace entry {} 缺少 sha256", entry.id);
    }

    let download_url_count = download_urls.len();
    let mut last_error = None;
    for (index, download_url) in download_urls.into_iter().enumerate() {
        match download_asset_to_staging(
            &http_client,
            &download_url,
            expected_sha256.as_deref(),
            Arc::clone(&on_progress),
        )
        .await
        {
            Ok(staging) => return Ok(staging),
            Err(err) => {
                let error = format!("{err:#}");
                let retrying = index + 1 < download_url_count;
                on_progress(DownloadProgress::Failed {
                    url: download_url,
                    error: error.clone(),
                    retrying,
                });
                if retrying {
                    tracing::warn!("扩展资产下载失败，尝试下一个源: {error}");
                } else {
                    tracing::warn!("扩展资产下载失败，没有更多源: {error}");
                }
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("marketplace entry {} 没有可用下载源", entry.id)))
}

async fn resolve_entry_downloads_if_needed(
    http_client: &Arc<dyn HttpClient>,
    entry: &MarketplaceEntry,
) -> Result<MarketplaceEntry> {
    if !entry.needs_extension_manifest() {
        return Ok(entry.clone());
    }
    let manifest_urls = entry.extension_manifest_urls();
    if manifest_urls.is_empty() {
        anyhow::bail!("marketplace entry {} 缺少插件 manifest 地址", entry.id);
    }

    let mut last_error = None;
    for manifest_url in manifest_urls {
        match fetch_resolved_entry_manifest(http_client, entry, &manifest_url).await {
            Ok(entry) => return Ok(entry),
            Err(err) => {
                tracing::warn!("插件 manifest 加载失败，尝试下一个源: {err:#}");
                last_error = Some(err);
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| anyhow!("marketplace entry {} 没有可用插件 manifest", entry.id)))
}

async fn fetch_resolved_entry_manifest(
    http_client: &Arc<dyn HttpClient>,
    entry: &MarketplaceEntry,
    manifest_url: &str,
) -> Result<MarketplaceEntry> {
    let bytes = fetch_bytes(http_client, manifest_url)
        .await
        .with_context(|| format!("fetch extension manifest from {manifest_url}"))?;
    let manifest: MarketplaceManifest =
        serde_json::from_slice(&bytes).context("parse extension manifest")?;
    entry
        .resolved_from_extension_manifest(manifest, manifest_url)
        .ok_or_else(|| {
            anyhow!(
                "extension manifest {manifest_url} missing entry {}",
                entry.id
            )
        })
}

async fn download_asset_to_staging(
    http_client: &Arc<dyn HttpClient>,
    asset_url: &str,
    expected_sha256: Option<&str>,
    on_progress: DownloadProgressCallback,
) -> Result<PathBuf> {
    on_progress(DownloadProgress::Started {
        url: asset_url.to_string(),
    });
    let tarball = fetch_bytes_with_progress(http_client, asset_url, Arc::clone(&on_progress))
        .await
        .with_context(|| format!("download asset {asset_url}"))?;
    if let Some(expected) = expected_sha256 {
        gpui_component::highlighter::verify_sha256(&tarball, expected)
            .with_context(|| format!("verify sha256 for {asset_url}"))?;
    }
    let staging = super::make_staging_dir()?;
    if let Err(err) = super::extract_tarball_to(&tarball, &staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(err);
    }
    on_progress(DownloadProgress::Finished);
    Ok(staging)
}

pub async fn install_marketplace_entry_generic(
    http_client: Arc<dyn HttpClient>,
    entry: &MarketplaceEntry,
    registry: &ExtensionRegistry,
) -> Result<ExtensionSummary> {
    let staging = download_marketplace_entry_to_staging(http_client, entry).await?;
    let result = super::install_from_staging_generic(&staging, registry, Some(entry.kind));
    let _ = std::fs::remove_dir_all(&staging);
    result
}

async fn fetch_bytes(client: &Arc<dyn HttpClient>, url: &str) -> Result<Vec<u8>> {
    fetch_bytes_with_progress(client, url, Arc::new(|_| {})).await
}

async fn fetch_bytes_with_progress(
    client: &Arc<dyn HttpClient>,
    url: &str,
    on_progress: DownloadProgressCallback,
) -> Result<Vec<u8>> {
    let request = Request::builder()
        .method(Method::GET)
        .uri(url)
        .body(AsyncBody::empty())
        .context("build request")?;
    let response = client
        .send(request)
        .await
        .map_err(|error| anyhow!("send request to {url}: {error}"))?;
    if !response.status().is_success() {
        anyhow::bail!("HTTP {} for {url}", response.status());
    }
    let total = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let mut body = response.into_body();
    let mut buf = Vec::new();
    let mut chunk = vec![0; DOWNLOAD_BUFFER_SIZE];
    let mut last_progress_at = Instant::now();
    let mut last_reported_downloaded = 0;
    loop {
        let read = body
            .read(&mut chunk)
            .await
            .with_context(|| format!("read body from {url}"))?;
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
        let downloaded = buf.len() as u64;
        if should_emit_byte_progress(
            downloaded,
            total,
            last_reported_downloaded,
            last_progress_at.elapsed(),
        ) {
            on_progress(DownloadProgress::Bytes { downloaded, total });
            last_reported_downloaded = downloaded;
            last_progress_at = Instant::now();
        }
    }
    let downloaded = buf.len() as u64;
    if downloaded > 0 && downloaded != last_reported_downloaded {
        on_progress(DownloadProgress::Bytes { downloaded, total });
    }
    Ok(buf)
}

fn should_emit_byte_progress(
    downloaded: u64,
    total: Option<u64>,
    last_reported_downloaded: u64,
    elapsed: Duration,
) -> bool {
    if downloaded == last_reported_downloaded {
        return false;
    }
    if total.is_some_and(|total| total > 0 && downloaded >= total) {
        return true;
    }
    elapsed >= DOWNLOAD_PROGRESS_EVENT_INTERVAL
}
