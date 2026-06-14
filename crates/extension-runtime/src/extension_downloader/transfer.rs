use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result, anyhow};
use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request};

use super::marketplace::{MarketplaceEntry, MarketplaceManifest};
use crate::extension::{ExtensionKind, ExtensionRegistry, ExtensionSummary};

pub const DEFAULT_EXTENSION_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/feigeCode/onetcli-language-extensions/main/manifest.json";

pub async fn fetch_manifest_url(
    http_client: Arc<dyn HttpClient>,
    url: &str,
) -> Result<MarketplaceManifest> {
    let bytes = fetch_bytes(&http_client, url)
        .await
        .with_context(|| format!("fetch release manifest from {url}"))?;
    serde_json::from_slice(&bytes).context("parse release manifest")
}

pub async fn download_marketplace_entry_to_staging(
    http_client: Arc<dyn HttpClient>,
    entry: &MarketplaceEntry,
) -> Result<PathBuf> {
    if entry.sha256.is_none() && entry.kind != ExtensionKind::Language {
        anyhow::bail!("marketplace entry {} 缺少 sha256", entry.id);
    }
    let tarball = fetch_bytes(&http_client, &entry.asset_url)
        .await
        .with_context(|| format!("download asset {}", entry.asset_url))?;
    if let Some(expected) = &entry.sha256 {
        gpui_component::highlighter::verify_sha256(&tarball, expected)
            .with_context(|| format!("verify sha256 for {}", entry.asset_url))?;
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
    let mut body = response.into_body();
    let mut buf = Vec::new();
    body.read_to_end(&mut buf)
        .await
        .with_context(|| format!("read body from {url}"))?;
    Ok(buf)
}
