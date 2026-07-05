use std::collections::BTreeMap;
use std::path::{Component, PathBuf};
use std::sync::{OnceLock, RwLock};

const EXTENSION_ASSET_SCHEME: &str = "onet-extension://";
static EXTENSION_ASSET_ROOTS: OnceLock<RwLock<BTreeMap<String, PathBuf>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlPreviewAssetResolver {
    extension_id: String,
    extension_root: PathBuf,
}

impl HtmlPreviewAssetResolver {
    pub fn new(extension_id: impl Into<String>, extension_root: impl Into<PathBuf>) -> Self {
        Self {
            extension_id: extension_id.into(),
            extension_root: extension_root.into(),
        }
    }

    pub fn resolve(&self, path: &str) -> Option<String> {
        let path = path.trim();
        if !is_safe_relative_asset_path(path) {
            return None;
        }
        Some(format!(
            "{EXTENSION_ASSET_SCHEME}{}/{}",
            self.extension_id, path
        ))
    }

    pub fn extension_root(&self) -> &PathBuf {
        &self.extension_root
    }
}

pub fn register_extension_asset_root(
    extension_id: impl Into<String>,
    assets_root: impl Into<PathBuf>,
) {
    let roots = EXTENSION_ASSET_ROOTS.get_or_init(|| RwLock::new(BTreeMap::new()));
    if let Ok(mut roots) = roots.write() {
        roots.insert(extension_id.into(), assets_root.into());
    }
}

pub fn resolve_extension_asset_url(url: &str) -> Option<PathBuf> {
    let (extension_id, path) = parse_extension_asset_url(url)?;
    if !is_safe_relative_asset_path(path) {
        return None;
    }
    let roots = EXTENSION_ASSET_ROOTS.get_or_init(|| RwLock::new(BTreeMap::new()));
    let roots = roots.read().ok()?;
    let root = roots.get(extension_id)?;
    Some(root.join(path).components().collect())
}

pub(crate) fn parse_extension_asset_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix(EXTENSION_ASSET_SCHEME)?;
    let (extension_id, path) = rest.split_once('/')?;
    (!extension_id.is_empty() && !path.is_empty()).then_some((extension_id, path))
}

fn is_safe_relative_asset_path(path: &str) -> bool {
    if path.is_empty() || path.contains("://") || path.starts_with('/') {
        return false;
    }
    PathBuf::from(path)
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
}
