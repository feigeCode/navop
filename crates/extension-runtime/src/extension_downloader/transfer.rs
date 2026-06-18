use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result, anyhow};
use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request, http::header};

use super::marketplace::{MarketplaceEntry, MarketplaceManifest};
use crate::extension::{ExtensionKind, ExtensionRegistry, ExtensionSummary};

pub const GITHUB_EXTENSION_MANIFEST_URL: &str = "https://github.com/feigeCode/onetcli-extensions/releases/latest/download/extension-manifest.json";
pub const DEFAULT_EXTENSION_MANIFEST_URL: &str = GITHUB_EXTENSION_MANIFEST_URL;
const EXTENSION_GITHUB_MANIFEST_URL_ENV: &str = "ONETCLI_EXTENSION_GITHUB_MANIFEST_URL";
const DOWNLOAD_BUFFER_SIZE: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadProgress {
    Started { url: String },
    Bytes { downloaded: u64, total: Option<u64> },
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
    manifest.resolve_asset_urls(url);
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
        configured_github_extension_manifest_url(),
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

pub fn github_extension_manifest_url_from_parts(
    runtime: Option<&str>,
    build_time: Option<&str>,
) -> String {
    runtime
        .and_then(non_empty_trimmed)
        .or_else(|| build_time.and_then(non_empty_trimmed))
        .unwrap_or_else(|| GITHUB_EXTENSION_MANIFEST_URL.to_string())
}

fn configured_github_extension_manifest_url() -> String {
    let runtime = std::env::var(EXTENSION_GITHUB_MANIFEST_URL_ENV).ok();
    github_extension_manifest_url_from_parts(
        runtime.as_deref(),
        option_env!("ONETCLI_EXTENSION_GITHUB_MANIFEST_URL"),
    )
}

fn configured_extension_manifest_url() -> Option<String> {
    #[cfg(feature = "github-marketplace")]
    {
        return None;
    }

    #[cfg(not(feature = "github-marketplace"))]
    {
        runtime_env("ONETCLI_EXTENSION_MANIFEST_URL")
            .or_else(|| {
                non_empty_trimmed(option_env!("ONETCLI_EXTENSION_MANIFEST_URL").unwrap_or_default())
            })
            .or_else(extension_manifest_url_from_public_base)
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

#[cfg(not(feature = "github-marketplace"))]
fn runtime_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .and_then(|value| non_empty_trimmed(&value))
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
    let asset_urls = entry.download_urls();
    if asset_urls.is_empty() {
        anyhow::bail!("marketplace entry {} 缺少 asset_url", entry.id);
    }
    let expected_sha256 = entry.sha256();
    if expected_sha256.is_none() && entry.kind != ExtensionKind::Language {
        anyhow::bail!("marketplace entry {} 缺少 sha256", entry.id);
    }

    let mut last_error = None;
    for asset_url in asset_urls {
        match download_asset_to_staging(
            &http_client,
            &asset_url,
            expected_sha256.as_deref(),
            Arc::clone(&on_progress),
        )
        .await
        {
            Ok(staging) => return Ok(staging),
            Err(err) => {
                tracing::warn!("扩展资产下载失败，尝试下一个源: {err:#}");
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("marketplace entry {} 没有可用下载源", entry.id)))
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
    let tarball = fetch_bytes_with_progress(http_client, asset_url, on_progress)
        .await
        .with_context(|| format!("download asset {asset_url}"))?;
    if let Some(expected) = expected_sha256 {
        gpui_component::highlighter::verify_sha256(&tarball, expected)
            .with_context(|| format!("verify sha256 for {asset_url}"))?;
    }
    let staging = super::make_staging_dir()?;
    let result = super::extract_tarball_to(&tarball, &staging).map(|_| staging.clone());
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
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
    loop {
        let read = body
            .read(&mut chunk)
            .await
            .with_context(|| format!("read body from {url}"))?;
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
        on_progress(DownloadProgress::Bytes {
            downloaded: buf.len() as u64,
            total,
        });
    }
    on_progress(DownloadProgress::Finished);
    Ok(buf)
}
