use crate::NotesStorage;
use crate::storage_support::{read_optional_json, write_json_atomic};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const LOCATION_FILE: &str = "notes-location.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct NotesLocation {
    root: PathBuf,
}

impl NotesStorage {
    pub fn default_root() -> Result<PathBuf> {
        let base = dirs::data_local_dir().context("local data directory is unavailable")?;
        Ok(base.join("navop").join("notes"))
    }

    pub fn configured_root() -> Result<PathBuf> {
        let default = Self::default_root()?;
        Self::configured_root_from(&location_path(&default)?, &default)
    }

    pub fn has_configured_root() -> Result<bool> {
        let default = Self::default_root()?;
        Self::has_configured_root_at(&location_path(&default)?)
    }

    pub fn save_configured_root(root: &Path) -> Result<()> {
        let default = Self::default_root()?;
        Self::save_configured_root_to(&location_path(&default)?, root)
    }

    pub(crate) fn configured_root_from(config: &Path, default: &Path) -> Result<PathBuf> {
        Ok(read_optional_json::<NotesLocation>(config)?
            .map(|location| location.root)
            .unwrap_or_else(|| default.to_path_buf()))
    }

    pub(crate) fn has_configured_root_at(config: &Path) -> Result<bool> {
        Ok(read_optional_json::<NotesLocation>(config)?.is_some())
    }

    pub(crate) fn save_configured_root_to(config: &Path, root: &Path) -> Result<()> {
        let root = root.canonicalize()?;
        if let Some(parent) = config.parent() {
            fs::create_dir_all(parent)?;
        }
        write_json_atomic(config, &NotesLocation { root })
    }
}

fn location_path(default_root: &Path) -> Result<PathBuf> {
    Ok(default_root
        .parent()
        .context("default notes root has no parent")?
        .join(LOCATION_FILE))
}
