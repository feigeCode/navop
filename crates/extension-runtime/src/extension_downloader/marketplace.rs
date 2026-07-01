use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize, de::Error};
use serde_json::Value;

use crate::extension::ExtensionKind;

const EXTENSION_RELEASE_MANIFEST_FILE: &str = "extension-manifest.json";
const DEFAULT_GITHUB_RELEASE_DOWNLOAD_BASE: &str =
    "https://github.com/feigeCode/onetcli-extensions/releases/download/";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceManifest {
    #[serde(default, deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub release_version: String,
    #[serde(default, deserialize_with = "deserialize_marketplace_entries")]
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
    pub manifest: String,
    #[serde(default)]
    pub artifacts: HashMap<String, MarketplaceArtifact>,
    #[serde(skip)]
    resolved_download_urls: Vec<String>,
    #[serde(skip)]
    resolved_sha256: Option<String>,
    #[serde(skip)]
    source_manifest_url: Option<String>,
    #[serde(skip)]
    github_manifest_url: Option<String>,
    #[serde(skip)]
    resolved_manifest_urls: Vec<String>,
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

        self.source_manifest_url = Some(manifest_url.to_string());
        self.github_manifest_url = Some(github_manifest_url.to_string());

        let Some(artifact) = self.artifact_for_keys(marketplace_target_keys()) else {
            return;
        };

        let github_url =
            self.github_download_url(&artifact.file, manifest_url, github_manifest_url);
        let mut urls = Vec::new();
        if is_github_release_download_url(manifest_url) {
            push_unique(&mut urls, github_url);
        } else {
            let primary_path = if self.is_extension_manifest_url(manifest_url) {
                format!("{}/{}", self.version, artifact.file)
            } else {
                format!("{}/{}/{}", self.id, self.version, artifact.file)
            };
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
            manifest: String::new(),
            artifacts: HashMap::new(),
            resolved_download_urls: download_urls,
            resolved_sha256: sha256,
            source_manifest_url: None,
            github_manifest_url: None,
            resolved_manifest_urls: Vec::new(),
        }
    }

    pub(crate) fn with_manifest_urls(mut self, manifest_urls: Vec<String>) -> Self {
        self.resolved_manifest_urls =
            manifest_urls
                .into_iter()
                .filter_map(non_empty)
                .fold(Vec::new(), |mut urls, url| {
                    push_unique(&mut urls, Some(url));
                    urls
                });
        if let Some(github_manifest_url) = self
            .resolved_manifest_urls
            .iter()
            .find(|url| is_github_release_download_url(url))
            .cloned()
        {
            self.github_manifest_url = Some(github_manifest_url);
        }
        self
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

    pub(crate) fn needs_extension_manifest(&self) -> bool {
        (!self.manifest.trim().is_empty() || !self.resolved_manifest_urls.is_empty())
            && self.artifacts.is_empty()
    }

    pub(crate) fn extension_manifest_url(&self) -> Option<String> {
        if let Some(manifest_url) = self.resolved_manifest_urls.first() {
            return Some(manifest_url.clone());
        }
        let manifest = non_empty(self.manifest.clone())?;
        if has_http_url_scheme(&manifest) {
            return Some(manifest);
        }
        let source = self.source_manifest_url.as_deref()?;
        Some(resolve_asset_url(source, &manifest))
    }

    pub(crate) fn extension_manifest_urls(&self) -> Vec<String> {
        let mut urls = self.resolved_manifest_urls.clone();
        push_unique(&mut urls, self.extension_manifest_url());
        push_unique(&mut urls, self.github_extension_manifest_url());
        urls
    }

    pub(crate) fn resolved_from_extension_manifest(
        &self,
        mut manifest: MarketplaceManifest,
        manifest_url: &str,
    ) -> Option<Self> {
        manifest.resolve_downloads(
            manifest_url,
            self.github_manifest_url.as_deref().unwrap_or_default(),
        );
        manifest
            .extensions
            .into_iter()
            .find(|entry| entry.id == self.id || (!self.name.is_empty() && entry.name == self.name))
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
            .or_else(|| Some(DEFAULT_GITHUB_RELEASE_DOWNLOAD_BASE.to_string()))
            .map(|base| format!("{base}{release_tag}/{file}"))
    }

    fn github_extension_manifest_url(&self) -> Option<String> {
        let release_tag = non_empty(self.release_tag.clone())?;
        self.github_manifest_url
            .as_deref()
            .and_then(github_release_download_base)
            .or_else(|| {
                self.source_manifest_url
                    .as_deref()
                    .and_then(github_release_download_base)
            })
            .or_else(|| Some(DEFAULT_GITHUB_RELEASE_DOWNLOAD_BASE.to_string()))
            .map(|base| format!("{base}{release_tag}/{EXTENSION_RELEASE_MANIFEST_FILE}"))
    }

    fn is_extension_manifest_url(&self, manifest_url: &str) -> bool {
        let path = strip_url_query(manifest_url).trim_end_matches('/');
        path.ends_with(&format!("/{}/manifest.json", self.id))
    }
}

fn default_marketplace_kind() -> ExtensionKind {
    ExtensionKind::Language
}

fn deserialize_schema_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::Null => Ok(0),
        Value::Number(number) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| D::Error::custom("schema_version must be a u32")),
        Value::String(value) => value
            .parse::<u32>()
            .map_err(|_| D::Error::custom("schema_version string must be a u32")),
        _ => Err(D::Error::custom("schema_version must be a u32 or string")),
    }
}

fn deserialize_marketplace_entries<'de, D>(
    deserializer: D,
) -> Result<Vec<MarketplaceEntry>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Vec::<Value>::deserialize(deserializer)?
        .into_iter()
        .filter_map(|value| match serde_json::from_value(value) {
            Ok(entry) => Some(entry),
            Err(err) => {
                tracing::warn!("跳过不兼容的扩展市场条目: {err}");
                None
            }
        })
        .collect())
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

fn strip_url_query(url: &str) -> &str {
    let url = url.trim();
    let path_end = url.find(['?', '#']).unwrap_or(url.len());
    &url[..path_end]
}

fn marketplace_target_keys() -> &'static [&'static str] {
    marketplace_target_keys_for(std::env::consts::OS, std::env::consts::ARCH)
}

pub(crate) fn marketplace_target_keys_for(os: &str, arch: &str) -> &'static [&'static str] {
    match (os, arch) {
        ("macos", "aarch64") => &["aarch64-apple-darwin", "macos", "universal"],
        ("macos", "x86_64") => &["x86_64-apple-darwin", "macos", "universal"],
        ("linux", "x86_64") => &["x86_64-unknown-linux-gnu", "linux", "universal"],
        ("linux", "aarch64") => &["aarch64-unknown-linux-gnu", "linux", "universal"],
        ("windows", "x86_64") => &["x86_64-pc-windows-msvc", "windows", "universal"],
        _ => &["universal"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marketplace_target_keys_include_linux_arm64() {
        assert_eq!(
            &["aarch64-unknown-linux-gnu", "linux", "universal"],
            marketplace_target_keys_for("linux", "aarch64")
        );
    }

    #[test]
    fn artifact_selection_prefers_linux_arm64_before_linux_fallback() {
        let entry = MarketplaceEntry {
            id: "duckdb".to_string(),
            kind: ExtensionKind::DatabaseDriver,
            name: "DuckDB".to_string(),
            version: "1.0.0".to_string(),
            release_tag: "duckdb-v1.0.0".to_string(),
            description: String::new(),
            file_extensions: Vec::new(),
            manifest: String::new(),
            artifacts: HashMap::from([
                (
                    "linux".to_string(),
                    MarketplaceArtifact {
                        file: "duckdb-driver-linux.tar.gz".to_string(),
                        sha256: Some("linux-sha".to_string()),
                    },
                ),
                (
                    "aarch64-unknown-linux-gnu".to_string(),
                    MarketplaceArtifact {
                        file: "duckdb-driver-aarch64-unknown-linux-gnu.tar.gz".to_string(),
                        sha256: Some("linux-arm64-sha".to_string()),
                    },
                ),
            ]),
            resolved_download_urls: Vec::new(),
            resolved_sha256: None,
            source_manifest_url: None,
            github_manifest_url: None,
            resolved_manifest_urls: Vec::new(),
        };

        let artifact = entry
            .artifact_for_keys(marketplace_target_keys_for("linux", "aarch64"))
            .expect("linux arm64 应选择专属 artifact");

        assert_eq!(
            "duckdb-driver-aarch64-unknown-linux-gnu.tar.gz",
            artifact.file
        );
        assert_eq!(Some("linux-arm64-sha"), artifact.sha256.as_deref());
    }
}
