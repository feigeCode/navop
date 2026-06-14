use std::path::PathBuf;

use crate::extension::ExtensionKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionSummary {
    pub kind: ExtensionKind,
    pub name: String,
    pub version: String,
    pub description: String,
    pub file_extensions: Vec<String>,
    pub path: PathBuf,
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
            description: String::new(),
            file_extensions: Vec::new(),
            path,
            icon: None,
            driver_id: None,
            default_port: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_file_extensions(mut self, file_extensions: Vec<String>) -> Self {
        self.file_extensions = file_extensions;
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_driver_id(mut self, driver_id: impl Into<String>) -> Self {
        self.driver_id = Some(driver_id.into());
        self
    }

    pub fn with_default_port(mut self, port: u16) -> Self {
        self.default_port = Some(port);
        self
    }
}
