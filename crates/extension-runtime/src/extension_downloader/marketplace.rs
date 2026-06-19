use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::extension::ExtensionKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceManifest {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub release_version: String,
    #[serde(default)]
    pub extensions: Vec<MarketplaceEntry>,
}

impl MarketplaceManifest {
    pub fn into_entries(self) -> Vec<MarketplaceEntry> {
        self.extensions
    }

    pub(crate) fn resolve_downloads(&mut self, manifest_url: &str, github_manifest_url: &str) {
        for entry in &mut self.extensions {
            entry.resolve_downloads(manifest_url, github_manifest_url);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default = "default_marketplace_kind")]
    pub kind: ExtensionKind,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub release_tag: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub file_extensions: Vec<String>,
    #[serde(default)]
    pub artifacts: HashMap<String, MarketplaceArtifact>,
    #[serde(skip)]
    resolved_download_urls: Vec<String>,
    #[serde(skip)]
    resolved_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceArtifact {
    pub file: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

impl MarketplaceEntry {
    fn resolve_downloads(&mut self, manifest_url: &str, github_manifest_url: &str) {
        if self.id.is_empty() {
            self.id = self.name.clone();
        }

        let Some(artifact) = self.artifact_for_keys(marketplace_target_keys()) else {
            return;
        };

        let github_url =
            self.github_download_url(&artifact.file, manifest_url, github_manifest_url);
        let mut urls = Vec::new();
        if is_github_release_download_url(manifest_url) {
            push_unique(&mut urls, github_url);
        } else {
            let primary_path = format!("{}/{}/{}", self.id, self.version, artifact.file);
            push_unique(
                &mut urls,
                Some(resolve_asset_url(manifest_url, &primary_path)),
            );
            push_unique(&mut urls, github_url);
        }

        self.resolved_download_urls = urls;
        self.resolved_sha256 = artifact.sha256.clone();
    }

    pub(crate) fn from_resolved_urls(
        id: impl Into<String>,
        kind: ExtensionKind,
        name: impl Into<String>,
        version: impl Into<String>,
        description: impl Into<String>,
        file_extensions: Vec<String>,
        download_urls: Vec<String>,
        sha256: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            name: name.into(),
            version: version.into(),
            release_tag: String::new(),
            description: description.into(),
            file_extensions,
            artifacts: HashMap::new(),
            resolved_download_urls: download_urls,
            resolved_sha256: sha256,
        }
    }

    pub(crate) fn asset_url(&self) -> Option<String> {
        self.resolved_download_urls.first().cloned()
    }

    pub(crate) fn sha256(&self) -> Option<String> {
        self.resolved_sha256.clone()
    }

    pub(crate) fn download_urls(&self) -> Vec<String> {
        self.resolved_download_urls.clone()
    }

    pub(crate) fn fallback_asset_url(&self) -> Option<String> {
        self.resolved_download_urls.get(1).cloned()
    }

    pub(crate) fn artifact_for_keys(&self, keys: &[&str]) -> Option<MarketplaceArtifact> {
        keys.iter()
            .find_map(|key| self.artifacts.get(*key).cloned())
    }

    fn github_download_url(
        &self,
        file: &str,
        manifest_url: &str,
        github_manifest_url: &str,
    ) -> Option<String> {
        let release_tag = non_empty(self.release_tag.clone())?;
        github_release_download_base(github_manifest_url)
            .or_else(|| github_release_download_base(manifest_url))
            .map(|base| format!("{base}{release_tag}/{file}"))
    }
}

fn default_marketplace_kind() -> ExtensionKind {
    ExtensionKind::Language
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn push_unique(urls: &mut Vec<String>, url: Option<String>) {
    let Some(url) = url else {
        return;
    };
    if !urls.iter().any(|existing| existing == &url) {
        urls.push(url);
    }
}

fn resolve_asset_url(manifest_url: &str, asset_url: &str) -> String {
    let asset_url = asset_url.trim();
    if asset_url.is_empty() || has_http_url_scheme(asset_url) {
        return asset_url.to_string();
    }
    manifest_url_prefix(manifest_url)
        .map(|prefix| format!("{prefix}{}", asset_url.trim_start_matches('/')))
        .unwrap_or_else(|| asset_url.to_string())
}

fn has_http_url_scheme(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn is_github_release_download_url(url: &str) -> bool {
    github_release_download_base(url).is_some()
}

fn github_release_download_base(url: &str) -> Option<String> {
    let url = url.trim();
    if !has_http_url_scheme(url) {
        return None;
    }
    let releases_index = url.find("/releases/")?;
    Some(format!("{}/releases/download/", &url[..releases_index]))
}

fn manifest_url_prefix(manifest_url: &str) -> Option<&str> {
    let manifest_url = manifest_url.trim();
    let scheme_end = manifest_url.find("://").map(|index| index + 3)?;
    let path_end = manifest_url.find(['?', '#']).unwrap_or(manifest_url.len());
    let without_query = &manifest_url[..path_end];
    let path_slash = without_query[scheme_end..].rfind('/')? + scheme_end;
    Some(&without_query[..=path_slash])
}

fn marketplace_target_keys() -> &'static [&'static str] {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return &["aarch64-apple-darwin", "macos", "universal"];
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return &["x86_64-apple-darwin", "macos", "universal"];
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return &["x86_64-unknown-linux-gnu", "linux", "universal"];
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return &["aarch64-unknown-linux-gnu", "linux", "universal"];
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return &["x86_64-pc-windows-msvc", "windows", "universal"];
    }
    #[allow(unreachable_code)]
    &["universal"]
}
