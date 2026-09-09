use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::extension::manifest::{ShellHostModule, ShellSurface};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredShellViewContribution {
    pub extension_id: String,
    pub extension_version: String,
    pub id: String,
    pub view_key: String,
    pub title: String,
    pub description: Option<String>,
    pub icon_path: Option<PathBuf>,
    pub extension_root: PathBuf,
    pub entry_path: PathBuf,
    pub surface: ShellSurface,
    pub singleton: bool,
    pub backends: BTreeMap<String, String>,
    pub modules: BTreeSet<ShellHostModule>,
    pub permissions: Vec<String>,
    pub shell_api_version: String,
    pub required_gpui_shell_version: String,
}
