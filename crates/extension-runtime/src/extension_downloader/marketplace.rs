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
    pub asset_url: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

impl MarketplaceEntry {
    fn normalized(mut self) -> Self {
        if self.id.is_empty() {
            self.id = self.name.clone();
        }
        self
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
        }
    }
}

impl From<LegacyLanguageEntry> for MarketplaceEntry {
    fn from(entry: LegacyLanguageEntry) -> Self {
        Self::from(&entry)
    }
}

fn default_marketplace_kind() -> ExtensionKind {
    ExtensionKind::Language
}
