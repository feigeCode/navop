use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, RwLock},
};

use anyhow::{Context, Result, anyhow};

use crate::extension::{ExtensionKind, ExtensionSummary};

pub trait ExtensionProvider: Send + Sync {
    fn kind(&self) -> ExtensionKind;
    fn list_installed(&self, root: &Path) -> Result<Vec<ExtensionSummary>>;
    fn install_from_dir(&self, dir: &Path) -> Result<ExtensionSummary>;
    fn uninstall(&self, dir: &Path) -> Result<String>;
}

pub struct ExtensionRegistry {
    providers: HashMap<ExtensionKind, Arc<dyn ExtensionProvider>>,
    extensions_root: PathBuf,
}

impl ExtensionRegistry {
    pub fn new(extensions_root: PathBuf) -> Self {
        Self {
            providers: HashMap::new(),
            extensions_root,
        }
    }

    pub fn register_provider(&mut self, provider: Arc<dyn ExtensionProvider>) {
        self.providers.insert(provider.kind(), provider);
    }

    pub fn provider(&self, kind: ExtensionKind) -> Option<Arc<dyn ExtensionProvider>> {
        self.providers.get(&kind).cloned()
    }

    pub fn root_for(&self, kind: ExtensionKind) -> PathBuf {
        self.extensions_root.join(kind.dir_name())
    }

    pub fn list_installed_of(&self, kind: ExtensionKind) -> Result<Vec<ExtensionSummary>> {
        let Some(provider) = self.provider(kind) else {
            return Ok(Vec::new());
        };
        let root = self.root_for(kind);
        if !root.exists() {
            return Ok(Vec::new());
        }
        provider.list_installed(&root)
    }

    pub fn list_all_installed(&self) -> Vec<ExtensionSummary> {
        let mut out = Vec::new();
        for kind in ExtensionKind::all() {
            match self.list_installed_of(*kind) {
                Ok(list) => out.extend(list),
                Err(e) => tracing::warn!("list installed for kind {:?} failed: {:?}", kind, e),
            }
        }
        out
    }

    pub fn uninstall(&self, kind: ExtensionKind, name: &str) -> Result<String> {
        let provider = self
            .provider(kind)
            .ok_or_else(|| anyhow!("no provider for {:?}", kind))?;
        let dir = self.root_for(kind).join(name);
        if !dir.exists() {
            anyhow::bail!("extension {} not found at {}", name, dir.display());
        }
        provider.uninstall(&dir).context("provider uninstall")
    }

    pub fn global() -> Option<&'static RwLock<ExtensionRegistry>> {
        GLOBAL.get()
    }
}

static GLOBAL: OnceLock<RwLock<ExtensionRegistry>> = OnceLock::new();

pub fn init_global(registry: ExtensionRegistry) {
    let _ = GLOBAL.set(RwLock::new(registry));
}
