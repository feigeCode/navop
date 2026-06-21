use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::RemoteDesktopProtocol;
use crate::provider::{PROVIDER_MANIFEST_FILE, RemoteDesktopProviderManifest};

const EXTENSIONS_DIR_NAME: &str = "extensions";
const PROVIDERS_DIR_NAME: &str = "remote_desktop_providers";
const PROVIDER_DIR_ENV: &str = "ONETCLI_REMOTE_DESKTOP_PROVIDER_DIR";

#[derive(Clone, Debug)]
pub struct RemoteDesktopProviderRegistry {
    providers: Vec<RemoteDesktopProviderManifest>,
}

#[derive(Clone, Debug)]
pub struct RemoteDesktopProviderRegistryLoadReport {
    pub registry: RemoteDesktopProviderRegistry,
    pub loaded: Vec<RemoteDesktopProviderLoadedEntry>,
    pub skipped: Vec<RemoteDesktopProviderSkippedEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDesktopProviderLoadedEntry {
    pub id: String,
    pub name: String,
    pub protocol: RemoteDesktopProtocol,
    pub version: String,
    pub dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDesktopProviderSkippedEntry {
    pub dir: PathBuf,
    pub error: String,
}

impl RemoteDesktopProviderRegistry {
    pub fn load_default() -> Self {
        Self::load_from_dirs(&default_provider_dirs()).unwrap_or_else(|_| Self::empty())
    }

    pub fn load_from_dirs(dirs: &[PathBuf]) -> anyhow::Result<Self> {
        let mut providers = Vec::new();
        let mut seen = HashSet::new();
        for dir in dirs {
            let report = Self::load_from_dir_with_report(dir)?;
            for provider in report.registry.providers {
                if seen.insert(provider.id.clone()) {
                    providers.push(provider);
                }
            }
        }
        sort_providers(&mut providers);
        Ok(Self { providers })
    }

    pub fn load_from_dir(dir: &Path) -> anyhow::Result<Self> {
        Ok(Self::load_from_dir_with_report(dir)?.registry)
    }

    pub fn load_from_dir_with_report(
        dir: &Path,
    ) -> anyhow::Result<RemoteDesktopProviderRegistryLoadReport> {
        if !dir.exists() {
            return Ok(RemoteDesktopProviderRegistryLoadReport {
                registry: Self::empty(),
                loaded: Vec::new(),
                skipped: Vec::new(),
            });
        }
        let mut providers = Vec::new();
        let mut loaded = Vec::new();
        let mut skipped = Vec::new();
        load_candidates(dir, &mut providers, &mut loaded, &mut skipped)?;
        dedupe_first(&mut providers, &mut loaded);
        Ok(RemoteDesktopProviderRegistryLoadReport {
            registry: Self { providers },
            loaded,
            skipped,
        })
    }

    pub fn load_provider_from_dir(
        dir: &Path,
    ) -> anyhow::Result<Option<RemoteDesktopProviderManifest>> {
        if !dir.exists() {
            return Ok(None);
        }
        let path = dir.join(PROVIDER_MANIFEST_FILE);
        if !path.exists() {
            return Ok(None);
        }
        load_manifest(dir).map(Some)
    }

    pub fn empty() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn providers(&self) -> &[RemoteDesktopProviderManifest] {
        &self.providers
    }

    pub fn find(&self, protocol: RemoteDesktopProtocol) -> Option<RemoteDesktopProviderManifest> {
        self.providers
            .iter()
            .find(|provider| provider.protocol == protocol)
            .cloned()
    }

    pub fn find_by_id(&self, id: &str) -> Option<RemoteDesktopProviderManifest> {
        self.providers
            .iter()
            .find(|provider| provider.id == id)
            .cloned()
    }
}

pub fn default_provider_dir() -> PathBuf {
    default_provider_dirs()
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from(EXTENSIONS_DIR_NAME).join(PROVIDERS_DIR_NAME))
}

pub fn default_provider_dirs() -> Vec<PathBuf> {
    if let Some(path) = std::env::var_os(PROVIDER_DIR_ENV).filter(|value| !value.is_empty()) {
        return vec![PathBuf::from(path)];
    }
    vec![default_user_provider_dir()]
}

fn default_user_provider_dir() -> PathBuf {
    one_core::storage::get_config_dir()
        .map(|dir| dir.join(EXTENSIONS_DIR_NAME).join(PROVIDERS_DIR_NAME))
        .unwrap_or_else(|_| PathBuf::from(EXTENSIONS_DIR_NAME).join(PROVIDERS_DIR_NAME))
}

fn load_candidates(
    dir: &Path,
    providers: &mut Vec<RemoteDesktopProviderManifest>,
    loaded: &mut Vec<RemoteDesktopProviderLoadedEntry>,
    skipped: &mut Vec<RemoteDesktopProviderSkippedEntry>,
) -> anyhow::Result<()> {
    if let Some(provider) = load_one_candidate(dir, skipped) {
        push_loaded(provider, providers, loaded);
        return Ok(());
    }
    for entry in sorted_dirs(dir)? {
        if let Some(provider) = load_one_candidate(&entry, skipped) {
            push_loaded(provider, providers, loaded);
        }
    }
    Ok(())
}

fn load_one_candidate(
    dir: &Path,
    skipped: &mut Vec<RemoteDesktopProviderSkippedEntry>,
) -> Option<RemoteDesktopProviderManifest> {
    match RemoteDesktopProviderRegistry::load_provider_from_dir(dir) {
        Ok(provider) => provider,
        Err(error) => {
            skipped.push(RemoteDesktopProviderSkippedEntry {
                dir: dir.to_path_buf(),
                error: error.to_string(),
            });
            None
        }
    }
}

fn push_loaded(
    provider: RemoteDesktopProviderManifest,
    providers: &mut Vec<RemoteDesktopProviderManifest>,
    loaded: &mut Vec<RemoteDesktopProviderLoadedEntry>,
) {
    loaded.push(RemoteDesktopProviderLoadedEntry {
        id: provider.id.clone(),
        name: provider.name.clone(),
        protocol: provider.protocol,
        version: provider.version.clone(),
        dir: provider.manifest_dir.clone(),
    });
    providers.push(provider);
}

fn sorted_dirs(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn dedupe_first(
    providers: &mut Vec<RemoteDesktopProviderManifest>,
    loaded: &mut Vec<RemoteDesktopProviderLoadedEntry>,
) {
    let mut seen = HashSet::new();
    providers.retain(|provider| seen.insert(provider.id.clone()));
    let mut seen = HashSet::new();
    loaded.retain(|entry| seen.insert(entry.id.clone()));
    sort_providers(providers);
    loaded.sort_by(|left, right| left.id.cmp(&right.id));
}

fn load_manifest(provider_dir: &Path) -> anyhow::Result<RemoteDesktopProviderManifest> {
    let path = provider_dir.join(PROVIDER_MANIFEST_FILE);
    let content = std::fs::read_to_string(&path)?;
    let mut manifest: RemoteDesktopProviderManifest = serde_json::from_str(&content)?;
    manifest.manifest_dir = provider_dir.to_path_buf();
    manifest.validate()?;
    Ok(manifest)
}

fn sort_providers(providers: &mut [RemoteDesktopProviderManifest]) {
    providers.sort_by(|left, right| left.id.cmp(&right.id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_user_provider_dir_matches_extension_runtime_root() {
        let expected = one_core::storage::get_config_dir()
            .unwrap()
            .join(EXTENSIONS_DIR_NAME)
            .join(PROVIDERS_DIR_NAME);

        assert_eq!(expected, default_user_provider_dir());
    }
}
