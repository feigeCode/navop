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
    pub languages: Vec<LegacyLanguageEntry>,
    #[serde(default)]
    pub extensions: Vec<MarketplaceEntry>,
}

impl MarketplaceManifest {
    pub fn into_entries(self) -> Vec<MarketplaceEntry> {
        if !self.extensions.is_empty() {
            return self
                .extensions
                .into_iter()
                .map(MarketplaceEntry::normalized)
                .collect();
        }
        self.languages
            .into_iter()
            .map(MarketplaceEntry::from)
            .collect()
    }

    pub(crate) fn resolve_asset_urls(&mut self, manifest_url: &str) {
        for entry in &mut self.extensions {
            entry.resolve_asset_urls(manifest_url);
        }
        for entry in &mut self.languages {
            entry.resolve_asset_urls(manifest_url);
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
    pub description: String,
    #[serde(default)]
    pub file_extensions: Vec<String>,
    #[serde(default)]
    pub asset_url: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub asset_urls: HashMap<String, String>,
    #[serde(default)]
    pub sha256s: HashMap<String, String>,
    #[serde(default, alias = "github_asset_url")]
    pub fallback_asset_url: Option<String>,
    #[serde(default, alias = "github_asset_urls")]
    pub fallback_asset_urls: HashMap<String, String>,
}

impl MarketplaceEntry {
    fn normalized(mut self) -> Self {
        if self.id.is_empty() {
            self.id = self.name.clone();
        }
        self
    }

    pub(crate) fn asset_url(&self) -> Option<String> {
        self.asset_url_for_keys(marketplace_target_keys())
    }

    pub(crate) fn sha256(&self) -> Option<String> {
        self.sha256_for_keys(marketplace_target_keys())
    }

    pub(crate) fn download_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        push_unique(&mut urls, self.asset_url());
        push_unique(&mut urls, self.fallback_asset_url());
        urls
    }

    pub(crate) fn asset_url_for_keys(&self, keys: &[&str]) -> Option<String> {
        select_keyed_value(&self.asset_urls, keys).or_else(|| non_empty(self.asset_url.clone()))
    }

    pub(crate) fn sha256_for_keys(&self, keys: &[&str]) -> Option<String> {
        select_keyed_value(&self.sha256s, keys).or_else(|| self.sha256.clone())
    }

    pub(crate) fn fallback_asset_url_for_keys(&self, keys: &[&str]) -> Option<String> {
        select_keyed_value(&self.fallback_asset_urls, keys)
            .or_else(|| self.fallback_asset_url.clone().and_then(non_empty))
    }

    pub(crate) fn fallback_asset_url(&self) -> Option<String> {
        self.fallback_asset_url_for_keys(marketplace_target_keys())
    }

    fn resolve_asset_urls(&mut self, manifest_url: &str) {
        self.asset_url = resolve_asset_url(manifest_url, &self.asset_url);
        resolve_asset_url_map(manifest_url, &mut self.asset_urls);
        if let Some(asset_url) = &mut self.fallback_asset_url {
            *asset_url = resolve_asset_url(manifest_url, asset_url);
        }
        resolve_asset_url_map(manifest_url, &mut self.fallback_asset_urls);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyLanguageEntry {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub file_extensions: Vec<String>,
    pub asset_url: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

impl From<&LegacyLanguageEntry> for MarketplaceEntry {
    fn from(entry: &LegacyLanguageEntry) -> Self {
        Self {
            id: entry.name.clone(),
            kind: ExtensionKind::Language,
            name: entry.name.clone(),
            version: entry.version.clone(),
            description: entry.description.clone(),
            file_extensions: entry.file_extensions.clone(),
            asset_url: entry.asset_url.clone(),
            sha256: entry.sha256.clone(),
            asset_urls: HashMap::new(),
            sha256s: HashMap::new(),
            fallback_asset_url: None,
            fallback_asset_urls: HashMap::new(),
        }
    }
}

impl From<LegacyLanguageEntry> for MarketplaceEntry {
    fn from(entry: LegacyLanguageEntry) -> Self {
        Self::from(&entry)
    }
}

impl LegacyLanguageEntry {
    fn resolve_asset_urls(&mut self, manifest_url: &str) {
        self.asset_url = resolve_asset_url(manifest_url, &self.asset_url);
    }
}

fn default_marketplace_kind() -> ExtensionKind {
    ExtensionKind::Language
}

fn select_keyed_value(values: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| values.get(*key).cloned())
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

fn resolve_asset_url_map(manifest_url: &str, asset_urls: &mut HashMap<String, String>) {
    for asset_url in asset_urls.values_mut() {
        *asset_url = resolve_asset_url(manifest_url, asset_url);
    }
}

fn has_http_url_scheme(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
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
        return &["aarch64-apple-darwin", "macos"];
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return &["x86_64-apple-darwin", "macos"];
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return &["x86_64-unknown-linux-gnu", "linux"];
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return &["x86_64-pc-windows-msvc", "windows"];
    }
    #[allow(unreachable_code)]
    &[]
}
