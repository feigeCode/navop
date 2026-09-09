use std::path::PathBuf;

use crate::extension::manifest::ResourceConnectionForm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredResourceConnectionContribution {
    pub extension_id: String,
    pub extension_root: PathBuf,
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub icon_path: Option<PathBuf>,
    pub runtime_id: String,
    pub resource_type: String,
    pub shell_view_id: Option<String>,
    pub form: ResourceConnectionForm,
}
