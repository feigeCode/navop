use std::path::PathBuf;

use serde_json::Value;

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
    pub driver_api: Option<String>,
    pub driver_compatibility: Option<Value>,
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
            driver_api: None,
            driver_compatibility: None,
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

    pub fn with_driver_api(mut self, driver_api: impl Into<String>) -> Self {
        self.driver_api = Some(driver_api.into());
        self
    }

    pub fn with_driver_compatibility(mut self, compatibility: Value) -> Self {
        self.driver_compatibility = Some(compatibility);
        self
    }

    pub fn with_default_port(mut self, port: u16) -> Self {
        self.default_port = Some(port);
        self
    }
}
