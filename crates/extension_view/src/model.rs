use std::path::PathBuf;

use semver::Version;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionKind {
    Language,
    LanguageBundle,
    DatabaseDriver,
    RemoteDesktopProvider,
    McpHelper,
    AcpAgent,
    Composite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionSummary {
    pub kind: ExtensionKind,
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub description: String,
    pub file_extensions: Vec<String>,
    pub icon: Option<String>,
    pub driver_id: Option<String>,
    pub default_port: Option<u16>,
}

impl ExtensionSummary {
    pub fn new(
        kind: ExtensionKind,
        name: impl Into<String>,
        version: impl Into<String>,
        path: PathBuf,
    ) -> Self {
        Self {
            kind,
            name: name.into(),
            version: version.into(),
            path,
            description: String::new(),
            file_extensions: Vec::new(),
            icon: None,
            driver_id: None,
            default_port: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_file_extensions(mut self, extensions: Vec<String>) -> Self {
        self.file_extensions = extensions;
        self
    }

    pub fn with_icon(mut self, icon: Option<String>) -> Self {
        self.icon = icon;
        self
    }

    pub fn with_driver_id(mut self, driver_id: Option<String>) -> Self {
        self.driver_id = driver_id;
        self
    }

    pub fn with_default_port(mut self, default_port: Option<u16>) -> Self {
        self.default_port = default_port;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketplaceEntry {
    pub id: String,
    pub kind: ExtensionKind,
    pub name: String,
    pub version: String,
    pub description: String,
    pub file_extensions: Vec<String>,
    pub asset_url: String,
    pub sha256: Option<String>,
    pub fallback_asset_url: Option<String>,
    pub manifest_url: Option<String>,
    pub manifest_fallback_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionReviewModel {
    pub high_risk_count: usize,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedMarketplaceExtension {
    pub entry: MarketplaceEntry,
    pub staging: PathBuf,
    pub review: PermissionReviewModel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketplaceInstallOutcome {
    Installed(ExtensionSummary),
    NeedsPermission(DownloadedMarketplaceExtension),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketplaceInstallState {
    NotInstalled,
    Installed,
    UpdateAvailable,
}

pub fn marketplace_install_state(
    installed: &[ExtensionSummary],
    entry: &MarketplaceEntry,
) -> MarketplaceInstallState {
    let entry_id = marketplace_entry_install_id(entry);
    let Some(summary) = installed
        .iter()
        .find(|summary| summary.kind == entry.kind && summary.name == entry_id)
    else {
        return MarketplaceInstallState::NotInstalled;
    };
    if marketplace_entry_is_update(summary, entry) {
        MarketplaceInstallState::UpdateAvailable
    } else {
        MarketplaceInstallState::Installed
    }
}

pub fn filter_installed(
    installed: &[ExtensionSummary],
    query: &str,
    kind: Option<ExtensionKind>,
) -> Vec<ExtensionSummary> {
    let query = normalized_query(query);
    installed
        .iter()
        .filter(|summary| kind.is_none_or(|kind| summary.kind == kind))
        .filter(|summary| summary_matches_query(summary, &query))
        .cloned()
        .collect()
}

pub fn filter_marketplace(
    entries: &[MarketplaceEntry],
    query: &str,
    kind: Option<ExtensionKind>,
) -> Vec<MarketplaceEntry> {
    let query = normalized_query(query);
    entries
        .iter()
        .filter(|entry| kind.is_none_or(|kind| entry.kind == kind))
        .filter(|entry| marketplace_entry_matches_query(entry, &query))
        .cloned()
        .collect()
}

pub fn marketplace_entry_install_id(entry: &MarketplaceEntry) -> &str {
    if entry.id.trim().is_empty() {
        entry.name.as_str()
    } else {
        entry.id.as_str()
    }
}

fn marketplace_entry_is_update(summary: &ExtensionSummary, entry: &MarketplaceEntry) -> bool {
    if entry.version.trim().is_empty() || summary.version == entry.version {
        return false;
    }
    match (
        Version::parse(&summary.version),
        Version::parse(&entry.version),
    ) {
        (Ok(current), Ok(latest)) => latest > current,
        _ => true,
    }
}

fn summary_matches_query(summary: &ExtensionSummary, query: &str) -> bool {
    query.is_empty()
        || contains_query(&summary.name, query)
        || contains_query(&summary.description, query)
        || summary
            .file_extensions
            .iter()
            .any(|extension| contains_query(extension, query))
}

fn marketplace_entry_matches_query(entry: &MarketplaceEntry, query: &str) -> bool {
    query.is_empty()
        || contains_query(&entry.id, query)
        || contains_query(&entry.name, query)
        || contains_query(&entry.description, query)
        || entry
            .file_extensions
            .iter()
            .any(|extension| contains_query(extension, query))
}

fn normalized_query(query: &str) -> String {
    query.trim().to_lowercase()
}

fn contains_query(value: &str, query: &str) -> bool {
    value.to_lowercase().contains(query)
}
